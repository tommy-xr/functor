use std::fmt;

use fable_library_rust::{NativeArray_, Native_::Func1};

use crate::net::{self, HttpMethod, HttpResult, NetCommand};

#[derive(Clone)]
pub enum Effect<T: Clone + 'static> {
    None,
    Wrapped(T),
    /// An HTTP request (the Elm `Http.get { expect = ... }`). `command` is the
    /// plain-data request the host performs; `tagger` maps the eventual result to
    /// a message. When the effect runs, the command goes to the outbound queue and
    /// the tagger into the pending-request registry (keyed by the command's
    /// token); the response is delivered as a message later. The tagger is a
    /// closure, so the effect queue is no longer persisted across hot reload.
    Http {
        command: NetCommand,
        tagger: Func1<HttpResult, T>,
    },
}

// Implement Debug manually so it doesn't require `T: Debug`. This lets types
// that embed an Effect (e.g. the generated Game record) derive Debug, while
// keeping effects opaque - the payload is plain data but not necessarily printable.
impl<T: Clone + 'static> fmt::Debug for Effect<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Effect::None => f.write_str("Effect::None"),
            Effect::Wrapped(_) => f.write_str("Effect::Wrapped(..)"),
            Effect::Http { command, .. } => write!(f, "Effect::Http({command:?})"),
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

    /// Build an HTTP request effect. `tagger` maps the eventual result into a
    /// message (the Elm `expect`); `token` correlates request and response.
    pub fn http(
        token: u64,
        method: HttpMethod,
        url: String,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
        tagger: Func1<HttpResult, T>,
    ) -> Effect<T> {
        Effect::Http {
            command: NetCommand::HttpRequest {
                token,
                method,
                url,
                headers,
                body,
            },
            tagger,
        }
    }

    pub fn map<U: Clone + 'static>(mapping: Func1<T, U>, source: Effect<T>) -> Effect<U> {
        match source {
            Effect::None => Effect::None,
            Effect::Wrapped(v) => Effect::Wrapped(mapping(v)),
            // Compose the mapping after the tagger so the request still resolves to
            // the new message type (Elm's Cmd.map over an Http command).
            Effect::Http { command, tagger } => Effect::Http {
                command,
                tagger: Func1::new(move |result: HttpResult| mapping(tagger(result))),
            },
        }
    }

    pub fn run(effect: Effect<T>) -> NativeArray_::Array<T> {
        match effect {
            Effect::None => NativeArray_::array_from(vec![]),
            Effect::Wrapped(v) => NativeArray_::array_from(vec![v]),
            // Register the tagger (keyed by token) and hand the command to the host
            // via the outbound queue; the result returns later through the inbox
            // and is matched back to this tagger. No in-frame message.
            Effect::Http { command, tagger } => {
                let NetCommand::HttpRequest { token, .. } = &command;
                net::register_tagger(*token, tagger);
                net::push_command(command);
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
    fn run_http_queues_command_and_registers_tagger() {
        // Clear anything a prior run left behind, then perform an HTTP effect whose
        // tagger turns the result's status into a message.
        let _ = net::drain_commands();
        let eff: Effect<i32> = Effect::http(
            99,
            HttpMethod::Get,
            "https://example.com".to_string(),
            vec![],
            vec![],
            Func1::new(|r: HttpResult| r.status as i32),
        );
        let msgs = Effect::run(eff);
        assert_eq!(NativeArray_::count(msgs), 0);

        // The plain command went to the outbound queue...
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

        // ...and the tagger was registered: a response for token 99 maps to 200.
        let result = HttpResult {
            token: 99,
            status: 200,
            body: vec![],
            error: None,
        };
        assert_eq!(net::take_pending::<i32>(result), Some(200));
    }
}
