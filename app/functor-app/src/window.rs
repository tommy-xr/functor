//! The window (macOS): a WKWebView (wry) on the served page, on tao's event
//! loop, which owns the main thread. The server and drivers run on a runtime
//! thread; closing the window or quitting stops them the way a signal does
//! in `--headless`.

use std::io;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};
use tao::dpi::{LogicalPosition, LogicalSize};
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoop, EventLoopBuilder};
use tao::platform::macos::{EventLoopExtMacOS, WindowBuilderExtMacOS};
use tao::platform::run_return::EventLoopExtRunReturn;
use tao::window::{Theme, Window, WindowBuilder};
use wry::{NewWindowResponse, WebView, WebViewBuilder};

use functor_app_core::folders::MAX_RECENTS;
use functor_app_core::Recent;

use crate::popouts::{self, Page, PopOuts, Wanted};
use crate::server::Token;
use crate::WindowLink;

/// The window's size on first launch, and the smallest it can be (points).
const DEFAULT: (f64, f64) = (1440.0, 900.0);
const MIN: (f64, f64) = (800.0, 560.0);

/// How long `--window-smoke` waits for the page to connect.
const SMOKE_TIMEOUT: Duration = Duration::from_secs(30);

/// The page's base colour (`--bg`), behind it before it paints.
const BACKGROUND: (u8, u8, u8, u8) = (0x0e, 0x12, 0x17, 0xff);

/// Where the traffic lights sit in the page's title bar (points from the
/// window's top left), and the least left padding that clears them.
const LIGHTS_AT: (f64, f64) = (16.0, 20.0);
const TRAFFIC_LIGHTS: f64 = 76.0;

/// `--window-smoke`'s probe: the page reports its title bar's left padding,
/// then presses the bar's empty middle, as a person would to move the window.
const SMOKE_PROBE: &str = r#"(() => {
  const bar = document.querySelector(".titlebar");
  window.ipc.postMessage(JSON.stringify({ inset: parseFloat(getComputedStyle(bar).paddingLeft) }));
  bar.querySelector(".spacer").dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0, detail: 1 }));
})();"#;

/// `--window-smoke`'s second probe: the first pane's pop-out button, which
/// opens a window whose page then names its tab, then closes and docks.
const SMOKE_POP_OUT: &str =
    r#"document.querySelector('[aria-label="Pop out into its own window"]').click();"#;

/// How long quitting waits for the runtime thread to stop everything.
const STOP_TIMEOUT: Duration = Duration::from_secs(30);

enum UserEvent {
    /// The page's `document.title`.
    Title(String),
    /// A menu bar item was chosen.
    Menu(MenuId),
    /// The page's recent projects, for Open Recent.
    Recents(Vec<Recent>),
    /// A press on the page's title bar: move the window, or (a double
    /// press) zoom it.
    Drag,
    Zoom,
    /// The title bar's left padding, which `--window-smoke` asks for.
    Inset(f64),
    /// The windows the layout lists (`popouts.rs`).
    Windows(Vec<Wanted>),
    /// Bring pane N's window to the front.
    ShowWindow(u64),
    /// Pane N's window's page named its tab.
    WindowTitle(u64, String),
    /// The runtime thread has finished.
    Done,
}

/// The File menu's New Project and Open Folder items.
const NEW_PROJECT: &str = "new-project";
const OPEN_FOLDER: &str = "open-folder";
/// Open Recent's items: this, then the entry's index.
const RECENT: &str = "recent-";

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
    let (done, finished) = mpsc::channel();
    std::thread::spawn(move || {
        // A panic still ends the loop, as a failure.
        let result = std::panic::catch_unwind(AssertUnwindSafe(|| app((listening, closed))));
        let _ = done.send(result.unwrap_or(Ok(ExitCode::FAILURE)));
        let _ = proxy.send_event(UserEvent::Done);
    });
    let failed = || Ok(ExitCode::FAILURE);
    // No origin: the app stopped before serving (and said why).
    let Ok(origin) = origin.recv() else {
        return finished.recv().unwrap_or_else(|_| failed());
    };
    let geometry = home.map(|home| home.join("window.json"));
    let (window, webview, (menu, open_recent)) =
        match open(&event_loop, &origin, &token, geometry.as_deref(), smoke) {
            Ok(opened) => opened,
            Err(e) => {
                let _ = close.send(());
                let _ = finished.recv();
                return Err(e);
            }
        };

    let chosen = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        let _ = chosen.send_event(UserEvent::Menu(e.id));
    }));
    let mut recents = Vec::new();

    let mut smoke_by = Instant::now() + SMOKE_TIMEOUT;
    // Set once stopping starts; past it the window gives up waiting.
    let mut stop_by = None;
    let mut close = Some(close);
    let mut result = None;
    let mut smoke_failed = false;
    // `--window-smoke`'s checks of the title bar: its inset, and a press on
    // it reaching here.
    let (mut inset, mut dragged) = (None, false);
    let mut fullscreen = false;
    let mut probed = false;
    // `--window-smoke`'s pop-out: asked for, then the pane its window shows
    // once that page has named its tab, then docked.
    let (mut popping, mut popped, mut docked) = (false, None, false);
    let mut popouts = PopOuts::default();
    let page = Page {
        origin: origin.clone(),
        token: token.clone(),
    };
    let titled = event_loop.create_proxy();
    event_loop.run_return(|event, target, flow| {
        match event {
            Event::NewEvents(StartCause::Init) => menu.init_for_nsapp(),
            Event::NewEvents(StartCause::ResumeTimeReached { .. }) => {
                if close.is_some() {
                    let what = match (probed, popping) {
                        (_, true) => "a pane did not pop out and dock",
                        (true, _) => "the title bar did not answer its probe",
                        (false, _) => "the page did not connect",
                    };
                    eprintln!(
                        "functor-app: window smoke: {what} within {}s",
                        SMOKE_TIMEOUT.as_secs()
                    );
                    smoke_failed = true;
                    stop(&mut close, geometry.as_deref(), &window);
                } else if stop_by.is_some_and(|by| Instant::now() >= by) {
                    *flow = ControlFlow::Exit;
                }
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                window_id,
                ..
            } if window_id == window.id() => {
                // The panes' windows go with it (the layout keeps them).
                popouts = PopOuts::default();
                stop(&mut close, geometry.as_deref(), &window)
            }
            // A pop-out closed by the user (its button, ⌘W): its tabs dock.
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                window_id,
                ..
            } => {
                if let Some(pane) = popouts.close(window_id) {
                    let _ = webview.evaluate_script(&popouts::closed_script(pane));
                }
            }
            Event::WindowEvent {
                event: WindowEvent::Moved(_) | WindowEvent::Resized(_),
                window_id,
                ..
            } if window_id != window.id() => {
                if let Some((pane, at)) = popouts.frame(window_id) {
                    let _ = webview.evaluate_script(&popouts::frame_script(pane, at));
                }
            }
            // Native fullscreen hides the traffic lights, so the page
            // drops its inset for them.
            Event::WindowEvent {
                event: WindowEvent::Resized(_),
                ..
            } => {
                if fullscreen != window.fullscreen().is_some() {
                    fullscreen = !fullscreen;
                    let _ = webview.evaluate_script(&fullscreen_script(fullscreen));
                }
            }
            Event::UserEvent(UserEvent::Title(t)) => {
                // The page names the project once its socket has the state.
                if smoke && close.is_some() && t.starts_with("Functor — ") && inset.is_none() {
                    eprintln!("[functor-app] window smoke: the page connected ({t})");
                    probed = true;
                    if let Err(e) = webview.evaluate_script(SMOKE_PROBE) {
                        eprintln!("functor-app: window smoke: cannot probe the title bar: {e}");
                    }
                }
                // A reloaded page starts without the fullscreen class.
                if fullscreen {
                    let _ = webview.evaluate_script(&fullscreen_script(true));
                }
                // Hidden, but Mission Control and the Window menu show it.
                window.set_title(&t);
            }
            Event::UserEvent(UserEvent::Drag) if smoke => dragged = true,
            // Fullscreen neither moves nor zooms (tao would zoom on exit).
            Event::UserEvent(UserEvent::Drag) if !fullscreen => {
                let _ = window.drag_window();
            }
            Event::UserEvent(UserEvent::Zoom) if !fullscreen => {
                window.set_maximized(!window.is_maximized())
            }
            Event::UserEvent(UserEvent::Inset(px)) => inset = Some(px),
            Event::UserEvent(UserEvent::Windows(list)) => {
                let titled = titled.clone();
                popouts.reconcile(target, &list, &page, &screens(target), !smoke, move |pane, t| {
                    let _ = titled.send_event(UserEvent::WindowTitle(pane, t));
                });
                if smoke && popped.is_some_and(|p| !list.iter().any(|w| w.pane == p)) {
                    docked = true;
                }
            }
            Event::UserEvent(UserEvent::ShowWindow(pane)) => popouts.show(pane),
            Event::UserEvent(UserEvent::WindowTitle(pane, t)) => {
                popouts.set_title(pane, &t);
                // The smoke closes the window once its page is up, as a person would.
                if smoke && popping && popped.is_none() && t.ends_with(" — Functor") {
                    eprintln!("[functor-app] window smoke: pane {pane} opened in its own window ({t})");
                    popped = Some(pane);
                    if popouts.close_pane(pane) {
                        let _ = webview.evaluate_script(&popouts::closed_script(pane));
                    }
                }
            }
            // The page acts on its own menu commands, as on its buttons.
            Event::UserEvent(UserEvent::Menu(id)) => {
                if let Some(script) = menu_script(id.as_ref(), &recents) {
                    if let Err(e) = webview.evaluate_script(&script) {
                        eprintln!("functor-app: cannot run the menu command: {e}");
                    }
                }
            }
            Event::UserEvent(UserEvent::Recents(list)) => {
                recents = list;
                fill_recent(&open_recent, &recents);
            }
            Event::UserEvent(UserEvent::Done) => *flow = ControlFlow::Exit,
            // Also reached on Quit (the app menu's, the Dock's), which exits
            // the process right after: everything stops here, first.
            Event::LoopDestroyed => {
                stop(&mut close, geometry.as_deref(), &window);
                let by = *stop_by.get_or_insert_with(|| Instant::now() + STOP_TIMEOUT);
                match finished.recv_timeout(by.saturating_duration_since(Instant::now())) {
                    Ok(r) => result = Some(r),
                    Err(_) => eprintln!(
                        "functor-app: still stopping after {}s; exiting",
                        STOP_TIMEOUT.as_secs()
                    ),
                }
            }
            _ => {}
        }
        // The smoke passes once the bar clears the traffic lights and a
        // press on it has arrived here.
        match inset {
            Some(px) if smoke && close.is_some() && px < TRAFFIC_LIGHTS => {
                eprintln!("functor-app: window smoke: the title bar's inset is {px}px");
                smoke_failed = true;
                stop(&mut close, geometry.as_deref(), &window);
            }
            Some(px) if smoke && close.is_some() && dragged && !popping => {
                eprintln!("[functor-app] window smoke: title bar inset {px}px; a press on it arrived");
                popping = true;
                smoke_by = Instant::now() + SMOKE_TIMEOUT;
                if let Err(e) = webview.evaluate_script(SMOKE_POP_OUT) {
                    eprintln!("functor-app: window smoke: cannot pop a pane out: {e}");
                }
            }
            _ => {}
        }
        if smoke && close.is_some() && docked {
            eprintln!("[functor-app] window smoke: its window closed and its tabs docked");
            stop(&mut close, geometry.as_deref(), &window);
        }
        if *flow != ControlFlow::Exit {
            *flow = match (&close, smoke) {
                (Some(_), true) => ControlFlow::WaitUntil(smoke_by),
                (Some(_), false) => ControlFlow::Wait,
                (None, _) => ControlFlow::WaitUntil(
                    *stop_by.get_or_insert_with(|| Instant::now() + STOP_TIMEOUT),
                ),
            };
        }
    });
    drop(webview);
    match (smoke_failed, result) {
        (false, Some(result)) => result,
        _ => failed(),
    }
}

/// The window, its webview on `origin` and the menu bar (with its Open
/// Recent submenu).
fn open(
    event_loop: &EventLoop<UserEvent>,
    origin: &str,
    token: &Token,
    geometry: Option<&Path>,
    smoke: bool,
) -> io::Result<(Window, WebView, (Menu, Submenu))> {
    let at = place(geometry.and_then(load), &screens(event_loop));
    // The page's own bar is the title bar: the native one is transparent,
    // under the page, with only its traffic lights showing.
    let window = WindowBuilder::new()
        .with_title("Functor")
        .with_title_hidden(true)
        .with_titlebar_transparent(true)
        .with_fullsize_content_view(true)
        .with_traffic_light_inset(LogicalPosition::new(LIGHTS_AT.0, LIGHTS_AT.1))
        // Dark before the page paints, and the lights drawn for a dark bar.
        .with_theme(Some(Theme::Dark))
        .with_background_color(BACKGROUND)
        .with_inner_size(LogicalSize::new(at.w, at.h))
        .with_min_inner_size(LogicalSize::new(MIN.0, MIN.1))
        .with_position(LogicalPosition::new(at.x, at.y))
        .with_focused(!smoke)
        .build(event_loop)
        .map_err(io::Error::other)?;

    let title = event_loop.create_proxy();
    let ipc = event_loop.create_proxy();
    let ipc_origin = origin.to_string();
    let webview = page_webview(&format!("{origin}/"), origin, token)
        .with_document_title_changed_handler(move |t| {
            let _ = title.send_event(UserEvent::Title(t));
        })
        // Only our page's messages; they only label menu items.
        .with_ipc_handler(move |req| {
            if own(&req.uri().to_string(), &ipc_origin) {
                if let Some(event) = parse_ipc(req.body()) {
                    let _ = ipc.send_event(event);
                }
            }
        })
        .build(&window)
        .map_err(io::Error::other)?;
    let menu = menu().map_err(io::Error::other)?;
    Ok((window, webview, menu))
}

/// A webview on the app's page at `url`: the session token for our own
/// origin only, web links to the browser, and nothing else loads. Every
/// window's page, the main one and each pane's.
pub(crate) fn page_webview<'a>(url: &str, origin: &str, token: &Token) -> WebViewBuilder<'a> {
    let nav_origin = origin.to_string();
    WebViewBuilder::new()
        .with_url(url)
        .with_background_color(BACKGROUND)
        // A press on a window behind others acts at once, so its title bar
        // drags on the first press, as a native one does.
        .with_accept_first_mouse(true)
        // Only on our own page; `server` describes the token's model.
        .with_initialization_script(format!(
            "if (location.origin === {}) {{ window.functorToken = {}; window.functorTitlebar = true; }}",
            serde_json::to_string(origin).expect("a string"),
            serde_json::to_string(token.reveal()).expect("a string"),
        ))
        .with_navigation_handler(move |url| navigate(&url, &nav_origin))
        .with_new_window_req_handler(|url, _| {
            if outside(&url) {
                open_outside(&url);
            }
            NewWindowResponse::Deny
        })
}

/// Every screen, the main one first, in points.
fn screens<T>(target: &tao::event_loop::EventLoopWindowTarget<T>) -> Vec<Rect> {
    target
        .primary_monitor()
        .into_iter()
        .chain(target.available_monitors())
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
        .collect()
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

/// The menu bar: Quit and Hide, New Project (⇧⌘N), Open Folder (⌘O) and
/// Open Recent, the Edit
/// shortcuts (WKWebView takes copy and paste from the menu), and the
/// standard Window items.
fn menu() -> muda::Result<(Menu, Submenu)> {
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
    let new_project = MenuItem::with_id(
        NEW_PROJECT,
        "New Project…",
        true,
        Some(Accelerator::new(
            Modifiers::META | Modifiers::SHIFT,
            Code::KeyN,
        )),
    );
    let open_folder = MenuItem::with_id(
        OPEN_FOLDER,
        "Open Folder…",
        true,
        Some(Accelerator::new(Modifiers::META, Code::KeyO)),
    );
    let open_recent = Submenu::new("Open Recent", false);
    let file = Submenu::with_items("File", true, &[&new_project, &open_folder, &open_recent])?;
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
    Ok((
        Menu::with_items(&[&app, &file, &edit, &window])?,
        open_recent,
    ))
}

/// Replaces Open Recent's items with `recents`; disabled when there are none.
fn fill_recent(menu: &Submenu, recents: &[Recent]) {
    for item in menu.items() {
        if let Some(item) = item.as_menuitem() {
            let _ = menu.remove(item);
        }
    }
    for (i, r) in recents.iter().enumerate() {
        let label = format!("{} — {}", r.name, r.path);
        let _ = menu.append(&MenuItem::with_id(
            format!("{RECENT}{i}"),
            label,
            true,
            None,
        ));
    }
    menu.set_enabled(!recents.is_empty());
}

/// The page's messages: `{"recents": [{"name", "path"}, …]}` (at most
/// [`MAX_RECENTS`]), `{"titlebar": "drag" | "zoom"}`, `{"inset": px}`,
/// `{"windows": [{"pane", "frame", "title"}, …]}` or `{"window": {"show": pane}}`.
fn parse_ipc(body: &str) -> Option<UserEvent> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "lowercase")]
    enum Msg {
        Recents(Vec<Recent>),
        Titlebar(Press),
        Inset(f64),
        Windows(Vec<Wanted>),
        Window(WindowMsg),
    }
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "lowercase")]
    enum WindowMsg {
        Show(u64),
    }
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "lowercase")]
    enum Press {
        Drag,
        Zoom,
    }
    Some(match serde_json::from_str::<Msg>(body).ok()? {
        Msg::Recents(mut list) => {
            list.truncate(MAX_RECENTS);
            UserEvent::Recents(list)
        }
        Msg::Titlebar(Press::Drag) => UserEvent::Drag,
        Msg::Titlebar(Press::Zoom) => UserEvent::Zoom,
        Msg::Inset(px) => UserEvent::Inset(px),
        Msg::Windows(list) => UserEvent::Windows(list),
        Msg::Window(WindowMsg::Show(pane)) => UserEvent::ShowWindow(pane),
    })
}

/// The script that tells the page whether the window is in fullscreen.
fn fullscreen_script(on: bool) -> String {
    format!("document.documentElement.classList.toggle(\"fullscreen\", {on});")
}

/// The script that hands menu item `id`'s command to the page, which sends
/// it as its own buttons do (the server checks every path).
fn menu_script(id: &str, recents: &[Recent]) -> Option<String> {
    let msg = match id {
        NEW_PROJECT => serde_json::json!({"type": "show_new_project", "here": false}),
        OPEN_FOLDER => serde_json::json!({"type": "pick_folder"}),
        _ => {
            let i: usize = id.strip_prefix(RECENT)?.parse().ok()?;
            serde_json::json!({"type": "open_folder", "path": recents.get(i)?.path})
        }
    };
    Some(format!("window.functorMenu?.({msg});"))
}

/// Whether the webview may load `url`: only the app's own page. A web link
/// opens in the browser instead; anything else (files, `about:`, data) is
/// dropped.
fn navigate(url: &str, origin: &str) -> bool {
    if own(url, origin) {
        return true;
    }
    if outside(url) {
        open_outside(url);
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

fn open_outside(url: &str) {
    eprintln!("[functor-app] opening in the browser: {url}");
    match std::process::Command::new("/usr/bin/open").arg(url).spawn() {
        // Reaped off the main thread.
        Ok(mut child) => drop(std::thread::spawn(move || child.wait())),
        Err(e) => eprintln!("functor-app: cannot open {url}: {e}"),
    }
}

/// A window's content or a screen, in points, with y down from the top of the
/// main screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Where the window opens: where it was, if its title bar can still be
/// grabbed on one of `screens`; else that size (or the default), fitted to
/// the first screen and centred on it.
fn place(saved: Option<Rect>, screens: &[Rect]) -> Rect {
    place_sized(saved, screens, DEFAULT, MIN)
}

/// [`place`] for a window with its own default and least size.
pub(crate) fn place_sized(
    saved: Option<Rect>,
    screens: &[Rect],
    default: (f64, f64),
    min: (f64, f64),
) -> Rect {
    let size = |r: Rect| Rect {
        w: r.w.max(min.0),
        h: r.h.max(min.1),
        ..r
    };
    if let Some(saved) = saved.map(size) {
        if screens.iter().any(|s| grabbable(saved, *s)) {
            return saved;
        }
    }
    let (w, h) = saved.map_or(default, |s| (s.w, s.h));
    let Some(screen) = screens.first() else {
        return size(Rect {
            x: 0.0,
            y: 0.0,
            w,
            h,
        });
    };
    let (w, h) = (w.min(screen.w).max(min.0), h.min(screen.h).max(min.1));
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
    if window.fullscreen().is_some() {
        return;
    }
    let Some(rect) = content_rect(window) else {
        return;
    };
    if let Err(e) = crate::store::write_whole(path, &format(rect)) {
        eprintln!("functor-app: cannot save {}: {e}", path.display());
    }
}

/// Where `window`'s content is, in points (its corner is what
/// `with_position` places on macOS); `None` while minimised.
pub(crate) fn content_rect(window: &Window) -> Option<Rect> {
    if window.is_minimized() {
        return None;
    }
    let scale = window.scale_factor();
    let at = window.inner_position().ok()?.to_logical::<f64>(scale);
    let size = window.inner_size().to_logical::<f64>(scale);
    Some(Rect {
        x: at.x,
        y: at.y,
        w: size.width,
        h: size.height,
    })
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
    fn menu_commands_reach_the_page_as_messages() {
        let recents = [recent("hello", "/a/\"hello\"');")];
        assert_eq!(
            menu_script(OPEN_FOLDER, &recents).unwrap(),
            r#"window.functorMenu?.({"type":"pick_folder"});"#
        );
        // The path is JSON, so no quote in it ends the script's string.
        assert_eq!(
            menu_script("recent-0", &recents).unwrap(),
            r#"window.functorMenu?.({"path":"/a/\"hello\"');","type":"open_folder"});"#
        );
        assert_eq!(
            menu_script(NEW_PROJECT, &recents).unwrap(),
            r#"window.functorMenu?.({"here":false,"type":"show_new_project"});"#
        );
        for id in ["recent-1", "recent-x", "quit", ""] {
            assert_eq!(menu_script(id, &recents), None, "{id}");
        }
    }

    fn parse_recents(body: &str) -> Option<Vec<Recent>> {
        match parse_ipc(body)? {
            UserEvent::Recents(list) => Some(list),
            _ => None,
        }
    }

    #[test]
    fn title_bar_presses_from_the_page() {
        let press = |body| parse_ipc(body);
        assert!(matches!(press(r#"{"titlebar":"drag"}"#), Some(UserEvent::Drag)));
        assert!(matches!(press(r#"{"titlebar":"zoom"}"#), Some(UserEvent::Zoom)));
        assert!(matches!(press(r#"{"inset":84}"#), Some(UserEvent::Inset(px)) if px == 84.0));
        for bad in [r#"{"titlebar":"close"}"#, r#"{"titlebar":1}"#, r#"{"inset":"84px"}"#] {
            assert!(press(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn recents_from_the_page_are_names_and_paths() {
        let body = r#"{"recents":[{"name":"a","path":"/a"},{"name":"b","path":"/b"}]}"#;
        let ab = vec![recent("a", "/a"), recent("b", "/b")];
        assert_eq!(parse_recents(body), Some(ab));
        for bad in ["", "{}", r#"{"recents":[{"name":1,"path":"/a"}]}"#] {
            assert_eq!(parse_recents(bad), None, "{bad}");
        }
        let many = (0..20).map(|i| format!(r#"{{"name":"{i}","path":"/{i}"}}"#));
        let body = format!(r#"{{"recents":[{}]}}"#, many.collect::<Vec<_>>().join(","));
        assert_eq!(parse_recents(&body).unwrap().len(), MAX_RECENTS);
    }

    fn recent(name: &str, path: &str) -> Recent {
        Recent {
            name: name.into(),
            path: path.into(),
            missing: false,
        }
    }

    #[test]
    fn web_and_mail_links_go_outside_and_nothing_else_does() {
        for url in [
            "https://example.com/a",
            "HTTP://example.com",
            "mailto:a@b.c",
        ] {
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
        assert_eq!(
            (at.x, at.y),
            ((1728.0 - 1440.0) / 2.0, (1117.0 - 900.0) / 2.0)
        );
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
        assert_eq!(
            place(Some(small), &[SCREEN]),
            Rect {
                w: MIN.0,
                h: MIN.1,
                ..edge
            }
        );
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
        for text in [
            "",
            "{}",
            "[1,2,3,4]",
            r#"{"x":1,"y":2,"width":3}"#,
            "not json",
        ] {
            assert_eq!(parse(text), None, "{text}");
        }
    }
}
