//! Pending-request registry: the runtime's hold for an in-flight request's
//! tagger (docs/multiplayer.md, Elm-style HTTP API).
//!
//! An HTTP request `Cmd` carries a `tagger: HttpResult -> Msg` (the Elm
//! `expect`). The request→response gap spans many frames, so the tagger must be
//! held somewhere across it. It can't ride in the persisted state (a closure is
//! code in the current dylib; after a hot-reload swap it would dangle), so it
//! lives here: a thread-local table keyed by the request's token, populated when
//! the effect runs and drained when the response lands.
//!
//! On hot reload this table is dropped with the old dylib, so an in-flight
//! request loses its tagger and its response is dropped (with a warning from the
//! executor) rather than crashing. The game loop is single-threaded, so a
//! thread-local (no `Send` needed -- `Func1` is `Rc`-backed) is exactly right.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use fable_library_rust::Native_::Func1;

use super::HttpResult;

thread_local! {
    // Box<dyn Any> erases the Msg type so one table serves any game; the executor
    // (which knows Msg) downcasts on the way out. In practice every entry is the
    // same concrete `Func1<HttpResult, GameMsg>`.
    static PENDING: RefCell<HashMap<u64, Box<dyn Any>>> = RefCell::new(HashMap::new());
    static NEXT_TOKEN: Cell<u64> = Cell::new(1);
}

/// A fresh correlation token for an outbound request.
pub fn next_token() -> u64 {
    NEXT_TOKEN.with(|c| {
        let token = c.get();
        c.set(token + 1);
        token
    })
}

/// Store the tagger for `token`, holding it across the request→response gap.
pub fn register_tagger<M: 'static>(token: u64, tagger: Func1<HttpResult, M>) {
    PENDING.with(|p| {
        p.borrow_mut().insert(token, Box::new(tagger));
    });
}

/// Take the tagger for this result's token and apply it, yielding the message.
/// Returns `None` when no tagger is registered -- an unknown token, or one whose
/// tagger was dropped by a hot reload while the request was in flight.
pub fn take_pending<M: 'static>(result: HttpResult) -> Option<M> {
    let boxed = PENDING.with(|p| p.borrow_mut().remove(&result.token))?;
    match boxed.downcast::<Func1<HttpResult, M>>() {
        Ok(tagger) => {
            let tagger = *tagger;
            Some(tagger(result))
        }
        Err(_) => None,
    }
}

/// Number of requests awaiting a response.
pub fn pending_count() -> i32 {
    PENDING.with(|p| p.borrow().len() as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_monotonic() {
        let a = next_token();
        let b = next_token();
        assert!(b > a);
    }

    #[test]
    fn register_then_take_applies_the_tagger() {
        let token = next_token();
        register_tagger(token, Func1::new(|r: HttpResult| r.status as i64 + 1));
        let result = HttpResult {
            token,
            status: 200,
            body: vec![],
            error: None,
        };
        assert_eq!(take_pending::<i64>(result), Some(201));
    }

    #[test]
    fn take_unknown_token_is_none() {
        let result = HttpResult {
            token: 999_999,
            status: 200,
            body: vec![],
            error: None,
        };
        assert_eq!(take_pending::<i64>(result), None);
    }

    #[test]
    fn take_consumes_the_entry() {
        let token = next_token();
        register_tagger(token, Func1::new(|_r: HttpResult| 7i64));
        let first = HttpResult { token, status: 200, body: vec![], error: None };
        let second = HttpResult { token, status: 200, body: vec![], error: None };
        assert_eq!(take_pending::<i64>(first), Some(7));
        // Second time the tagger is gone.
        assert_eq!(take_pending::<i64>(second), None);
    }
}
