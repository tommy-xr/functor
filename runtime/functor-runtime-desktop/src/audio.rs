//! Native audio output via rodio.
//!
//! The host owns the output device (a rodio `OutputStream`) for the lifetime of
//! the runner, so it survives hot reload — the game dylib only *queues*
//! `AudioCommand`s, which `main.rs` drains each frame and hands here. Sounds
//! play on rodio's own thread, so a play call never stalls the frame loop.
//!
//! Fire-and-forget one-shots are detached and free themselves when done; a
//! `playThen` one-shot (with a token) plays on a thread that waits for it to
//! finish and reports the token over `completion_tx`, which `main.rs` forwards
//! back to the game. Positioned sounds (`Audio.playAt`, soundscape voices) are
//! spread to stereo with the shared `spatialize` gain + an equal-power pan via a
//! `Panned` source — the same model the wasm backend uses (we don't use rodio's
//! `SpatialSink`, whose pan law is too gentle to localize).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::BufReader;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

use functor_runtime_common::audio::{AudioCommand, AudioScene, AudioSource, Listener, SceneUpdate};
use rodio::source::Source;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink};

/// Per-channel `[left, right]` gains, stored as f32 bits so the frame loop can
/// update a live voice's pan/volume while it plays on rodio's thread.
type PanGains = Arc<[AtomicU32; 2]>;

fn store_gains(gains: &PanGains, left: f32, right: f32) {
    gains[0].store(left.to_bits(), Ordering::Relaxed);
    gains[1].store(right.to_bits(), Ordering::Relaxed);
}

/// Equal-power stereo pan (matching the wasm `StereoPannerNode`): `pan` −1 → all
/// left, +1 → all right, 0 → −3dB both. Scaled by the distance `gain`.
fn equal_power(pan: f32, gain: f32) -> (f32, f32) {
    let theta = (pan.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
    (gain * theta.cos(), gain * theta.sin())
}

/// A mono rodio source spread to stereo with live per-channel gains. Assumes a
/// mono input (the demo/audio assets are mono); each input sample is emitted to
/// both channels, scaled by that channel's current gain.
struct Panned<I> {
    input: I,
    gains: PanGains,
    // Next output channel: 0 = left, 1 = right.
    channel: usize,
    sample: f32,
}

impl<I> Panned<I>
where
    I: Source<Item = f32>,
{
    fn new(input: I, gains: PanGains) -> Panned<I> {
        Panned {
            input,
            gains,
            channel: 0,
            sample: 0.0,
        }
    }
}

impl<I> Iterator for Panned<I>
where
    I: Source<Item = f32>,
{
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.channel == 0 {
            self.sample = self.input.next()?;
            self.channel = 1;
            Some(self.sample * f32::from_bits(self.gains[0].load(Ordering::Relaxed)))
        } else {
            self.channel = 0;
            Some(self.sample * f32::from_bits(self.gains[1].load(Ordering::Relaxed)))
        }
    }
}

impl<I> Source for Panned<I>
where
    I: Source<Item = f32>,
{
    fn current_frame_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        2
    }
    fn sample_rate(&self) -> u32 {
        self.input.sample_rate()
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

/// A live looping voice in the soundscape: the rodio sink plus the last-applied
/// source (so the reconciler can diff changes), and — for a positioned voice —
/// the pan gains the frame loop updates as the listener moves.
struct LoopVoice {
    source: AudioSource,
    sink: Sink,
    pan_gains: Option<PanGains>,
}

/// Holds the audio device.
pub struct AudioPlayer {
    // The stream must stay alive for anything to play; held though otherwise unused.
    _stream: OutputStream,
    handle: OutputStreamHandle,
    completion_tx: Sender<u64>,
    listener: Listener,
    // Live soundscape voices, keyed for cross-frame reconciliation. Lives in the
    // host (not the dylib), so it survives a hot reload — the voices keep playing
    // and the next frame's `soundScape` re-diffs against them.
    voices: HashMap<String, LoopVoice>,
    // Asset paths whose open/decode already failed and were logged. A missing
    // soundscape voice never enters `voices`, so `reconcile` re-spawns it every
    // frame; without this the decode error would print every frame. Warn once per
    // path instead (`RefCell` because `decode` takes `&self`). Not reset on hot
    // reload — a path that's still missing stays quiet until the file appears.
    warned_paths: RefCell<HashSet<String>>,
}

impl AudioPlayer {
    /// Open the default output device. `None` when there is no device (headless
    /// / CI) — the runner then simply drops audio commands. `completion_tx`
    /// carries finished `playThen` tokens back to the main loop.
    pub fn new(completion_tx: Sender<u64>) -> Option<AudioPlayer> {
        match OutputStream::try_default() {
            Ok((stream, handle)) => Some(AudioPlayer {
                _stream: stream,
                handle,
                completion_tx,
                listener: Listener {
                    position: [0.0, 0.0, 0.0],
                    forward: [0.0, 0.0, 1.0],
                    up: [0.0, 1.0, 0.0],
                },
                voices: HashMap::new(),
                warned_paths: RefCell::new(HashSet::new()),
            }),
            Err(e) => {
                eprintln!("[audio] no output device, audio disabled: {e}");
                None
            }
        }
    }

    /// Update the listener from the render camera (called each frame).
    pub fn set_listener(&mut self, eye: [f32; 3], target: [f32; 3], up: [f32; 3]) {
        self.listener = Listener::from_eye_target_up(eye, target, up);
    }

    /// Reconcile the desired soundscape against the live voices: spawn new ones,
    /// stop gone ones, and update changed gain/position in place (no restart).
    pub fn reconcile_scene(&mut self, scene: &AudioScene) {
        let live: HashMap<String, AudioSource> = self
            .voices
            .iter()
            .map(|(k, v)| (k.clone(), v.source.clone()))
            .collect();
        for update in functor_runtime_common::audio::reconcile(&live, scene) {
            match update {
                SceneUpdate::Spawn(src) => self.spawn_voice(src),
                SceneUpdate::Update(src) => self.update_voice(src),
                SceneUpdate::Stop(key) => {
                    self.voices.remove(&key); // dropping the sink stops playback
                }
            }
        }
    }

    /// Re-spatialize the live positioned voices for the current listener (called
    /// each frame after `set_listener`), so a looping emitter pans/attenuates as
    /// the camera moves around it — even on frames where its source didn't change.
    pub fn respatialize_voices(&self) {
        for voice in self.voices.values() {
            if let (Some(gains), Some(pos)) = (&voice.pan_gains, voice.source.position) {
                let s = self.listener.spatialize(pos);
                let (l, r) = equal_power(s.pan, voice.source.gain * s.gain);
                store_gains(gains, l, r);
            }
        }
    }

    fn spawn_voice(&mut self, src: AudioSource) {
        let Some(source) = self.decode(&src.sound) else {
            return;
        };
        // Buffered so the decoded samples can be cloned and looped forever.
        let looped = source.buffered().repeat_infinite();
        let sink = match Sink::try_new(&self.handle) {
            Ok(sink) => sink,
            Err(e) => {
                eprintln!("[audio] sink: {e}");
                return;
            }
        };
        let pan_gains = match src.position {
            None => {
                sink.set_volume(src.gain);
                sink.append(looped);
                None
            }
            Some(pos) => {
                let s = self.listener.spatialize(pos);
                let (l, r) = equal_power(s.pan, src.gain * s.gain);
                let gains: PanGains = Arc::new([AtomicU32::new(0), AtomicU32::new(0)]);
                store_gains(&gains, l, r);
                sink.append(Panned::new(looped.convert_samples::<f32>(), gains.clone()));
                Some(gains)
            }
        };
        self.voices.insert(
            src.key.clone(),
            LoopVoice {
                source: src,
                sink,
                pan_gains,
            },
        );
    }

    fn update_voice(&mut self, src: AudioSource) {
        // A flip in spatial-ness (None <-> Some) is a different graph, so respawn.
        let spatial_changed = self
            .voices
            .get(&src.key)
            .map(|v| v.source.position.is_some() != src.position.is_some())
            .unwrap_or(true);
        if spatial_changed {
            self.voices.remove(&src.key);
            self.spawn_voice(src);
            return;
        }
        if let Some(voice) = self.voices.get_mut(&src.key) {
            // Non-spatial: apply the gain directly. Spatial voices are re-applied
            // each frame by `respatialize_voices` from the stored source below.
            if voice.pan_gains.is_none() {
                voice.sink.set_volume(src.gain);
            }
            voice.source = src;
        }
    }

    /// Perform one queued command.
    pub fn handle(&self, cmd: AudioCommand) {
        let AudioCommand::PlayOneShot {
            token,
            sound,
            gain,
            position,
        } = cmd;
        let Some(source) = self.decode(&sound) else {
            return;
        };
        let sink = match Sink::try_new(&self.handle) {
            Ok(sink) => sink,
            Err(e) => {
                eprintln!("[audio] sink: {e}");
                return;
            }
        };
        match position {
            None => {
                sink.set_volume(gain);
                sink.append(source);
            }
            Some(pos) => {
                let s = self.listener.spatialize(pos);
                let (l, r) = equal_power(s.pan, gain * s.gain);
                let gains: PanGains = Arc::new([AtomicU32::new(0), AtomicU32::new(0)]);
                store_gains(&gains, l, r);
                sink.append(Panned::new(source.convert_samples::<f32>(), gains));
            }
        }
        self.finish(sink, token);
    }

    /// Either detach the sink (fire-and-forget) or, for a `playThen` one-shot,
    /// wait for it on its own thread (so the frame loop never blocks) and report
    /// the token back to the main loop.
    fn finish(&self, sink: Sink, token: Option<u64>) {
        match token {
            None => sink.detach(),
            Some(tok) => {
                let tx = self.completion_tx.clone();
                std::thread::spawn(move || {
                    sink.sleep_until_end();
                    let _ = tx.send(tok);
                });
            }
        }
    }

    fn decode(&self, path: &str) -> Option<Decoder<BufReader<File>>> {
        let file = match File::open(path) {
            Ok(f) => f,
            Err(e) => {
                self.warn_once(path, format_args!("[audio] open '{path}': {e}"));
                return None;
            }
        };
        match Decoder::new(BufReader::new(file)) {
            Ok(s) => Some(s),
            Err(e) => {
                self.warn_once(path, format_args!("[audio] decode '{path}': {e}"));
                None
            }
        }
    }

    /// Log an asset-failure message the first time `path` fails, then stay quiet
    /// for it — a soundscape voice with a missing/undecodable file is re-spawned
    /// every frame, so an un-deduped `eprintln!` would spam the console.
    fn warn_once(&self, path: &str, message: std::fmt::Arguments) {
        if first_failure(&self.warned_paths, path) {
            eprintln!("{message}");
        }
    }
}

/// Record `path` as a failed asset and report whether this is the FIRST failure
/// for it (so the caller logs once and stays quiet thereafter). The `contains`
/// guard matters: a missing soundscape voice is re-`decode`d every frame, so the
/// common case is an already-known path — avoid allocating an owned `String` for
/// it, and only allocate on the genuinely-first failure.
fn first_failure(warned: &RefCell<HashSet<String>>, path: &str) -> bool {
    let mut warned = warned.borrow_mut();
    if warned.contains(path) {
        return false;
    }
    warned.insert(path.to_string());
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_failure_warns_once_per_path() {
        let warned = RefCell::new(HashSet::new());
        // First time a path fails -> warn; repeats (every frame) -> stay quiet.
        assert!(first_failure(&warned, "missing.ogg"));
        assert!(!first_failure(&warned, "missing.ogg"));
        assert!(!first_failure(&warned, "missing.ogg"));
        // A different path warns on its own first failure.
        assert!(first_failure(&warned, "other.ogg"));
        assert!(!first_failure(&warned, "other.ogg"));
    }
}
