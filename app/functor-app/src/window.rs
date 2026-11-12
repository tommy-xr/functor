//! The window (macOS): a WKWebView (wry) on the served page, on tao's event
//! loop, which owns the main thread. The server and drivers run on a runtime
//! thread; closing the window or quitting stops them the way a signal does
//! in `--headless`.

use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use muda::{Menu, PredefinedMenuItem, Submenu};
use tao::dpi::{LogicalPosition, LogicalSize};
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::macos::EventLoopExtMacOS;
use tao::platform::run_return::EventLoopExtRunReturn;
use tao::window::{Window, WindowBuilder};
use wry::{NewWindowResponse, WebViewBuilder};

use crate::server::Token;
use crate::WindowLink;

/// The window's size on first launch, and the smallest it can be (points).
const DEFAULT: (f64, f64) = (1440.0, 900.0);
const MIN: (f64, f64) = (800.0, 560.0);

/// How long `--window-smoke` waits for the page to connect.
const SMOKE_TIMEOUT: Duration = Duration::from_secs(30);

/// How long quitting waits for the runtime thread to stop everything.
const STOP_TIMEOUT: Duration = Duration::from_secs(30);

enum UserEvent {
    /// The page's `document.title`.
    Title(String),
    /// The runtime thread has finished.
    Done,
}

/// Runs `app` on a new thread with the window's link to it, and the window
/// here until `app` has finished; `app`'s result.
pub fn run(
    home: Option<PathBuf>,
    smoke: bool,
    token: Token,
    app: impl FnOnce(WindowLink) -> io::Result<ExitCode> + Send + 'static,
) -> io::Result<ExitCode> {
    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    // A smoke run must not take focus from whoever is using this machine.
    event_loop.set_activate_ignoring_other_apps(!smoke);
    let proxy = event_loop.create_proxy();
    let (listening, origin) = mpsc::channel();
    let (close, closed) = tokio::sync::oneshot::channel();
    let (done, stopped) = mpsc::channel();
    let thread = std::thread::spawn(move || {
        let result = app((listening, closed));
        let _ = done.send(());
        let _ = proxy.send_event(UserEvent::Done);
        result
    });
    // No origin: the app stopped before serving (and said why).
    let Ok(origin) = origin.recv() else {
        return joined(thread);
    };

    let geometry = home.map(|home| home.join("window.json"));
    let screens: Vec<Rect> = event_loop
        .primary_monitor()
        .into_iter()
        .chain(event_loop.available_monitors())
        .map(|m| {
            let at = m.position().to_logical::<f64>(m.scale_factor());
            let size = m.size().to_logical::<f64>(m.scale_factor());
            Rect {
                x: at.x,
                y: at.y,
                w: size.width,
                h: size.height,
            }
        })
        .collect();
    let saved = geometry.as_deref().and_then(load);
    let at = place(saved, &screens);
    let window = WindowBuilder::new()
        .with_title("Functor")
        .with_inner_size(LogicalSize::new(at.w, at.h))
        .with_min_inner_size(LogicalSize::new(MIN.0, MIN.1))
        .with_position(LogicalPosition::new(at.x, at.y))
        .with_focused(!smoke)
        .build(&event_loop)
        .map_err(io::Error::other)?;

    let title = event_loop.create_proxy();
    let nav_origin = origin.clone();
    let webview = WebViewBuilder::new()
        .with_url(format!("{origin}/"))
        // Only on our own page; `server` describes the token's model.
        .with_initialization_script(format!(
            "if (location.origin === {}) window.functorToken = {};",
            serde_json::to_string(&origin).expect("a string"),
            serde_json::to_string(token.reveal()).expect("a string"),
        ))
        .with_navigation_handler(move |url| navigate(&url, &nav_origin))
        .with_new_window_req_handler(|url, _| {
            if outside(&url) {
                open(&url);
            }
            NewWindowResponse::Deny
        })
        .with_document_title_changed_handler(move |t| {
            let _ = title.send_event(UserEvent::Title(t));
        })
        .build(&window)
        .map_err(io::Error::other)?;

    let menu = menu().map_err(io::Error::other)?;
    let deadline = Instant::now() + SMOKE_TIMEOUT;
    let mut close = Some(close);
    let mut thread = Some(thread);
    let mut result = None;
    let mut smoke_failed = false;
    event_loop.run_return(|event, _, flow| {
        if *flow != ControlFlow::Exit {
            *flow = match smoke && close.is_some() {
                true => ControlFlow::WaitUntil(deadline),
                false => ControlFlow::Wait,
            };
        }
        match event {
            Event::NewEvents(StartCause::Init) => menu.init_for_nsapp(),
            Event::NewEvents(StartCause::ResumeTimeReached { .. }) if close.is_some() => {
                eprintln!(
                    "functor-app: window smoke: the page did not connect within {}s",
                    SMOKE_TIMEOUT.as_secs()
                );
                smoke_failed = true;
                stop(&mut close, geometry.as_deref(), &window);
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => stop(&mut close, geometry.as_deref(), &window),
            Event::UserEvent(UserEvent::Title(t)) => {
                // The page names the project once its socket has the state.
                if smoke && close.is_some() && t.starts_with("Functor — ") {
                    eprintln!("[functor-app] window smoke: the page connected ({t})");
                    stop(&mut close, geometry.as_deref(), &window);
                }
                window.set_title(&t);
            }
            Event::UserEvent(UserEvent::Done) => *flow = ControlFlow::Exit,
            // Also reached on Quit (the app menu's, the Dock's), which exits
            // the process right after: everything stops here, first.
            Event::LoopDestroyed => {
                stop(&mut close, geometry.as_deref(), &window);
                if stopped.recv_timeout(STOP_TIMEOUT).is_err() {
                    eprintln!(
                        "functor-app: still stopping after {}s; exiting",
                        STOP_TIMEOUT.as_secs()
                    );
                }
                if let Some(thread) = thread.take().filter(JoinHandle::is_finished) {
                    result = Some(joined(thread));
                }
            }
            _ => {}
        }
    });
    drop(webview);
    match (smoke_failed, result) {
        (false, Some(result)) => result,
        _ => Ok(ExitCode::FAILURE),
    }
}

/// Saves where `window` is, hides it and asks the runtime thread to stop
/// everything; once.
fn stop(
    close: &mut Option<tokio::sync::oneshot::Sender<()>>,
    geometry: Option<&Path>,
    window: &Window,
) {
    if let Some(close) = close.take() {
        if let Some(path) = geometry {
            save(window, path);
        }
        window.set_visible(false);
        let _ = close.send(());
    }
}

fn joined(thread: JoinHandle<io::Result<ExitCode>>) -> io::Result<ExitCode> {
    thread.join().unwrap_or(Ok(ExitCode::FAILURE))
}

/// The menu bar: Quit and Hide, the Edit shortcuts (WKWebView takes copy and
/// paste from the menu), and the standard Window items.
fn menu() -> muda::Result<Menu> {
    let sep = PredefinedMenuItem::separator;
    let app = Submenu::with_items(
        "Functor",
        true,
        &[
            &PredefinedMenuItem::hide(None),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &sep(),
            &PredefinedMenuItem::quit(None),
        ],
    )?;
    let edit = Submenu::with_items(
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(None),
            &PredefinedMenuItem::redo(None),
            &sep(),
            &PredefinedMenuItem::cut(None),
            &PredefinedMenuItem::copy(None),
            &PredefinedMenuItem::paste(None),
            &PredefinedMenuItem::select_all(None),
        ],
    )?;
    let window = Submenu::with_items(
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(None),
            &PredefinedMenuItem::zoom(None),
            &PredefinedMenuItem::fullscreen(None),
            &sep(),
            &PredefinedMenuItem::close_window(None),
        ],
    )?;
    window.set_as_windows_menu_for_nsapp();
    Menu::with_items(&[&app, &edit, &window])
}

/// Whether the webview may load `url`: only the app's own page. A web link
/// opens in the browser instead; anything else (files, `about:`, data) is
/// dropped.
fn navigate(url: &str, origin: &str) -> bool {
    if own(url, origin) {
        return true;
    }
    if outside(url) {
        open(url);
    }
    false
}

/// `url` is on `origin` (scheme, host and port all equal).
fn own(url: &str, origin: &str) -> bool {
    url.strip_prefix(origin)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(['/', '?', '#']))
}

/// `url` is one the system browser (or mail app) should open.
fn outside(url: &str) -> bool {
    let scheme = url.split_once(':').map(|(s, _)| s.to_ascii_lowercase());
    matches!(scheme.as_deref(), Some("http" | "https" | "mailto"))
}

fn open(url: &str) {
    eprintln!("[functor-app] opening in the browser: {url}");
    match std::process::Command::new("/usr/bin/open").arg(url).spawn() {
        // Reaped off the main thread.
        Ok(mut child) => drop(std::thread::spawn(move || child.wait())),
        Err(e) => eprintln!("functor-app: cannot open {url}: {e}"),
    }
}

/// A window or screen, in points, with y down from the top of the main screen.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

/// Where the window opens: where it was, if its title bar can still be
/// grabbed on one of `screens`; else that size (or the default), fitted to
/// the first screen and centred on it.
fn place(saved: Option<Rect>, screens: &[Rect]) -> Rect {
    let size = |r: Rect| Rect {
        w: r.w.max(MIN.0),
        h: r.h.max(MIN.1),
        ..r
    };
    if let Some(saved) = saved.map(size) {
        if screens.iter().any(|s| grabbable(saved, *s)) {
            return saved;
        }
    }
    let (w, h) = saved.map_or(DEFAULT, |s| (s.w, s.h));
    let Some(screen) = screens.first() else {
        return size(Rect { x: 0.0, y: 0.0, w, h });
    };
    let (w, h) = (w.min(screen.w).max(MIN.0), h.min(screen.h).max(MIN.1));
    Rect {
        x: screen.x + (screen.w - w).max(0.0) / 2.0,
        y: screen.y + (screen.h - h).max(0.0) / 2.0,
        w,
        h,
    }
}

/// At least 100 points of `window`'s top 30 are on `screen`.
fn grabbable(window: Rect, screen: Rect) -> bool {
    let across = (window.x + window.w).min(screen.x + screen.w) - window.x.max(screen.x);
    across >= 100.0 && window.y >= screen.y && window.y + 30.0 <= screen.y + screen.h
}

fn parse(text: &str) -> Option<Rect> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let n = |k: &str| v[k].as_f64().filter(|n| n.is_finite());
    Some(Rect {
        x: n("x")?,
        y: n("y")?,
        w: n("width")?,
        h: n("height")?,
    })
}

fn format(r: Rect) -> String {
    serde_json::json!({"x": r.x, "y": r.y, "width": r.w, "height": r.h}).to_string()
}

fn load(path: &Path) -> Option<Rect> {
    parse(&std::fs::read_to_string(path).ok()?)
}

/// Saves where `window` is, unless it is full screen or minimised (then the
/// last saved place stays).
fn save(window: &Window, path: &Path) {
    if window.fullscreen().is_some() || window.is_minimized() {
        return;
    }
    let scale = window.scale_factor();
    let Ok(at) = window.outer_position() else {
        return;
    };
    let at = at.to_logical::<f64>(scale);
    let size = window.inner_size().to_logical::<f64>(scale);
    let rect = Rect {
        x: at.x,
        y: at.y,
        w: size.width,
        h: size.height,
    };
    if let Err(e) = std::fs::write(path, format(rect)) {
        eprintln!("functor-app: cannot save {}: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGIN: &str = "http://127.0.0.1:4321";

    #[test]
    fn only_the_apps_own_origin_loads() {
        for url in [
            "http://127.0.0.1:4321",
            "http://127.0.0.1:4321/",
            "http://127.0.0.1:4321/#tasks",
            "http://127.0.0.1:4321/?x=1",
        ] {
            assert!(own(url, ORIGIN), "{url}");
        }
        for url in [
            "http://127.0.0.1:43210/",
            "http://127.0.0.1:4321@evil.example/",
            "http://127.0.0.1:4321.evil.example/",
            "https://127.0.0.1:4321/",
            "http://localhost:4321/",
            "about:blank",
            "file:///etc/passwd",
        ] {
            assert!(!own(url, ORIGIN), "{url}");
        }
    }

    #[test]
    fn web_and_mail_links_go_outside_and_nothing_else_does() {
        for url in ["https://example.com/a", "HTTP://example.com", "mailto:a@b.c"] {
            assert!(outside(url), "{url}");
        }
        for url in [
            "file:///Applications/Calculator.app",
            "about:blank",
            "data:text/html,hi",
            "javascript:alert(1)",
            "x-apple.systempreferences:",
            "ssh://host",
            "",
        ] {
            assert!(!outside(url), "{url}");
        }
    }

    const SCREEN: Rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 1728.0,
        h: 1117.0,
    };

    #[test]
    fn first_launch_centres_the_default_size() {
        let at = place(None, &[SCREEN]);
        assert_eq!((at.w, at.h), DEFAULT);
        assert_eq!((at.x, at.y), ((1728.0 - 1440.0) / 2.0, (1117.0 - 900.0) / 2.0));
        // A small screen: fitted, never below the minimum.
        let small = Rect {
            w: 1280.0,
            h: 800.0,
            ..SCREEN
        };
        let at = place(None, &[small]);
        assert_eq!((at.x, at.y, at.w, at.h), (0.0, 0.0, 1280.0, 800.0));
        let tiny = Rect {
            w: 640.0,
            h: 480.0,
            ..SCREEN
        };
        assert_eq!((place(None, &[tiny]).w, place(None, &[tiny]).h), MIN);
    }

    #[test]
    fn a_saved_place_on_any_screen_is_kept() {
        let left = Rect {
            x: -1920.0,
            y: -200.0,
            w: 1920.0,
            h: 1080.0,
        };
        let saved = Rect {
            x: -1500.0,
            y: 0.0,
            w: 1000.0,
            h: 700.0,
        };
        assert_eq!(place(Some(saved), &[SCREEN, left]), saved);
        // Mostly off the edge, with enough title bar left to grab.
        let edge = Rect { x: 1600.0, ..saved };
        assert_eq!(place(Some(edge), &[SCREEN]), edge);
        // Too small a saved size grows to the minimum.
        let small = Rect {
            w: 100.0,
            h: 100.0,
            ..edge
        };
        assert_eq!(place(Some(small), &[SCREEN]), Rect { w: MIN.0, h: MIN.1, ..edge });
    }

    #[test]
    fn a_saved_place_off_every_screen_is_recentred_keeping_its_size() {
        // The screen it was on is unplugged; or above the top; or below.
        for saved in [
            Rect {
                x: -1500.0,
                y: 0.0,
                w: 1000.0,
                h: 700.0,
            },
            Rect {
                x: 100.0,
                y: -50.0,
                w: 1000.0,
                h: 700.0,
            },
            Rect {
                x: 100.0,
                y: 1100.0,
                w: 1000.0,
                h: 700.0,
            },
            Rect {
                x: 1650.0,
                y: 0.0,
                w: 1000.0,
                h: 700.0,
            },
        ] {
            let at = place(Some(saved), &[SCREEN]);
            assert_eq!((at.w, at.h), (1000.0, 700.0), "{saved:?}");
            assert_eq!((at.x, at.y), (364.0, 208.5), "{saved:?}");
        }
    }

    #[test]
    fn geometry_round_trips_and_bad_files_are_ignored() {
        let r = Rect {
            x: -12.5,
            y: 40.0,
            w: 1200.0,
            h: 800.0,
        };
        assert_eq!(parse(&format(r)), Some(r));
        for text in ["", "{}", "[1,2,3,4]", r#"{"x":1,"y":2,"width":3}"#, "not json"] {
            assert_eq!(parse(text), None, "{text}");
        }
    }
}
