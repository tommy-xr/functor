//! Request-token minting for outbound networking.
//!
//! An HTTP request correlates with its later response by a `token` chosen when
//! the request is made and echoed back on the result. This module mints those
//! tokens. The MLE producer holds the in-flight taggers keyed by token in its
//! own per-session registry (see `mle_prelude`), so nothing type-erased lives
//! here — just a monotonic counter.

use std::cell::Cell;

thread_local! {
    static NEXT_TOKEN: Cell<u64> = const { Cell::new(1) };
}

/// A fresh correlation token for an outbound request.
pub fn next_token() -> u64 {
    NEXT_TOKEN.with(|c| {
        let token = c.get();
        c.set(token + 1);
        token
    })
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
}
