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

    /// The listener's right axis (unit) — +pan is toward here.
    pub fn right(&self) -> [f32; 3] {
        normalize(cross(self.forward, self.up))
    }

    /// The shared spatialization for an emitter at `position`: a distance `gain`
    /// and a stereo `pan`, used identically by both backends so attenuation
    /// matches (each backend only applies these — it doesn't run its own distance
    /// model). See [`Spatialization`].
    pub fn spatialize(&self, position: [f32; 3]) -> Spatialization {
        let to = sub(position, self.position);
        let dist = length(to);

        // Rolloff: full gain within `min`, silent past `max`. The linear factor is
        // squared so the falloff is steep — a positioned voice fades to near-
        // nothing well before `max`, instead of lingering across the whole scene.
        let gain = if dist <= SPATIAL_MIN_DISTANCE {
            1.0
        } else if dist >= SPATIAL_MAX_DISTANCE {
            0.0
        } else {
            let t = 1.0 - (dist - SPATIAL_MIN_DISTANCE) / (SPATIAL_MAX_DISTANCE - SPATIAL_MIN_DISTANCE);
            t * t
        };

        // Pan = how far the emitter is to the listener's right (−1 left, +1 right).
        let pan = if dist > 1e-6 {
            dot(normalize(to), self.right()).clamp(-1.0, 1.0)
        } else {
            0.0
        };

        Spatialization { gain, pan }
    }
}

/// Distance attenuation (`gain`, linear 0..1) and stereo `pan` (−1 left .. +1
/// right) for a positioned voice, relative to the listener. Computed once in the
/// shared core so native (rodio) and wasm (Web Audio) attenuate identically.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spatialization {
    pub gain: f32,
    pub pan: f32,
}

/// Distances (world units) for the rolloff: at/below `MIN` a positioned voice is
/// at full gain; at/above `MAX` it's silent. The curve between is steep
/// (quadratic), so most of the audible range sits well inside `MAX`.
pub const SPATIAL_MIN_DISTANCE: f32 = 1.0;
pub const SPATIAL_MAX_DISTANCE: f32 = 10.0;

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

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn length(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

// --- Soundscape: the `Sub`-shaped half (continuous, reconciled voices) ----------
//
// Where a one-shot is fire-and-forget, a soundscape is a *standing* set of
// looping voices, declared each frame as a pure function of the model
// (`soundScape : model -> AudioScene`). Each voice has a `key` for cross-frame
// identity, so the runtime can keep a live voice playing across frames (panning
// and attenuating as the listener moves) instead of restarting it.
//
// The diff (`reconcile`) is pure and lives here; the live-voice registry lives in
// the *shell* (rodio sinks / Web Audio nodes), so it survives a hot reload for
// free — the dylib reloads, the voices keep playing, and the next frame's
// `soundScape` re-diffs against them.

/// One continuous voice in an [`AudioScene`]. Looping is implied (soundscape
/// voices are beds/emitters); one-shots use [`AudioCommand`] instead.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioSource {
    /// Cross-frame identity. A source with the same key across frames is the same
    /// live voice (kept playing); changing `position`/`gain` updates it in place.
    pub key: String,
    /// Asset path the host loads/decodes and loops.
    pub sound: String,
    /// Linear volume (1.0 = full).
    pub gain: f32,
    /// `Some` for a positioned (spatial) voice — panned/attenuated relative to the
    /// listener; `None` for a non-spatial bed (2D / music).
    #[serde(default)]
    pub position: Option<[f32; 3]>,
}

impl AudioSource {
    /// A non-spatial bed (wind, music) at full volume.
    pub fn ambient(key: String, sound: String) -> AudioSource {
        AudioSource {
            key,
            sound,
            gain: 1.0,
            position: None,
        }
    }

    /// A positioned emitter (a fountain you can walk around) at full volume.
    pub fn at(key: String, sound: String, x: f32, y: f32, z: f32) -> AudioSource {
        AudioSource {
            key,
            sound,
            gain: 1.0,
            position: Some([x, y, z]),
        }
    }

    pub fn with_gain(mut self, gain: f32) -> AudioSource {
        self.gain = gain;
        self
    }
}

/// The full set of continuous voices the game wants playing this frame — what
/// `soundScape model` returns. Pure, serializable data; crosses the dylib
/// boundary as JSON (like [`AudioCommand`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct AudioScene {
    pub sources: Vec<AudioSource>,
}

impl AudioScene {
    pub fn new(sources: Vec<AudioSource>) -> AudioScene {
        AudioScene { sources }
    }

    /// Build from the Fable array F# `soundScape` returns (mirrors
    /// `Scene3D::group`), so the F# `AudioScene.create` shim stays a thin call.
    pub fn from_sources(sources: NativeArray_::Array<AudioSource>) -> AudioScene {
        AudioScene {
            sources: sources.to_vec(),
        }
    }
}

/// Serialize a scene for the dylib→host hop (behind the `audio_scene_json`
/// runtime export). The game produces the scene; the host deserializes and
/// reconciles it.
pub fn scene_to_json(scene: &AudioScene) -> String {
    serde_json::to_string(scene).unwrap_or_else(|_| "{\"sources\":[]}".to_string())
}

/// One reconciliation action the shell applies to its live-voice registry.
#[derive(Debug, Clone, PartialEq)]
pub enum SceneUpdate {
    /// Start a new looping voice for `key`.
    Spawn(AudioSource),
    /// A voice with this key is already live; apply the new gain/position.
    Update(AudioSource),
    /// Stop and drop the live voice for `key` (no longer in the scene).
    Stop(String),
}

/// Pure diff between the live voices (keyed by their last-applied source) and the
/// `desired` scene. Produces stops (gone), spawns (new), and updates (changed) —
/// the shell maps each to a backend op. Deterministic order (stops, then spawns
/// /updates by key) so it's golden-testable. A key repeated in `desired` keeps
/// its first occurrence (later duplicates are ignored).
pub fn reconcile(live: &HashMap<String, AudioSource>, desired: &AudioScene) -> Vec<SceneUpdate> {
    let mut seen: Vec<&String> = Vec::new();
    let mut wanted: Vec<&AudioSource> = Vec::new();
    for src in &desired.sources {
        if seen.contains(&&src.key) {
            continue; // first occurrence of a key wins
        }
        seen.push(&src.key);
        wanted.push(src);
    }

    let mut updates = Vec::new();

    // Stops: live keys no longer desired. Sorted for determinism.
    let mut stops: Vec<&String> = live
        .keys()
        .filter(|k| !seen.contains(k))
        .collect();
    stops.sort();
    for key in stops {
        updates.push(SceneUpdate::Stop(key.clone()));
    }

    // Spawns / updates, in declaration order.
    for src in wanted {
        match live.get(&src.key) {
            None => updates.push(SceneUpdate::Spawn(src.clone())),
            Some(current) if current != src => updates.push(SceneUpdate::Update(src.clone())),
            Some(_) => {} // unchanged: leave the live voice alone
        }
    }

    updates
}

/// Serializes tests that touch the process-global [`OUTBOUND`] queue (here and
/// in `mle_prelude`), so they don't drain each other's commands under CI's
/// parallel scheduling. `unwrap_or_else(into_inner)` shrugs off a poisoned lock
/// from an unrelated panicking test.
#[cfg(test)]
pub(crate) static OUTBOUND_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_drain_round_trips_and_serializes_to_json() {
        let _guard = OUTBOUND_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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

    // --- spatialization (shared distance gain + pan) ---

    fn listener_at_origin() -> Listener {
        // At origin, looking down +Z, up +Y → right axis is +X.
        Listener::from_eye_target_up([0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0])
    }

    #[test]
    fn spatialize_gain_by_distance() {
        let l = listener_at_origin();
        // Inside min distance: full gain.
        assert_eq!(l.spatialize([0.0, 0.0, 0.5]).gain, 1.0);
        // Past max distance: silent.
        assert_eq!(l.spatialize([0.0, 0.0, 100.0]).gain, 0.0);
        // Halfway between min and max the linear factor is 0.5, squared → 0.25.
        let mid = (SPATIAL_MIN_DISTANCE + SPATIAL_MAX_DISTANCE) / 2.0;
        let g = l.spatialize([0.0, 0.0, mid]).gain;
        assert!((g - 0.25).abs() < 1e-5, "gain {g} should be ~0.25 at the midpoint");
        // Monotonically decreasing with distance.
        assert!(l.spatialize([0.0, 0.0, 3.0]).gain > l.spatialize([0.0, 0.0, 6.0]).gain);
    }

    #[test]
    fn spatialize_pans_by_side() {
        let l = listener_at_origin();
        // Looking down +Z with +Y up, the listener's right axis is -X (matching
        // the existing native `ears`), so +X is on the left and -X on the right.
        assert!(l.spatialize([5.0, 0.0, 0.0]).pan < -0.9); // +X → left
        assert!(l.spatialize([-5.0, 0.0, 0.0]).pan > 0.9); // -X → right
        assert!(l.spatialize([0.0, 0.0, 5.0]).pan.abs() < 0.1); // ahead → centered
    }

    // --- soundscape reconcile ---

    fn live_of(sources: &[AudioSource]) -> HashMap<String, AudioSource> {
        sources.iter().map(|s| (s.key.clone(), s.clone())).collect()
    }

    #[test]
    fn reconcile_spawns_new_and_stops_gone() {
        let wind = AudioSource::ambient("wind".into(), "wind.wav".into());
        let fountain = AudioSource::at("fountain".into(), "water.wav".into(), 5.0, 0.0, 0.0);

        // From nothing live, a two-source scene spawns both (in declaration order).
        let live = HashMap::new();
        let desired = AudioScene::new(vec![wind.clone(), fountain.clone()]);
        assert_eq!(
            reconcile(&live, &desired),
            vec![
                SceneUpdate::Spawn(wind.clone()),
                SceneUpdate::Spawn(fountain.clone()),
            ]
        );

        // With both live, dropping the fountain stops just it.
        let live = live_of(&[wind.clone(), fountain]);
        let desired = AudioScene::new(vec![wind]);
        assert_eq!(
            reconcile(&live, &desired),
            vec![SceneUpdate::Stop("fountain".into())]
        );
    }

    #[test]
    fn reconcile_updates_changed_and_ignores_unchanged() {
        let fountain = AudioSource::at("fountain".into(), "water.wav".into(), 5.0, 0.0, 0.0);
        let live = live_of(&[fountain.clone()]);

        // Same source again: no action.
        let desired = AudioScene::new(vec![fountain.clone()]);
        assert!(reconcile(&live, &desired).is_empty());

        // Moved + quieter: a single Update carrying the new source.
        let moved = fountain.with_gain(0.4);
        let moved = AudioSource {
            position: Some([8.0, 0.0, 1.0]),
            ..moved
        };
        let desired = AudioScene::new(vec![moved.clone()]);
        assert_eq!(reconcile(&live, &desired), vec![SceneUpdate::Update(moved)]);
    }

    #[test]
    fn reconcile_dedupes_desired_by_key_and_json_round_trips() {
        // A key repeated in the scene keeps its first occurrence.
        let first = AudioSource::ambient("bed".into(), "a.wav".into());
        let dup = AudioSource::ambient("bed".into(), "b.wav".into());
        let desired = AudioScene::new(vec![first.clone(), dup]);
        assert_eq!(
            reconcile(&HashMap::new(), &desired),
            vec![SceneUpdate::Spawn(first)]
        );

        // scene_to_json -> serde round-trips (the dylog->host hop).
        let json = scene_to_json(&desired);
        let back: AudioScene = serde_json::from_str(&json).unwrap();
        assert_eq!(back, desired);
    }
}
