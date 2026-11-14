//! whisper.cpp, and its model fetched once and verified.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use sha2::{Digest, Sha256};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState};

use crate::{Model, Transcriber, SAMPLE_RATE};

/// How whisper.cpp runs: [`Tuning::default`] is the app's; `stt_bench`
/// compares others.
#[derive(Clone, Copy, Debug)]
pub struct Tuning {
    /// On the GPU, in a build that has one (Metal, on macOS); else the CPU.
    pub gpu: bool,
    /// CPU threads for the work that stays on the CPU.
    pub threads: usize,
}

impl Default for Tuning {
    fn default() -> Tuning {
        Tuning {
            gpu: true,
            // More than the performance cores only waits on the slowest.
            threads: std::thread::available_parallelism().map_or(4, |n| n.get().min(4)),
        }
    }
}

/// A loaded model, its decoder state kept from clip to clip.
pub struct Whisper {
    state: Mutex<WhisperState>,
    tuning: Tuning,
}

impl Whisper {
    pub fn load(model: &Path) -> Result<Whisper, String> {
        Whisper::load_tuned(model, Tuning::default())
    }

    /// Loads `model`, then transcribes a second of silence: the first clip
    /// after a load otherwise pays for setting up (GPU kernels, buffers).
    pub fn load_tuned(model: &Path, tuning: Tuning) -> Result<Whisper, String> {
        // whisper.cpp logs to stderr unless hooked; hooked with no backend, it is quiet.
        whisper_rs::install_logging_hooks();
        let path = model.to_str().ok_or("the model path is not UTF-8")?;
        let mut params = WhisperContextParameters::default();
        params.use_gpu(tuning.gpu);
        let ctx = WhisperContext::new_with_params(path, params)
            .map_err(|e| format!("cannot load {}: {e}", model.display()))?;
        let state = ctx.create_state().map_err(|e| e.to_string())?;
        let whisper = Whisper {
            state: Mutex::new(state),
            tuning,
        };
        // Only a head start: a failure shows on the first real clip.
        let _ = whisper.transcribe(&[0; SAMPLE_RATE as usize]);
        Ok(whisper)
    }
}

impl Transcriber for Whisper {
    fn transcribe(&self, pcm: &[i16]) -> Result<String, String> {
        let mut audio = vec![0.0; pcm.len()];
        whisper_rs::convert_integer_to_float_audio(pcm, &mut audio).map_err(|e| e.to_string())?;
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some("en"));
        params.set_n_threads(self.tuning.threads as i32);
        params.set_no_context(true);
        params.set_single_segment(true);
        params.set_no_timestamps(true);
        // No "[MUSIC]"-style tokens for noise.
        params.set_suppress_nst(true);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        // `full` starts each clip afresh, so a panic mid-clip leaves nothing behind.
        let mut state = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        state.full(params, &audio).map_err(|e| e.to_string())?;
        let text: String = state.as_iter().map(|s| s.to_string()).collect();
        Ok(words(&text))
    }
}

/// `text` trimmed, or empty when it only names a sound: whisper answers
/// silence with "[BLANK_AUDIO]" or "(silence)".
fn words(text: &str) -> String {
    let text = text.trim();
    let annotation = (text.starts_with('[') && text.ends_with(']'))
        || (text.starts_with('(') && text.ends_with(')'));
    match annotation {
        true => String::new(),
        false => text.to_string(),
    }
}

/// `model` in `dir`, checked against its size and SHA-256, and fetched
/// first if it is missing or does not match. A download goes to a file of
/// its own and is renamed into place only once it matches, so downloads at
/// once never mix and the model file is always whole.
pub fn ensure_model(dir: &Path, model: &Model) -> Result<PathBuf, String> {
    let path = dir.join(model.file);
    if std::fs::File::open(&path).is_ok_and(|f| matches(f, model)) {
        return Ok(path);
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let part = dir.join(format!(
        "{}.{}-{stamp}.part",
        model.file,
        std::process::id()
    ));
    let kept = fetch(model, &part).and_then(|()| {
        std::fs::rename(&part, &path).map_err(|e| format!("cannot keep the model: {e}"))
    });
    if kept.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    kept.map(|()| path)
}

/// `file` is `model`: its size and SHA-256.
fn matches(file: std::fs::File, model: &Model) -> bool {
    if file.metadata().map(|m| m.len()).ok() != Some(model.bytes) {
        return false;
    }
    let mut hash = Sha256::new();
    std::io::copy(&mut std::io::BufReader::new(file), &mut hash).is_ok()
        && hex(hash) == model.sha256
}

fn hex(hash: Sha256) -> String {
    hash.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn fetch(model: &Model, part: &Path) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        // A plain client name: nothing about the user.
        .user_agent("functor-app")
        // Per connect and per read: a stalled download fails, a slow one does not.
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())?;
    let mut response = client
        .get(model.url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("cannot download the speech model: {e}"))?;
    let mut file = std::fs::File::create_new(part).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut total = 0u64;
    let mut buf = vec![0; 1 << 16];
    loop {
        let n = response
            .read(&mut buf)
            .map_err(|e| format!("the speech model download broke off: {e}"))?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > model.bytes {
            return Err("the speech model download is larger than expected".into());
        }
        hash.update(&buf[..n]);
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
    }
    file.sync_all().map_err(|e| e.to_string())?;
    let got = hex(hash);
    match got == model.sha256 {
        true => Ok(()),
        false => Err(format!(
            "the speech model download is corrupt (sha256 {got}, expected {})",
            model.sha256
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sound_alone_is_no_words() {
        assert_eq!(words(" [BLANK_AUDIO] "), "");
        assert_eq!(words("(silence)"), "");
        assert_eq!(words(" Make it red. "), "Make it red.");
    }

    #[test]
    fn a_model_that_does_not_match_is_not_used() {
        let dir = tempfile::tempdir().unwrap();
        let model = Model {
            file: "m.bin",
            url: "http://127.0.0.1:9/never",
            sha256: "00",
            bytes: 3,
        };
        std::fs::write(dir.path().join("m.bin"), b"abc").unwrap();
        let file = std::fs::File::open(dir.path().join("m.bin")).unwrap();
        assert!(!matches(file, &model));
        // Fetched again: here, from nowhere, so an error and no part left.
        assert!(ensure_model(dir.path(), &model).is_err());
        let left: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(left.len(), 1);
    }
}
