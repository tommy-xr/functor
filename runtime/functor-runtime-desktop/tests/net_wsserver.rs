//! Headless end-to-end test of the WebSocket *server* path (docs/multiplayer.md,
//! Phase 2), driving the `wsserverdemo` sample dylib directly -- no GL, no window,
//! no socket.
//!
//! It asserts the server lifecycle:
//!   1. declaring `Sub.listen` makes the executor reconcile and queue a `Listen`
//!      command;
//!   2. injecting a per-client `Connected` event routes to the decoder -> the game
//!      greets that client with `Effect.send` (a `Send` for its id);
//!   3. injecting a `Message` from the client is echoed back to it.
//!
//! All per-client events are keyed by the listener's bind address, the routing
//! key the executor expects.
//!
//! Ignored by default. Run with:
//!
//! ```sh
//! ./target/debug/functor -d examples/wsserverdemo build native
//! cargo test -p functor-runtime-desktop --test net_wsserver -- --ignored --nocapture
//! ```

use std::path::PathBuf;

use fable_library_rust::String_::{fromString, LrcStr};
use functor_runtime_common::FrameTime;
use libloading::{Library, Symbol};

const BIND: &str = "127.0.0.1:9001";

fn wsserverdemo_dylib() -> PathBuf {
    let dylib = format!(
        "{}game_native{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("examples/wsserverdemo/build-native/target/debug")
        .join(dylib)
}

#[test]
#[ignore = "needs examples/wsserverdemo built: functor -d examples/wsserverdemo build native"]
fn websocket_server_accepts_and_echoes() {
    let path = wsserverdemo_dylib();
    assert!(
        path.exists(),
        "missing {} — build it first (see this test's doc comment)",
        path.display()
    );

    unsafe {
        let lib = Library::new(&path).expect("load wsserverdemo dylib");

        let init: Symbol<fn()> = lib.get(b"init").unwrap();
        let tick: Symbol<fn(FrameTime)> = lib.get(b"tick").unwrap();
        let drain_conn: Symbol<fn() -> LrcStr> = lib.get(b"net_drain_conn_commands_json").unwrap();
        let push_connected: Symbol<fn(LrcStr, i32)> = lib.get(b"net_push_connected").unwrap();
        let push_message: Symbol<fn(LrcStr, i32, LrcStr)> =
            lib.get(b"net_push_conn_message").unwrap();
        let state_debug: Symbol<fn() -> LrcStr> = lib.get(b"emit_state_debug").unwrap();

        init();

        // Tick 1: the declared Sub.listen reconciles -> a Listen command queued.
        tick(FrameTime { tts: 0.0, dts: 0.0 });
        let commands = drain_conn().to_string();
        assert!(
            commands.contains("Listen") && commands.contains(BIND),
            "expected a Listen command for the bind address, got: {commands}"
        );

        // Inject a client connecting (keyed by the listener's bind address).
        push_connected(fromString(BIND.to_string()), 1);

        // Tick 2: routed -> the game greets the client (Send "welcome" to id 1).
        tick(FrameTime { tts: 0.016, dts: 0.016 });
        let connected = state_debug().to_string();
        assert!(
            connected.contains("client-connected"),
            "model should show a connected client, got: {connected}"
        );
        let greeting = drain_conn().to_string();
        // "welcome" = [119,101,108,99,111,109,101]
        assert!(
            greeting.contains("Send") && greeting.contains("119,101,108,99,111,109,101"),
            "expected a welcome Send to the new client, got: {greeting}"
        );

        // Inject a message from that client; the server echoes it back.
        push_message(fromString(BIND.to_string()), 1, fromString("ahoy".to_string()));
        tick(FrameTime { tts: 0.032, dts: 0.016 });
        let echoed = state_debug().to_string();
        assert!(
            echoed.contains("echoed") && echoed.contains("ahoy"),
            "model should reflect the echoed message, got: {echoed}"
        );
        let echo_back = drain_conn().to_string();
        // "ahoy" = [97,104,111,121]
        assert!(
            echo_back.contains("Send") && echo_back.contains("97,104,111,121"),
            "expected the echo Send back to the client, got: {echo_back}"
        );
    }
}
