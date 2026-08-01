//! Inbound events delivered by an *embedding page* rather than by a socket.
//!
//! The web runtime can route its networking to the page that embeds it (the
//! "embedder" transport — see `runtime/functor-runtime-web/src/lib.rs`): it
//! posts its drained [`ConnCommand`](super::ConnCommand)s outward and the
//! embedder posts events back in. This is the wire shape of that return path,
//! mirroring the four `net_push_*` producer methods one-for-one:
//!
//! ```json
//! [{ "kind": "connected",    "key": "ws://…", "conn": 1 },
//!  { "kind": "message",      "key": "ws://…", "conn": 1, "text": "…" },
//!  { "kind": "disconnected", "key": "ws://…", "conn": 1 },
//!  { "kind": "error",        "key": "ws://…", "conn": 1, "message": "…" }]
//! ```
//!
//! The two directions of the seam use different shapes ON PURPOSE. Egress is
//! the drained [`ConnCommand`](super::ConnCommand) JSON verbatim — the already
//! versioned logic↔shell protocol type, so the embedder consumes exactly what
//! every other host consumes (byte payloads included; an embedder that wants
//! text decodes UTF-8 once). Ingress cannot be `NetEvent` or a mirrored
//! `ConnCommand`: its job is to feed the four `net_push_*` producer methods,
//! which take the routing `key` beside each event and a message as TEXT — so
//! this type mirrors those signatures one-for-one instead.

use serde::Deserialize;

/// One inbound network event handed to the runtime by its embedder.
#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum DeliveredEvent {
    /// The connection identified by `key` became usable as `conn`.
    Connected { key: String, conn: i32 },
    /// Text arrived on `conn`.
    Message { key: String, conn: i32, text: String },
    /// `conn` closed.
    Disconnected { key: String, conn: i32 },
    /// A transport-level error on `conn`; the connection is considered dead.
    Error {
        key: String,
        conn: i32,
        message: String,
    },
}

impl DeliveredEvent {
    /// The connection this event is about.
    pub fn conn(&self) -> i32 {
        match *self {
            DeliveredEvent::Connected { conn, .. }
            | DeliveredEvent::Message { conn, .. }
            | DeliveredEvent::Disconnected { conn, .. }
            | DeliveredEvent::Error { conn, .. } => conn,
        }
    }
}

/// Parse an embedder delivery (a JSON array of the events above). The error is
/// a teaching string the host logs — a malformed delivery is dropped, never
/// fatal.
///
/// A negative `conn` is rejected rather than pushed: the `net_push_*` methods
/// take an `i32` but the id is a `u64` [`ConnectionId`](super::ConnectionId)
/// on the other side of the prelude, so `-1` would reach the game as
/// `18446744073709551615`. [xreview]
pub fn parse_delivered_events(json: &str) -> Result<Vec<DeliveredEvent>, String> {
    let events: Vec<DeliveredEvent> = serde_json::from_str(json).map_err(|e| {
        format!(
            "{e} — expected a JSON array of \
             {{\"kind\":\"connected\"|\"message\"|\"disconnected\"|\"error\", \
             \"key\":…, \"conn\":…}}"
        )
    })?;
    if let Some(bad) = events.iter().find(|e| e.conn() < 0) {
        return Err(format!("negative connection id {}", bad.conn()));
    }
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_event_kind() {
        let json = r#"[
            { "kind": "connected", "key": "ws://x/", "conn": 1 },
            { "kind": "message", "key": "ws://x/", "conn": 1, "text": "hi" },
            { "kind": "disconnected", "key": "ws://x/", "conn": 1 },
            { "kind": "error", "key": "ws://x/", "conn": 2, "message": "boom" }
        ]"#;
        assert_eq!(
            parse_delivered_events(json).unwrap(),
            vec![
                DeliveredEvent::Connected {
                    key: "ws://x/".into(),
                    conn: 1
                },
                DeliveredEvent::Message {
                    key: "ws://x/".into(),
                    conn: 1,
                    text: "hi".into()
                },
                DeliveredEvent::Disconnected {
                    key: "ws://x/".into(),
                    conn: 1
                },
                DeliveredEvent::Error {
                    key: "ws://x/".into(),
                    conn: 2,
                    message: "boom".into()
                },
            ]
        );
    }

    #[test]
    fn an_empty_delivery_is_valid() {
        assert_eq!(parse_delivered_events("[]").unwrap(), vec![]);
    }

    #[test]
    fn malformed_deliveries_are_errors_not_panics() {
        // Not an array, an unknown kind, and a missing field.
        assert!(parse_delivered_events("{}").is_err());
        assert!(parse_delivered_events(r#"[{"kind":"nope","key":"k","conn":1}]"#).is_err());
        assert!(parse_delivered_events(r#"[{"kind":"connected","key":"k"}]"#).is_err());
    }

    #[test]
    fn a_negative_conn_is_rejected() {
        // It would reach the game as u64::MAX through the prelude's cast.
        let err = parse_delivered_events(r#"[{"kind":"connected","key":"k","conn":-1}]"#)
            .unwrap_err();
        assert!(err.contains("negative connection id"), "{err}");
    }
}
