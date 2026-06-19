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
//! back to the game. Positioned one-shots (`Audio.playAt`) play through a
//! `SpatialSink`, panned and attenuated relative to the listener (the render
//! camera), which `main.rs` updates from the frame's camera each frame.

use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::sync::mpsc::Sender;

use functor_runtime_common::audio::{AudioCommand, AudioScene, AudioSource, Listener, SceneUpdate};
use rodio::source::Source;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, SpatialSink};

/// Distance (world units) from the listener to each ear; sets how strongly
/// spatial sounds pan left/right.
const HALF_EAR: f32 = 0.3;

/// A rodio sink we can either detach (fire-and-forget) or wait on to report
/// completion. Both `Sink` and `SpatialSink` qualify, so the play/finish tail is
/// shared across spatial and non-spatial one-shots.
trait Playable: Send + 'static {
    fn sleep_until_end(&self);
    fn detach(self);
}

impl Playable for Sink {
    fn sleep_until_end(&self) {
        Sink::sleep_until_end(self)
    }
    fn detach(self) {
        Sink::detach(self)
    }
}

impl Playable for SpatialSink {
    fn sleep_until_end(&self) {
        SpatialSink::sleep_until_end(self)
    }
    fn detach(self) {
        SpatialSink::detach(self)
    }
}

/// A live looping voice in the soundscape: the rodio sink plus the last-applied
/// source, so the reconciler can diff gain/position changes against it.
struct LoopVoice {
    source: AudioSource,
    sink: VoiceSink,
}

enum VoiceSink {
    Plain(Sink),
    Spatial(SpatialSink),
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

    /// Re-aim the live spatial voices at the current listener (called each frame
    /// after `set_listener`), so a looping emitter pans/attenuates as the camera
    /// moves around it — even on frames where its source didn't change.
    pub fn update_spatial_listener(&self) {
        let (left, right) = self.listener.ears(HALF_EAR);
        for voice in self.voices.values() {
            if let VoiceSink::Spatial(sink) = &voice.sink {
                sink.set_left_ear_position(left);
                sink.set_right_ear_position(right);
            }
        }
    }

    fn spawn_voice(&mut self, src: AudioSource) {
        let Some(source) = self.decode(&src.sound) else {
            return;
        };
        // Buffered so the decoded samples can be cloned and looped forever.
        let looped = source.buffered().repeat_infinite();
        let sink = match src.position {
            None => match Sink::try_new(&self.handle) {
                Ok(sink) => {
                    sink.set_volume(src.gain);
                    sink.append(looped);
                    VoiceSink::Plain(sink)
                }
                Err(e) => {
                    eprintln!("[audio] sink: {e}");
                    return;
                }
            },
            Some(pos) => {
                let (left, right) = self.listener.ears(HALF_EAR);
                match SpatialSink::try_new(&self.handle, pos, left, right) {
                    Ok(sink) => {
                        sink.set_volume(src.gain);
                        sink.append(looped);
                        VoiceSink::Spatial(sink)
                    }
                    Err(e) => {
                        eprintln!("[audio] spatial sink: {e}");
                        return;
                    }
                }
            }
        };
        self.voices.insert(src.key.clone(), LoopVoice { source: src, sink });
    }

    fn update_voice(&mut self, src: AudioSource) {
        // A flip in spatial-ness (None <-> Some) is a different sink type, so
        // respawn rather than mutate in place.
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
            match &voice.sink {
                VoiceSink::Plain(sink) => sink.set_volume(src.gain),
                VoiceSink::Spatial(sink) => {
                    sink.set_volume(src.gain);
                    if let Some(pos) = src.position {
                        sink.set_emitter_position(pos);
                    }
                }
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
        match position {
            None => match Sink::try_new(&self.handle) {
                Ok(sink) => {
                    if self.append(&sink, &sound, gain) {
                        self.finish(sink, token);
                    }
                }
                Err(e) => eprintln!("[audio] sink: {e}"),
            },
            Some(pos) => {
                let (left, right) = self.listener.ears(HALF_EAR);
                match SpatialSink::try_new(&self.handle, pos, left, right) {
                    Ok(sink) => {
                        if self.append_spatial(&sink, &sound, gain) {
                            self.finish(sink, token);
                        }
                    }
                    Err(e) => eprintln!("[audio] spatial sink: {e}"),
                }
            }
        }
    }

    /// Either detach the sink (fire-and-forget) or, for a `playThen` one-shot,
    /// wait for it on its own thread (so the frame loop never blocks) and report
    /// the token back to the main loop.
    fn finish<S: Playable>(&self, sink: S, token: Option<u64>) {
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

    fn append(&self, sink: &Sink, path: &str, gain: f32) -> bool {
        match self.decode(path) {
            Some(source) => {
                sink.set_volume(gain);
                sink.append(source);
                true
            }
            None => false,
        }
    }

    fn append_spatial(&self, sink: &SpatialSink, path: &str, gain: f32) -> bool {
        match self.decode(path) {
            Some(source) => {
                sink.set_volume(gain);
                sink.append(source);
                true
            }
            None => false,
        }
    }

    fn decode(&self, path: &str) -> Option<Decoder<BufReader<File>>> {
        let file = match File::open(path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("[audio] open '{path}': {e}");
                return None;
            }
        };
        match Decoder::new(BufReader::new(file)) {
            Ok(s) => Some(s),
            Err(e) => {
                eprintln!("[audio] decode '{path}': {e}");
                None
            }
        }
    }
}
