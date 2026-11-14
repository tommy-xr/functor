//! Panes in windows of their own (D20), in the macOS app: each a second
//! window on the same page in its single-pane mode (`/?pane=N`), sharing the
//! server and the session token. The main window's page sends the list the
//! layout holds (`{"windows": [...]}`); this opens what is missing and closes
//! what is gone (a project switch, a dock), and reports what the user does
//! to one: closing it (its tabs then dock) and moving or resizing it.

use std::collections::HashMap;

use tao::dpi::{LogicalPosition, LogicalSize};
use tao::event_loop::EventLoopWindowTarget;
use tao::window::{Theme, Window, WindowBuilder, WindowId};
use wry::{NewWindowResponse, WebView, WebViewBuilder};

use crate::server::Token;
use crate::window::{navigate, open_outside, outside, place_sized, Rect, BACKGROUND};

/// A pop-out's size when it has none of its own yet, and the least (points).
const DEFAULT: (f64, f64) = (900.0, 640.0);
const MIN: (f64, f64) = (420.0, 300.0);

/// A window the layout lists: its pane, where it last was, and its title.
#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub(crate) struct Wanted {
    pub pane: u64,
    pub frame: Option<WantedFrame>,
    pub title: String,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize)]
pub(crate) struct WantedFrame {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

struct Open {
    pane: u64,
    window: Window,
    _webview: WebView,
}

/// The open pop-out windows.
#[derive(Default)]
pub(crate) struct PopOuts {
    open: HashMap<WindowId, Open>,
}

impl PopOuts {
    /// Opens each of `wanted` not open, closes each open one not in it.
    /// `title` gets a pop-out page's document title, by pane.
    pub(crate) fn reconcile<T: 'static>(
        &mut self,
        target: &EventLoopWindowTarget<T>,
        wanted: &[Wanted],
        page: &Page,
        screens: &[Rect],
        title: impl Fn(u64, String) + Clone + 'static,
    ) {
        self.open
            .retain(|_, o| wanted.iter().any(|w| w.pane == o.pane));
        for w in wanted {
            if self.open.values().any(|o| o.pane == w.pane) {
                continue;
            }
            match open(target, w, page, screens, title.clone()) {
                Ok(o) => {
                    self.open.insert(o.window.id(), o);
                }
                Err(e) => eprintln!("functor-app: cannot open the window for pane {}: {e}", w.pane),
            }
        }
    }

    /// Brings pane `pane`'s window to the front.
    pub(crate) fn show(&self, pane: u64) {
        if let Some(o) = self.open.values().find(|o| o.pane == pane) {
            o.window.set_minimized(false);
            o.window.set_focus();
        }
    }

    pub(crate) fn set_title(&self, pane: u64, title: &str) {
        if let Some(o) = self.open.values().find(|o| o.pane == pane) {
            o.window.set_title(title);
        }
    }

    /// Closes the window `id` the user closed; its pane, if it was a pop-out.
    pub(crate) fn close(&mut self, id: WindowId) -> Option<u64> {
        self.open.remove(&id).map(|o| o.pane)
    }

    /// Closes pane `pane`'s window, as if the user had (`--window-smoke`).
    pub(crate) fn close_pane(&mut self, pane: u64) -> bool {
        let before = self.open.len();
        self.open.retain(|_, o| o.pane != pane);
        self.open.len() != before
    }

    /// Window `id`'s pane and where it is now, if it is a pop-out.
    pub(crate) fn frame(&self, id: WindowId) -> Option<(u64, Rect)> {
        let o = self.open.get(&id)?;
        if o.window.is_minimized() {
            return None;
        }
        let scale = o.window.scale_factor();
        let at = o.window.inner_position().ok()?.to_logical::<f64>(scale);
        let size = o.window.inner_size().to_logical::<f64>(scale);
        Some((
            o.pane,
            Rect {
                x: at.x,
                y: at.y,
                w: size.width,
                h: size.height,
            },
        ))
    }
}

/// What every pop-out page loads from: the app's origin and its token.
pub(crate) struct Page {
    pub origin: String,
    pub token: Token,
}

fn open<T: 'static>(
    target: &EventLoopWindowTarget<T>,
    wanted: &Wanted,
    page: &Page,
    screens: &[Rect],
    title: impl Fn(u64, String) + 'static,
) -> Result<Open, String> {
    let saved = wanted.frame.map(|f| Rect {
        x: f.x,
        y: f.y,
        w: f.width,
        h: f.height,
    });
    // Where it was if that is still on a screen, else centred.
    let at = place_sized(saved, screens, DEFAULT, MIN);
    let window = WindowBuilder::new()
        .with_title(&wanted.title)
        .with_theme(Some(Theme::Dark))
        .with_background_color(BACKGROUND)
        .with_inner_size(LogicalSize::new(at.w, at.h))
        .with_min_inner_size(LogicalSize::new(MIN.0, MIN.1))
        .with_position(LogicalPosition::new(at.x, at.y))
        .build(target)
        .map_err(|e| e.to_string())?;
    let origin = page.origin.clone();
    let pane = wanted.pane;
    let webview = WebViewBuilder::new()
        .with_url(format!("{}/?pane={pane}", page.origin))
        .with_background_color(BACKGROUND)
        .with_accept_first_mouse(true)
        // As the main window's: the token on our own page only.
        .with_initialization_script(format!(
            "if (location.origin === {}) {{ window.functorToken = {}; window.functorTitlebar = true; }}",
            serde_json::to_string(&page.origin).expect("a string"),
            serde_json::to_string(page.token.reveal()).expect("a string"),
        ))
        .with_navigation_handler(move |url| navigate(&url, &origin))
        .with_new_window_req_handler(|url, _| {
            if outside(&url) {
                open_outside(&url);
            }
            NewWindowResponse::Deny
        })
        .with_document_title_changed_handler(move |t| title(pane, t))
        .build(&window)
        .map_err(|e| e.to_string())?;
    Ok(Open {
        pane,
        window,
        _webview: webview,
    })
}

/// The script telling the main window's page that the user closed pane
/// `pane`'s window (it docks its tabs).
pub(crate) fn closed_script(pane: u64) -> String {
    format!("window.functorWindowClosed?.({pane});")
}

/// The script telling the main window's page where pane `pane`'s window is.
pub(crate) fn frame_script(pane: u64, r: Rect) -> String {
    let frame = serde_json::json!({
        "x": r.x.round() as i64,
        "y": r.y.round() as i64,
        "width": r.w.round().max(0.0) as u64,
        "height": r.h.round().max(0.0) as u64,
    });
    format!("window.functorWindowFrame?.({pane}, {frame});")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_hears_of_a_closed_or_moved_window() {
        assert_eq!(closed_script(7), "window.functorWindowClosed?.(7);");
        let r = Rect {
            x: -1200.4,
            y: 40.6,
            w: 900.0,
            h: 640.2,
        };
        assert_eq!(
            frame_script(7, r),
            r#"window.functorWindowFrame?.(7, {"height":640,"width":900,"x":-1200,"y":41});"#
        );
    }

    #[test]
    fn the_list_names_each_pane_its_frame_and_title() {
        let list: Vec<Wanted> = serde_json::from_str(
            r#"[{"pane":4,"frame":{"x":10,"y":20,"width":800,"height":600},"title":"a.fun"},
                {"pane":7,"frame":null,"title":"Game"}]"#,
        )
        .unwrap();
        assert_eq!(list[0].frame.unwrap().width, 800.0);
        assert_eq!((list[1].pane, list[1].frame, list[1].title.as_str()), (7, None, "Game"));
    }
}
