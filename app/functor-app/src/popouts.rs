//! Panes in windows of their own (D20), in the macOS app: each a second
//! window on the same page in its single-pane mode (`/?pane=N`), sharing the
//! server and the session token. The main window's page sends the list the
//! active project's layout holds (`{"windows": [...]}`); this opens what is
//! missing and closes what is gone (a dock, a project switch), and reports
//! what the user does to one: closing it (its tabs then dock) and moving or
//! resizing it. Each window's title is its page's.

use std::collections::HashMap;

use functor_app_core::layout::Frame;
use tao::dpi::{LogicalPosition, LogicalSize};
use tao::event_loop::EventLoopWindowTarget;
use tao::window::{Theme, Window, WindowBuilder, WindowId};
use wry::WebView;

use crate::server::Token;
use crate::window::{content_rect, page_webview, place_sized, Rect};

/// A pop-out's size when it has none of its own yet, and the least (points).
const DEFAULT: (f64, f64) = (900.0, 640.0);
const MIN: (f64, f64) = (420.0, 300.0);

/// A window the layout lists: its project and pane, and where it last was.
#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub(crate) struct Wanted {
    pub project: String,
    pub pane: u64,
    pub frame: Option<Frame>,
}

struct Open {
    project: String,
    pane: u64,
    // Dropped before its window, as the main window's webview is.
    _webview: WebView,
    window: Window,
}

impl Open {
    fn is(&self, w: &Wanted) -> bool {
        (self.project.as_str(), self.pane) == (w.project.as_str(), w.pane)
    }
}

/// The open pop-out windows.
#[derive(Default)]
pub(crate) struct PopOuts {
    open: HashMap<WindowId, Open>,
}

impl PopOuts {
    /// Opens each of `wanted` not open (taking focus when `focus`), closes
    /// each open one not in it. `title` gets a page's document title, by pane.
    pub(crate) fn reconcile<T: 'static>(
        &mut self,
        target: &EventLoopWindowTarget<T>,
        wanted: &[Wanted],
        page: &Page,
        screens: &[Rect],
        focus: bool,
        title: impl Fn(u64, String) + Clone + 'static,
    ) {
        self.open.retain(|_, o| wanted.iter().any(|w| o.is(w)));
        for w in wanted {
            if self.open.values().any(|o| o.is(w)) {
                continue;
            }
            match open(target, w, page, screens, focus, title.clone()) {
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
        Some((o.pane, content_rect(&o.window)?))
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
    focus: bool,
    title: impl Fn(u64, String) + 'static,
) -> Result<Open, String> {
    let saved = wanted.frame.map(|f| Rect {
        x: f.x.into(),
        y: f.y.into(),
        w: f.width.into(),
        h: f.height.into(),
    });
    // Where it was if that is still on a screen, else centred.
    let at = place_sized(saved, screens, DEFAULT, MIN);
    let window = WindowBuilder::new()
        .with_title("Functor")
        .with_theme(Some(Theme::Dark))
        .with_inner_size(LogicalSize::new(at.w, at.h))
        .with_min_inner_size(LogicalSize::new(MIN.0, MIN.1))
        .with_position(LogicalPosition::new(at.x, at.y))
        .with_focused(focus)
        .build(target)
        .map_err(|e| e.to_string())?;
    let pane = wanted.pane;
    let url = format!("{}/?pane={pane}", page.origin);
    let webview = page_webview(&url, &page.origin, &page.token)
        .with_document_title_changed_handler(move |t| title(pane, t))
        .build(&window)
        .map_err(|e| e.to_string())?;
    Ok(Open {
        project: wanted.project.clone(),
        pane,
        _webview: webview,
        window,
    })
}

/// The script telling the main window's page that the user closed pane
/// `pane`'s window (it docks its tabs).
pub(crate) fn closed_script(pane: u64) -> String {
    format!("window.functorWindowClosed?.({pane});")
}

/// The script telling the main window's page where pane `pane`'s window is.
pub(crate) fn frame_script(pane: u64, r: Rect) -> String {
    let frame = Frame {
        x: r.x.round() as i32,
        y: r.y.round() as i32,
        width: r.w.round().max(0.0) as u32,
        height: r.h.round().max(0.0) as u32,
    };
    let frame = serde_json::to_string(&frame).expect("a frame serializes");
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
            r#"window.functorWindowFrame?.(7, {"x":-1200,"y":41,"width":900,"height":640});"#
        );
    }

    #[test]
    fn the_list_names_each_project_pane_and_frame() {
        let list: Vec<Wanted> = serde_json::from_str(
            r#"[{"project":"a","pane":4,"frame":{"x":10,"y":20,"width":800,"height":600}},
                {"project":"a","pane":7,"frame":null}]"#,
        )
        .unwrap();
        assert_eq!(list[0].frame.unwrap().width, 800);
        assert_eq!((list[1].pane, list[1].frame), (7, None));
    }
}
