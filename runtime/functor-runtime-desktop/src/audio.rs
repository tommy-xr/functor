//! Native audio output via rodio.
//!
//! The host owns the output device (a rodio `OutputStream`) for the lifetime of
//! the runner, so it survives hot reload — the game dylib only *queues*
//! `AudioCommand`s, which `main.rs` drains each frame and hands here. Sounds
//! play on rodio's own thread, so a play call never stalls the frame loop.

use std::fs::File;
use std::io::BufReader;

use functor_runtime_common::audio::AudioCommand;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink};

/// Holds the audio device. One-shots are detached and free themselves when done.
pub struct AudioPlayer {
    // The stream must stay alive for anything to play; held though otherwise unused.
    _stream: OutputStream,
    handle: OutputStreamHandle,
}

impl AudioPlayer {
    /// Open the default output device. `None` when there is no device (headless
    /// / CI) — the runner then simply drops audio commands.
    pub fn new() -> Option<AudioPlayer> {
        match OutputStream::try_default() {
            Ok((stream, handle)) => Some(AudioPlayer {
                _stream: stream,
                handle,
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
            AudioCommand::PlayOneShot { sound, gain } => self.play_one_shot(&sound, gain),
        }
    }

    fn play_one_shot(&self, path: &str, gain: f32) {
        let file = match File::open(path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("[audio] open '{path}': {e}");
                return;
            }
        };
        let source = match Decoder::new(BufReader::new(file)) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[audio] decode '{path}': {e}");
                return;
            }
        };
        let sink = match Sink::try_new(&self.handle) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[audio] sink: {e}");
                return;
            }
        };
        sink.set_volume(gain);
        sink.append(source);
        sink.detach(); // play to completion, then clean up on rodio's thread
    }
}
