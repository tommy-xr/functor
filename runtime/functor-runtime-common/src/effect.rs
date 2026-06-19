use std::fmt;

use fable_library_rust::{NativeArray_, Native_::Func1};

use crate::audio::{self, AudioCommand};
use crate::net::{self, ConnCommand, HttpMethod, HttpResult, NetCommand};

#[derive(Clone)]
pub enum Effect<T: Clone + 'static> {
    None,
    Wrapped(T),
    /// A fire-and-forget audio command (e.g. `Audio.play "gunshot.wav"`). When
    /// the effect runs, the command goes to the audio outbound queue for the
    /// host to perform; there is no message back.
    PlayAudio(AudioCommand),
    /// An audio one-shot that delivers `on_finished` as a message when the sound
    /// ends (`Audio.playThen`) — the audio twin of `Http`'s tagger. The command
    /// carries a token; running the effect registers `on_finished` under it, and
    /// the host reports completion back through the inbox (`audio_push_finished`).
    PlayAudioThen {
        command: AudioCommand,
        on_finished: T,
    },
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
    /// A persistent-connection command (send/close). Plain data, no message; the
    /// host performs it. Inbound events arrive separately via the connection
    /// inbox and are decoded by the connection's `Sub`.
    Conn(ConnCommand),
}

// Implement Debug manually so it doesn't require `T: Debug`. This lets types
// that embed an Effect (e.g. the generated Game record) derive Debug, while
// keeping effects opaque - the payload is plain data but not necessarily printable.
impl<T: Clone + 'static> fmt::Debug for Effect<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Effect::None => f.write_str("Effect::None"),
            Effect::Wrapped(_) => f.write_str("Effect::Wrapped(..)"),
            Effect::PlayAudio(command) => write!(f, "Effect::PlayAudio({command:?})"),
            Effect::PlayAudioThen { command, .. } => {
                write!(f, "Effect::PlayAudioThen({command:?})")
            }
            Effect::Http { command, .. } => write!(f, "Effect::Http({command:?})"),
            Effect::Conn(cmd) => write!(f, "Effect::Conn({cmd:?})"),
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

    /// A fire-and-forget audio command (e.g. play a one-shot sound).
    pub fn play_audio(command: AudioCommand) -> Effect<T> {
        Effect::PlayAudio(command)
    }

    /// A one-shot that delivers `on_finished` as a message when it ends. `token`
    /// correlates the play with the host's completion report.
    pub fn play_audio_then(token: u64, sound: String, on_finished: T) -> Effect<T> {
        Effect::PlayAudioThen {
            command: AudioCommand::play_one_shot_token(token, sound),
            on_finished,
        }
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
            // No message to remap — the command carries through unchanged.
            Effect::PlayAudio(cmd) => Effect::PlayAudio(cmd),
            // Remap the completion message to the new type.
            Effect::PlayAudioThen {
                command,
                on_finished,
            } => Effect::PlayAudioThen {
                command,
                on_finished: mapping(on_finished),
            },
            // Compose the mapping after the tagger so the request still resolves to
            // the new message type (Elm's Cmd.map over an Http command).
            Effect::Http { command, tagger } => Effect::Http {
                command,
                tagger: Func1::new(move |result: HttpResult| mapping(tagger(result))),
            },
            // No message, so the type change is a no-op on the payload.
            Effect::Conn(cmd) => Effect::Conn(cmd),
        }
    }

    pub fn run(effect: Effect<T>) -> NativeArray_::Array<T> {
        match effect {
            Effect::None => NativeArray_::array_from(vec![]),
            Effect::Wrapped(v) => NativeArray_::array_from(vec![v]),
            // Hand the command to the host via the audio outbound queue; no
            // in-frame (or later) message.
            Effect::PlayAudio(cmd) => {
                audio::push_command(cmd);
                NativeArray_::array_from(vec![])
            }
            // Register the completion message under the command's token, then
            // queue the command. The message returns later via the inbox.
            Effect::PlayAudioThen {
                command,
                on_finished,
            } => {
                if let AudioCommand::PlayOneShot {
                    token: Some(tok), ..
                } = &command
                {
                    audio::register_completion(*tok, on_finished);
                }
                audio::push_command(command);
                NativeArray_::array_from(vec![])
            }
            // Register the tagger (keyed by token) and hand the command to the host
            // via the outbound queue; the result returns later through the inbox
            // and is matched back to this tagger. No in-frame message.
            Effect::Http { command, tagger } => {
                let NetCommand::HttpRequest { token, .. } = &command;
                net::register_tagger(*token, tagger);
                net::push_command(command);
                NativeArray_::array_from(vec![])
            }
            // Hand the connection command to the host; no in-frame message.
            Effect::Conn(cmd) => {
                net::push_conn_command(cmd);
                NativeArray_::array_from(vec![])
            }
        }
    }

    /// Build a persistent-connection command effect (send/close).
    pub fn conn(command: ConnCommand) -> Effect<T> {
        Effect::Conn(command)
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
