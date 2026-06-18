//! Headless end-to-end test of the WebSocket (persistent-connection) path
//! (docs/multiplayer.md, Phase 2), driving the `wsdemo` sample dylib directly --
//! no GL, no window, no socket.
//!
//! It steps the game through its exported `tick`, asserting the connection
//! lifecycle:
//!   1. declaring `Sub.connect` makes the executor reconcile and queue a `Connect`
//!      command (read back via `net_drain_conn_commands_json`);
//!   2. injecting a `Connected` event routes to the decoder -> the game stores the
//!      `ConnectionId` and replies with `Effect.send`, which queues a `Send`;
//!   3. injecting a `Message` event delivers it to the model.
//!
//! This is exactly how the in-process netsim (Phase 3) will drive connected
//! clients: inject events, step ticks, read state -- no real I/O.
//!
//! Ignored by default: needs the sample dylib built first. Run with:
//!
//! ```sh
//! ./target/debug/functor -d examples/wsdemo build native
//! cargo test -p functor-runtime-desktop --test net_ws -- --ignored --nocapture
//! ```

use std::path::PathBuf;

use fable_library_rust::String_::{fromString, LrcStr};
use functor_runtime_common::FrameTime;
use libloading::{Library, Symbol};

const ENDPOINT: &str = "ws://127.0.0.1:9001/echo";

fn wsdemo_dylib() -> PathBuf {
    let dylib = format!(
        "{}game_native{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("examples/wsdemo/build-native/target/debug")
        .join(dylib)
}

#[test]
#[ignore = "needs examples/wsdemo built: functor -d examples/wsdemo build native"]
fn websocket_connect_send_receive() {
    let path = wsdemo_dylib();
    assert!(
        path.exists(),
        "missing {} — build it first (see this test's doc comment)",
        path.display()
    );

    unsafe {
        let lib = Library::new(&path).expect("load wsdemo dylib");

        let init: Symbol<fn()> = lib.get(b"init").unwrap();
        let tick: Symbol<fn(FrameTime)> = lib.get(b"tick").unwrap();
        let drain_conn: Symbol<fn() -> LrcStr> = lib.get(b"net_drain_conn_commands_json").unwrap();
        let push_connected: Symbol<fn(LrcStr, i32)> = lib.get(b"net_push_connected").unwrap();
        let push_message: Symbol<fn(LrcStr, i32, LrcStr)> =
            lib.get(b"net_push_conn_message").unwrap();
        let state_debug: Symbol<fn() -> LrcStr> = lib.get(b"emit_state_debug").unwrap();

        init();

        // Tick 1: the declared Sub.connect reconciles -> a Connect command queued.
        tick(FrameTime { tts: 0.0, dts: 0.0 });
        let commands = drain_conn().to_string();
        assert!(
            commands.contains("Connect") && commands.contains(ENDPOINT),
            "expected a Connect command for the endpoint, got: {commands}"
        );
        // Reconciled — not re-queued next frame.
        assert_eq!(drain_conn().to_string(), "[]");

        // Inject Connected(conn=1) for this endpoint's key.
        push_connected(fromString(ENDPOINT.to_string()), 1);

        // Tick 2: routed to the decoder -> model `connected` + Effect.send "hello".
        tick(FrameTime { tts: 0.016, dts: 0.016 });
        let connected = state_debug().to_string();
        assert!(
            connected.contains("connected"),
            "model should be connected, got: {connected}"
        );
        let after_send = drain_conn().to_string();
        // payload is JSON bytes: "hello" = [104,101,108,108,111].
        assert!(
            after_send.contains("Send") && after_send.contains("104,101,108,108,111"),
            "expected the greeting Send command, got: {after_send}"
        );

        // Inject a message on the connection.
        push_message(fromString(ENDPOINT.to_string()), 1, fromString("echo-hello".to_string()));

        // Tick 3: delivered to the model.
        tick(FrameTime { tts: 0.032, dts: 0.016 });
        let got = state_debug().to_string();
        assert!(
            got.contains("got-message") && got.contains("echo-hello"),
            "model should reflect the received message, got: {got}"
        );
    }
}
