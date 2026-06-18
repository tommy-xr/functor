//! Native audio output via rodio.
//!
//! The host owns the output device (a rodio `OutputStream`) for the lifetime of
//! the runner, so it survives hot reload — the game dylib only *queues*
//! `AudioCommand`s, which `main.rs` drains each frame and hands here. Sounds
//! play on rodio's own thread, so a play call never stalls the frame loop.

use std::fs::File;
use std::io::BufReader;
use std::sync::mpsc::Sender;

use functor_runtime_common::audio::AudioCommand;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink};

/// Holds the audio device. Fire-and-forget one-shots are detached and free
/// themselves when done; a `playThen` one-shot (with a token) plays on a thread
/// that waits for it to finish and reports the token over `completion_tx`, which
/// `main.rs` forwards back to the game.
pub struct AudioPlayer {
    // The stream must stay alive for anything to play; held though otherwise unused.
    _stream: OutputStream,
    handle: OutputStreamHandle,
    completion_tx: Sender<u64>,
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
            }),
            Err(e) => {
                eprintln!("[audio] no output device, audio disabled: {e}");
                None
            }
        }
    }

    /// Perform one queued command.
    pub fn handle(&self, cmd: AudioCommand) {
        match cmd {
            AudioCommand::PlayOneShot { token, sound, gain } => match token {
                None => self.play_one_shot(&sound, gain),
                Some(tok) => self.play_one_shot_then(sound, gain, tok),
            },
        }
    }

    /// Decode + start a sound on a fresh sink. Returns the sink, or `None` (with
    /// a logged reason) if the file can't be opened/decoded.
    fn start(&self, path: &str, gain: f32) -> Option<Sink> {
        let file = match File::open(path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("[audio] open '{path}': {e}");
                return None;
            }
        };
        let source = match Decoder::new(BufReader::new(file)) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[audio] decode '{path}': {e}");
                return None;
            }
        };
        let sink = match Sink::try_new(&self.handle) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[audio] sink: {e}");
                return None;
            }
        };
        sink.set_volume(gain);
        sink.append(source);
        Some(sink)
    }

    fn play_one_shot(&self, path: &str, gain: f32) {
        if let Some(sink) = self.start(path, gain) {
            sink.detach(); // play to completion, then clean up on rodio's thread
        }
    }

    fn play_one_shot_then(&self, path: String, gain: f32, token: u64) {
        if let Some(sink) = self.start(&path, gain) {
            // Wait for the sound on its own thread (so the frame loop never
            // blocks), then report the token back to the main loop.
            let tx = self.completion_tx.clone();
            std::thread::spawn(move || {
                sink.sleep_until_end();
                let _ = tx.send(token);
            });
        }
    }
}
