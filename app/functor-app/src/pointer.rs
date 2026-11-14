//! The game view's pointer lock in the window. WKWebView refuses the Pointer
//! Lock API, so the window locks instead: on the page's `{"pointer": "lock"}`
//! it holds the cursor still (tao's cursor grab) and hands the page the
//! mouse's deltas, once per batch of events, as `window.functorPointer`. It
//! lets go on the page's unlock, on a page load, when the window loses focus
//! (telling the page, so the game view releases) and on quitting.

/// The lock and the motion not yet handed to the page.
#[derive(Default)]
pub(crate) struct PointerLock {
    locked: bool,
    dx: f64,
    dy: f64,
}

impl PointerLock {
    /// Locks or unlocks; the cursor grab to apply when that changes it.
    pub(crate) fn set(&mut self, lock: bool) -> Option<bool> {
        if self.locked == lock {
            return None;
        }
        *self = Self {
            locked: lock,
            ..Self::default()
        };
        Some(lock)
    }

    /// The mouse moved by `delta` points.
    pub(crate) fn motion(&mut self, (dx, dy): (f64, f64)) {
        if self.locked {
            self.dx += dx;
            self.dy += dy;
        }
    }

    /// The script handing the page what moved since the last one, if anything.
    pub(crate) fn flush(&mut self) -> Option<String> {
        if !self.locked || (self.dx == 0.0 && self.dy == 0.0) {
            return None;
        }
        let script = format!("window.functorPointer?.({{dx:{},dy:{}}});", self.dx, self.dy);
        (self.dx, self.dy) = (0.0, 0.0);
        Some(script)
    }

    /// Lets go on the window's own account: the script telling the page, if
    /// it was locked (the caller drops the grab).
    pub(crate) fn lose(&mut self) -> Option<&'static str> {
        self.set(false)
            .map(|_| "window.functorPointer?.({lost:true});")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deltas_reach_the_page_only_while_locked_once_per_batch() {
        let mut p = PointerLock::default();
        p.motion((5.0, 5.0));
        assert_eq!(p.flush(), None);
        assert_eq!(p.set(true), Some(true));
        assert_eq!(p.set(true), None);
        p.motion((1.5, -2.0));
        p.motion((0.5, 0.0));
        assert_eq!(p.flush().as_deref(), Some("window.functorPointer?.({dx:2,dy:-2});"));
        assert_eq!(p.flush(), None);
        p.motion((3.0, 0.0));
        // Unlocking drops what was not handed over.
        assert_eq!(p.set(false), Some(false));
        assert_eq!(p.flush(), None);
    }

    #[test]
    fn losing_focus_tells_the_page_only_when_locked() {
        let mut p = PointerLock::default();
        assert_eq!(p.lose(), None);
        p.set(true);
        assert_eq!(p.lose(), Some("window.functorPointer?.({lost:true});"));
        assert_eq!(p.lose(), None);
    }
}
