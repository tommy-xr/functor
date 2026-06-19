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

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use fable_library_rust::NativeArray_;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

/// A request to the host's audio device. Plain, serializable data — it crosses
/// the dylib boundary as JSON, like [`crate::net::NetCommand`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AudioCommand {
    /// Play a sound once and let it finish. `sound` is an asset path the host
    /// loads/decodes; `gain` is a linear volume (1.0 = full). `token` is `Some`
    /// only when the game wants a completion message (`Audio.playThen`): the host
    /// then reports back via `audio_push_finished(token)` when it ends. `position`
    /// is `Some` for a spatialized one-shot (world-space, panned and attenuated
    /// relative to the camera/listener — `Audio.playAt`); `None` plays non-spatial.
    PlayOneShot {
        token: Option<u64>,
        sound: String,
        gain: f32,
        #[serde(default)]
        position: Option<[f32; 3]>,
    },
}

impl AudioCommand {
    /// A fire-and-forget, non-spatial one-shot at full volume — behind `Audio.play`.
    pub fn play_one_shot(sound: String) -> AudioCommand {
        AudioCommand::PlayOneShot {
            token: None,
            sound,
            gain: 1.0,
            position: None,
        }
    }

    /// A one-shot whose completion is reported back under `token` — behind
    /// `Audio.playThen`.
    pub fn play_one_shot_token(token: u64, sound: String) -> AudioCommand {
        AudioCommand::PlayOneShot {
            token: Some(token),
            sound,
            gain: 1.0,
            position: None,
        }
    }

    /// A spatialized one-shot at `position` (world space) — behind `Audio.playAt`.
    pub fn play_one_shot_at(sound: String, x: f32, y: f32, z: f32) -> AudioCommand {
        AudioCommand::PlayOneShot {
            token: None,
            sound,
            gain: 1.0,
            position: Some([x, y, z]),
        }
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

// --- Completion: the `Audio.playThen` half (mirrors the `net` registry/inbox) ---
//
// A `playThen` one-shot carries a completion message; we hold it here keyed by
// the command's token across the play→finish gap, and the host reports the end
// via `audio_push_finished(token)` into the FINISHED queue. The executor drains
// that queue each tick, matches each token back to its message, and feeds it
// through `update`. Like `net`, these live on the dylib side and are dropped on
// hot reload (an in-flight sound then just loses its completion message).

thread_local! {
    // Box<dyn Any> erases the Msg type so one table serves any game; the executor
    // (which knows Msg) downcasts on the way out.
    static COMPLETIONS: RefCell<HashMap<u64, Box<dyn Any>>> = RefCell::new(HashMap::new());
    static NEXT_TOKEN: Cell<u64> = Cell::new(1);
    // Tokens of sounds the host has reported finished, awaiting the executor.
    static FINISHED: RefCell<VecDeque<u64>> = RefCell::new(VecDeque::new());
}

/// A fresh correlation token for a `playThen` one-shot.
pub fn next_token() -> u64 {
    NEXT_TOKEN.with(|c| {
        let token = c.get();
        c.set(token + 1);
        token
    })
}

/// Hold the completion message for `token` until the sound finishes.
pub fn register_completion<M: 'static>(token: u64, message: M) {
    COMPLETIONS.with(|c| {
        c.borrow_mut().insert(token, Box::new(message));
    });
}

/// Take the completion message for `token`. `None` for an unknown token or one
/// whose message was dropped by a hot reload while the sound was playing.
pub fn take_completion<M: 'static>(token: u64) -> Option<M> {
    let boxed = COMPLETIONS.with(|c| c.borrow_mut().remove(&token))?;
    boxed.downcast::<M>().ok().map(|m| *m)
}

/// Host: report that the sound for `token` has finished (via the runtime export).
pub fn push_finished(token: u64) {
    FINISHED.with(|f| f.borrow_mut().push_back(token));
}

/// The executor drains finished tokens each tick to deliver completion messages.
pub fn drain_finished() -> Vec<u64> {
    FINISHED.with(|f| f.borrow_mut().drain(..).collect())
}

/// Executor (F#-facing): the finished tokens as a Fable array, so they cross to
/// F# as `array` (mirrors `net::drain_http_results`).
pub fn drain_finished_array() -> NativeArray_::Array<u64> {
    NativeArray_::array_from(drain_finished())
}

/// Where the player hears from — the render camera. The host sets it from the
/// frame's camera each frame; spatial backends pan/attenuate emitters relative
/// to it (Y-up, right-handed).
#[derive(Debug, Clone, Copy)]
pub struct Listener {
    pub position: [f32; 3],
    pub forward: [f32; 3],
    pub up: [f32; 3],
}

impl Listener {
    /// Derive from a camera's `eye` / `target` / `up`.
    pub fn from_eye_target_up(eye: [f32; 3], target: [f32; 3], up: [f32; 3]) -> Listener {
        let forward = normalize(sub(target, eye));
        Listener {
            position: eye,
            forward,
            up: normalize(up),
        }
    }

    /// Left/right ear world positions `half_ear` apart along the listener's right
    /// axis — what rodio's `SpatialSink` wants (it derives L/R balance + distance
    /// attenuation from the emitter relative to these).
    pub fn ears(&self, half_ear: f32) -> ([f32; 3], [f32; 3]) {
        let right = normalize(cross(self.forward, self.up));
        let l = [
            self.position[0] - right[0] * half_ear,
            self.position[1] - right[1] * half_ear,
            self.position[2] - right[2] * half_ear,
        ];
        let r = [
            self.position[0] + right[0] * half_ear,
            self.position[1] + right[1] * half_ear,
            self.position[2] + right[2] * half_ear,
        ];
        (l, r)
    }
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len > 1e-6 {
        [v[0] / len, v[1] / len, v[2] / len]
    } else {
        [0.0, 0.0, 1.0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // One test, not two: `push_command`/`drain_commands` share the process-global
    // OUTBOUND queue, so two tests touching it run in parallel and can drain each
    // other's command (a flaky failure that showed up under CI's scheduling).
    // Keeping it to a single test makes the queue access sequential. (The
    // completion tests below use thread-local state, so they stand alone.)
    #[test]
    fn push_drain_round_trips_and_serializes_to_json() {
        let _ = drain_commands(); // clear anything a prior run left

        push_command(AudioCommand::play_one_shot("gunshot.wav".to_string()));
        let drained = drain_commands();
        assert_eq!(
            drained,
            vec![AudioCommand::PlayOneShot {
                token: None,
                sound: "gunshot.wav".to_string(),
                gain: 1.0,
                position: None
            }]
        );
        // Draining again yields nothing.
        assert!(drain_commands().is_empty());

        // drain_commands_json serializes the same queue as a JSON array.
        push_command(AudioCommand::play_one_shot("x.wav".to_string()));
        let json = drain_commands_json();
        assert!(json.contains("PlayOneShot"));
        assert!(json.contains("x.wav"));
    }

    #[test]
    fn completion_round_trips_by_token() {
        let token = next_token();
        register_completion(token, 42i32);
        push_finished(token);
        assert_eq!(drain_finished(), vec![token]);
        assert_eq!(take_completion::<i32>(token), Some(42));
        // Taken once: gone the second time.
        assert_eq!(take_completion::<i32>(token), None);
    }

    #[test]
    fn take_completion_unknown_token_is_none() {
        assert_eq!(take_completion::<i32>(999_999), None);
    }

}
