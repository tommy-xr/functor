//! The `Effect.preload` command queue (Track B.5 of the asset revamp) —
//! the asset analogue of [`crate::audio`]'s one-shot queue.
//!
//! Performing `Effect.preload(asset)` queues a [`PreloadCommand`]; the SHELL
//! drains the queue each frame (`SceneContext::drive_preloads`) into
//! `AssetCache::load_asset_with_pipeline` calls, warming exactly the cache
//! entry `draw` will later hit. The shell also keeps polling each in-flight
//! preload handle until it settles — asset futures advance only when polled,
//! and an abandoned preload would otherwise sit in `Sub.assets`' `total`
//! forever (the `resolve_while_pending` liveness rule, applied to preloads).
//!
//! `Effect.preloadThen` correlates its completion MESSAGE with the load via a
//! token, exactly like `Effect.playThen`: this module mints tokens, the
//! producer holds the pending message (`functor_lang_prelude::
//! register_preload_completion`), and the shell reports settlement through
//! `GameProducer::preload_push_settled`. Settled means loaded OR failed —
//! the message always arrives; `Sub.assets`' `failed` list carries which.

use std::cell::Cell;
use std::collections::VecDeque;
use std::sync::Mutex;

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

/// Which pipeline a preload warms. Sounds never appear here: they decode at
/// play time, outside the asset cache (the prelude rejects them with a
/// teaching error).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PreloadKind {
    Model,
    Texture,
}

/// One queued `Effect.preload` / `preloadThen`: plain data across the
/// logic↔runtime boundary (the [`crate::audio::AudioCommand`] shape).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreloadCommand {
    pub kind: PreloadKind,
    pub locator: String,
    /// `Some` for `preloadThen`: the completion token whose message the
    /// producer holds until the shell reports the load settled.
    pub token: Option<u64>,
}

static OUTBOUND: Lazy<Mutex<VecDeque<PreloadCommand>>> =
    Lazy::new(|| Mutex::new(VecDeque::new()));

/// Queue a command for the shell to perform (called by the effect drain).
pub fn push_command(cmd: PreloadCommand) {
    OUTBOUND.lock().unwrap().push_back(cmd);
}

/// Take everything queued since the last drain (the shell calls this each
/// frame via `SceneContext::drive_preloads`).
pub fn drain_commands() -> Vec<PreloadCommand> {
    OUTBOUND.lock().unwrap().drain(..).collect()
}

/// The drain as a JSON array — the `GameProducer::preload_drain_commands`
/// wire form.
pub fn drain_commands_json() -> String {
    serde_json::to_string(&drain_commands()).unwrap_or_else(|_| "[]".to_string())
}

thread_local! {
    static NEXT_TOKEN: Cell<u64> = const { Cell::new(1) };
}

/// A fresh correlation token for a `preloadThen`.
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
    fn commands_round_trip_the_wire_and_drain_once() {
        // The queue is process-global and other tests drain it with exact
        // assertions — serialize with them (the audio queue's rule).
        let _guard = crate::audio::OUTBOUND_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _ = drain_commands(); // isolate from other tests' leftovers
        push_command(PreloadCommand {
            kind: PreloadKind::Model,
            locator: "boss.glb".to_string(),
            token: None,
        });
        push_command(PreloadCommand {
            kind: PreloadKind::Texture,
            locator: "https://cdn/wood.png".to_string(),
            token: Some(7),
        });
        let json = drain_commands_json();
        assert_eq!(
            json,
            r#"[{"kind":"Model","locator":"boss.glb","token":null},{"kind":"Texture","locator":"https://cdn/wood.png","token":7}]"#
        );
        let back: Vec<PreloadCommand> = serde_json::from_str(&json).unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!(back[1].token, Some(7));
        // Drained means drained.
        assert_eq!(drain_commands_json(), "[]");
    }
}
