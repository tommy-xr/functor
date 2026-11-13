//! Local speech-to-text for the desktop app. Audio is 16 kHz mono 16-bit
//! PCM (the device link's voice format); text comes back. Nothing leaves the
//! machine: the only network use is the one-time model download.
//!
//! - [`Fake`] answers a fixed text: tests and CI.
//! - `Whisper` (feature `whisper`) runs whisper.cpp on a model fetched on
//!   first use into [`models_dir`], checked against its SHA-256.

#[cfg(feature = "whisper")]
mod whisper;

use std::path::PathBuf;

#[cfg(feature = "whisper")]
pub use whisper::{ensure_model, Whisper};

/// The sample rate every transcriber takes.
pub const SAMPLE_RATE: u32 = 16_000;

/// Turns speech into text.
pub trait Transcriber: Send + Sync {
    /// The text spoken in `pcm` (16 kHz mono), trimmed; empty for silence.
    fn transcribe(&self, pcm: &[i16]) -> Result<String, String>;
}

/// Answers every clip with the same text.
pub struct Fake(pub String);

impl Transcriber for Fake {
    fn transcribe(&self, _pcm: &[i16]) -> Result<String, String> {
        Ok(self.0.clone())
    }
}

/// A whisper.cpp model: where it comes from and what it must hash to.
#[derive(Clone, Copy, Debug)]
pub struct Model {
    /// The file name, also its name in the models directory.
    pub file: &'static str,
    pub url: &'static str,
    /// Lowercase hex.
    pub sha256: &'static str,
    pub bytes: u64,
}

/// The model the app uses: whisper.cpp's English-only base model, small
/// (148 MB) and good enough for short commands.
pub const DEFAULT_MODEL: Model = Model {
    file: "ggml-base.en.bin",
    url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-base.en.bin",
    sha256: "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002",
    bytes: 147_964_211,
};

/// Where models are kept: `FUNCTOR_MODELS`, else `~/.functor/models`.
pub fn models_dir(env: Option<std::ffi::OsString>, user_home: Option<PathBuf>) -> Option<PathBuf> {
    env.filter(|e| !e.is_empty())
        .map(PathBuf::from)
        .or_else(|| Some(user_home?.join(".functor").join("models")))
}

/// The samples of a 16-bit PCM WAV at [`SAMPLE_RATE`], mono; refuses others.
pub fn read_wav(bytes: &[u8]) -> Result<Vec<i16>, String> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a WAV file".into());
    }
    let mut at = 12;
    let mut format = None;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let len = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        let end = (at + 8).saturating_add(len);
        match id {
            _ if end > bytes.len() => return Err("the WAV is cut short".into()),
            b"fmt " if len >= 16 => {
                let body = &bytes[at + 8..end];
                let u16_at = |i: usize| u16::from_le_bytes([body[i], body[i + 1]]);
                let rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
                format = Some((u16_at(0), u16_at(2), rate, u16_at(14)));
            }
            b"data" => {
                let body = &bytes[at + 8..end];
                return match format {
                    _ if len % 2 == 1 => Err("the WAV ends mid-sample".into()),
                    Some((1, 1, SAMPLE_RATE, 16)) => Ok(body
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|s| i16::from_le_bytes(*s))
                        .collect()),
                    _ => Err(format!(
                        "the WAV must be 16-bit PCM, mono, {SAMPLE_RATE} Hz"
                    )),
                };
            }
            _ => {}
        }
        // Chunks are padded to even sizes.
        at = at + 8 + len + (len & 1);
    }
    Err("the WAV has no audio".into())
}

/// `pcm` as a 16 kHz mono 16-bit WAV.
pub fn write_wav(pcm: &[i16]) -> Vec<u8> {
    let data = (pcm.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    out.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for s in pcm {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wav_round_trips_and_other_formats_are_refused() {
        let pcm = [0i16, 1, -1, i16::MAX, i16::MIN];
        assert_eq!(read_wav(&write_wav(&pcm)).unwrap(), pcm);
        let mut stereo = write_wav(&pcm);
        stereo[22] = 2;
        assert!(read_wav(&stereo).is_err());
        let mut fast = write_wav(&pcm);
        fast[24..28].copy_from_slice(&44_100u32.to_le_bytes());
        assert!(read_wav(&fast).is_err());
        assert!(read_wav(b"not a wav").is_err());
        let whole = write_wav(&pcm);
        assert!(read_wav(&whole[..whole.len() - 1]).is_err(), "cut short");
    }

    #[test]
    fn models_live_in_the_override_or_under_home() {
        let home = Some(PathBuf::from("/home/u"));
        assert_eq!(
            models_dir(None, home.clone()),
            Some(PathBuf::from("/home/u/.functor/models"))
        );
        assert_eq!(
            models_dir(Some("/m".into()), home),
            Some(PathBuf::from("/m"))
        );
    }

    /// The checked-in clip (macOS `say`, 16 kHz mono) through the real
    /// model: `FUNCTOR_STT_REAL=1 cargo test -p functor-stt --features
    /// whisper -- --nocapture` (downloads the model on first run).
    #[cfg(feature = "whisper")]
    #[test]
    fn whisper_transcribes_the_checked_in_clip() {
        if std::env::var_os("FUNCTOR_STT_REAL").is_none() {
            eprintln!("SKIPPED: set FUNCTOR_STT_REAL=1 to run the real model");
            return;
        }
        let dir = models_dir(std::env::var_os("FUNCTOR_MODELS"), std::env::home_dir())
            .expect("a models dir");
        let model = ensure_model(&dir, &DEFAULT_MODEL).expect("the model");
        let whisper = Whisper::load(&model).expect("loads");
        let clip = include_bytes!("../fixtures/make-the-ball-red.wav");
        let pcm = read_wav(clip).unwrap();
        let started = std::time::Instant::now();
        let text = whisper.transcribe(&pcm).unwrap();
        eprintln!(
            "{:.1} s of audio in {:?}: {text:?}",
            pcm.len() as f64 / SAMPLE_RATE as f64,
            started.elapsed()
        );
        let words = text.to_lowercase();
        assert!(words.contains("ball") && words.contains("red"), "{text}");
    }
}
