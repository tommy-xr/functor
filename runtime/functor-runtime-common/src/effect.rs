use std::fmt;

use fable_library_rust::{NativeArray_, Native_::Func1};

use crate::net::{self, HttpMethod, NetCommand};

#[derive(Clone)]
pub enum Effect<T: Clone + 'static> {
    None,
    Wrapped(T),
    /// A networking side effect: plain data describing I/O for the host shell to
    /// perform. It produces no in-frame message; the result arrives later through
    /// the async inbox and is turned into a message by the matching `Sub`. Kept
    /// plain data so it survives hot reload like any other queued effect.
    Command(NetCommand),
}

// Implement Debug manually so it doesn't require `T: Debug`. This lets types
// that embed an Effect (e.g. the generated Game record) derive Debug, while
// keeping effects opaque - the payload is plain data but not necessarily printable.
impl<T: Clone + 'static> fmt::Debug for Effect<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Effect::None => f.write_str("Effect::None"),
            Effect::Wrapped(_) => f.write_str("Effect::Wrapped(..)"),
            Effect::Command(cmd) => write!(f, "Effect::Command({cmd:?})"),
        }
    }
}

impl<T: Clone + 'static> Effect<T> {
    pub fn is_none(effect: &Effect<T>) -> bool {
        match effect {
            Effect::None => true,
            _ => false,
        }
    }

    pub fn none() -> Effect<T> {
        Effect::None
    }

    pub fn wrapped(data: T) -> Effect<T> {
        Effect::Wrapped(data)
    }

    /// Build an HTTP request effect. `token` is echoed back on the response so the
    /// game can correlate it (see `net::NetInbound`).
    pub fn http_request(
        token: u64,
        method: HttpMethod,
        url: String,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    ) -> Effect<T> {
        Effect::Command(NetCommand::HttpRequest {
            token,
            method,
            url,
            headers,
            body,
        })
    }

    pub fn map<U: Clone + 'static>(mapping: Func1<T, U>, source: Effect<T>) -> Effect<U> {
        match source {
            Effect::None => Effect::None,
            Effect::Wrapped(v) => Effect::Wrapped(mapping(v)),
            // A command carries no message, so remapping the message type is a
            // no-op on the payload -- just re-tag it under the new type.
            Effect::Command(cmd) => Effect::Command(cmd),
        }
    }

    pub fn run(effect: Effect<T>) -> NativeArray_::Array<T> {
        match effect {
            Effect::None => NativeArray_::array_from(vec![]),
            Effect::Wrapped(v) => NativeArray_::array_from(vec![v]),
            // Hand the command to the host (via the outbound queue) and produce no
            // in-frame message; the result returns later through the inbox.
            Effect::Command(cmd) => {
                net::push_command(cmd);
                NativeArray_::array_from(vec![])
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_none_distinguishes_variants() {
        assert!(Effect::is_none(&Effect::<i32>::none()));
        assert!(!Effect::is_none(&Effect::wrapped(1)));
    }

    #[test]
    fn run_none_yields_no_messages() {
        let msgs = Effect::<i32>::run(Effect::none());
        assert_eq!(NativeArray_::count(msgs), 0);
    }

    #[test]
    fn run_wrapped_yields_single_message() {
        let msgs = Effect::run(Effect::wrapped(42));
        assert_eq!(NativeArray_::count(msgs.clone()), 1);
        assert!(NativeArray_::contains(msgs, 42));
    }

    #[test]
    fn run_command_queues_outbound_and_yields_no_message() {
        // Clear anything a prior run left behind, then perform an HTTP effect.
        let _ = net::drain_commands();
        let eff: Effect<i32> = Effect::http_request(
            99,
            HttpMethod::Get,
            "https://example.com".to_string(),
            vec![],
            vec![],
        );
        let msgs = Effect::run(eff);
        assert_eq!(NativeArray_::count(msgs), 0);

        let queued = net::drain_commands();
        assert_eq!(
            queued,
            vec![NetCommand::HttpRequest {
                token: 99,
                method: HttpMethod::Get,
                url: "https://example.com".to_string(),
                headers: vec![],
                body: vec![],
            }]
        );
    }
}
