//! Audio commands the game queues for the host to perform.
//!
//! Mirrors the `net` outbound-command pattern: game code returns an
//! `Effect::PlayAudio`, which pushes an [`AudioCommand`] onto a process-global
//! queue. The host drains it each frame (via the `audio_drain_commands_json`
//! runtime export) and plays it on its own audio device — rodio on native, Web
//! Audio on wasm. The queue lives in the game dylib; the host reaches it only
//! through the export, so it survives hot reload (the host's audio device, and
//! any sounds in flight, live in the host and aren't disturbed by a reload).
//!
//! One-shots here are fire-and-forget. A completion message (the audio twin of
//! the HTTP `tagger`) is a later addition that mirrors the `net` async inbox.

use std::collections::VecDeque;
use std::sync::Mutex;

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

/// A request to the host's audio device. Plain, serializable data — it crosses
/// the dylib boundary as JSON, like [`crate::net::NetCommand`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AudioCommand {
    /// Play a sound once and let it finish (fire-and-forget). `sound` is an
    /// asset path the host loads/decodes; `gain` is a linear volume (1.0 = full).
    PlayOneShot { sound: String, gain: f32 },
}

impl AudioCommand {
    /// A one-shot at full volume — the common case behind `Audio.play`.
    pub fn play_one_shot(sound: String) -> AudioCommand {
        AudioCommand::PlayOneShot { sound, gain: 1.0 }
    }
}

static OUTBOUND: Lazy<Mutex<VecDeque<AudioCommand>>> = Lazy::new(|| Mutex::new(VecDeque::new()));

/// Queue a command for the host to perform this frame (called by `Effect::run`).
pub fn push_command(cmd: AudioCommand) {
    OUTBOUND.lock().unwrap().push_back(cmd);
}

/// Take everything queued since the last drain (the host calls this each frame).
pub fn drain_commands() -> Vec<AudioCommand> {
    OUTBOUND.lock().unwrap().drain(..).collect()
}

/// The host drains over the dylib boundary as a JSON array (see the
/// `audio_drain_commands_json` runtime export).
pub fn drain_commands_json() -> String {
    serde_json::to_string(&drain_commands()).unwrap_or_else(|_| "[]".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_then_drain_round_trips() {
        let _ = drain_commands(); // clear anything a prior test left
        push_command(AudioCommand::play_one_shot("gunshot.wav".to_string()));
        let drained = drain_commands();
        assert_eq!(
            drained,
            vec![AudioCommand::PlayOneShot {
                sound: "gunshot.wav".to_string(),
                gain: 1.0
            }]
        );
        // Draining again yields nothing.
        assert!(drain_commands().is_empty());
    }

    #[test]
    fn drain_json_is_an_array() {
        let _ = drain_commands();
        push_command(AudioCommand::play_one_shot("x.wav".to_string()));
        let json = drain_commands_json();
        assert!(json.contains("PlayOneShot"));
        assert!(json.contains("x.wav"));
    }
}
