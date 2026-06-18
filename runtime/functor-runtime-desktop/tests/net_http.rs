//! Headless end-to-end test of the Phase 1 HTTP path (docs/multiplayer.md),
//! driving the `netdemo` sample dylib directly — no GL, no window, no network.
//!
//! It loads the game dylib and steps it through its exported `tick`, asserting the
//! full round trip:
//!   1. the startup `Fetch` runs `Effect.httpGet`, which queues a `NetCommand`
//!      (read back via `net_drain_commands_json`), and moves the model to Waiting;
//!   2. injecting a response with `net_push_http_response` lands it in the async
//!      inbox; the next `tick` drains it, the `Sub.httpResponses` decoder turns it
//!      into a message, and `update` moves the model to Done with the body.
//!
//! This is exactly how the in-process netsim (Phase 3) will drive clients: inject
//! inbound bytes, step ticks, read state — the host's real HTTP dispatch is not
//! involved, so the game logic is verified deterministically in isolation.
//!
//! Ignored by default: it needs the sample dylib built first. Run with:
//!
//! ```sh
//! ./target/debug/functor -d examples/netdemo build native
//! cargo test -p functor-runtime-desktop --test net_http -- --ignored --nocapture
//! ```

use std::path::PathBuf;

use fable_library_rust::String_::{fromString, LrcStr};
use functor_runtime_common::FrameTime;
use libloading::{Library, Symbol};

fn netdemo_dylib() -> PathBuf {
    let dylib = format!(
        "{}game_native{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("examples/netdemo/build-native/target/debug")
        .join(dylib)
}

#[test]
#[ignore = "needs examples/netdemo built: functor -d examples/netdemo build native"]
fn http_request_response_round_trip() {
    let path = netdemo_dylib();
    assert!(
        path.exists(),
        "missing {} — build it first (see this test's doc comment)",
        path.display()
    );

    unsafe {
        let lib = Library::new(&path).expect("load netdemo dylib");

        let init: Symbol<fn()> = lib.get(b"init").unwrap();
        let tick: Symbol<fn(FrameTime)> = lib.get(b"tick").unwrap();
        let drain_commands: Symbol<fn() -> LrcStr> = lib.get(b"net_drain_commands_json").unwrap();
        let push_response: Symbol<fn(i32, i32, LrcStr)> =
            lib.get(b"net_push_http_response").unwrap();
        let state_debug: Symbol<fn() -> LrcStr> = lib.get(b"emit_state_debug").unwrap();

        // Build the game (sets the global runner, seeds the `Fetch` startup effect).
        init();

        // Tick 1: the startup Fetch drains -> Waiting + an httpGet command queued.
        tick(FrameTime { tts: 0.0, dts: 0.0 });

        let commands = drain_commands().to_string();
        assert!(
            commands.contains("HttpRequest") && commands.contains("127.0.0.1:9000/hello"),
            "expected the queued GET in the outbound commands, got: {commands}"
        );

        let loading = state_debug().to_string();
        assert!(
            loading.contains("Loading"),
            "model should be Loading after firing the request, got: {loading}"
        );

        // The command should have been consumed by the drain above.
        assert_eq!(drain_commands().to_string(), "[]");

        // Inject the response into the async inbox. The request was the first
        // httpGet, so its auto-assigned token is 1.
        push_response(1, 200, fromString("pong".to_string()));

        // Tick 2: the inbox drains -> the request's tagger -> Done(200, "pong").
        tick(FrameTime { tts: 0.016, dts: 0.016 });

        let done = state_debug().to_string();
        assert!(
            done.contains("Done") && done.contains("200") && done.contains("pong"),
            "model should reflect the decoded response, got: {done}"
        );
    }
}
