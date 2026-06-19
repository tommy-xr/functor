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

use std::fs::File;
use std::io::BufReader;
use std::sync::mpsc::Sender;

use functor_runtime_common::audio::{AudioCommand, Listener};
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

/// Holds the audio device.
pub struct AudioPlayer {
    // The stream must stay alive for anything to play; held though otherwise unused.
    _stream: OutputStream,
    handle: OutputStreamHandle,
    completion_tx: Sender<u64>,
    listener: Listener,
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
