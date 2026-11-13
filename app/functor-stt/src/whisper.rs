//! whisper.cpp, and its model fetched once and verified.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::{Model, Transcriber};

/// A loaded model.
pub struct Whisper {
    ctx: WhisperContext,
}

impl Whisper {
    pub fn load(model: &Path) -> Result<Whisper, String> {
        // whisper.cpp logs to stderr unless hooked; hooked with no backend, it is quiet.
        whisper_rs::install_logging_hooks();
        let path = model.to_str().ok_or("the model path is not UTF-8")?;
        let ctx = WhisperContext::new_with_params(path, WhisperContextParameters::default())
            .map_err(|e| format!("cannot load {}: {e}", model.display()))?;
        Ok(Whisper { ctx })
    }
}

impl Transcriber for Whisper {
    fn transcribe(&self, pcm: &[i16]) -> Result<String, String> {
        let mut audio = vec![0.0; pcm.len()];
        whisper_rs::convert_integer_to_float_audio(pcm, &mut audio).map_err(|e| e.to_string())?;
        let mut state = self.ctx.create_state().map_err(|e| e.to_string())?;
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some("en"));
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get().min(8));
        params.set_n_threads(threads as i32);
        params.set_no_context(true);
        params.set_single_segment(true);
        params.set_no_timestamps(true);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        state.full(params, &audio).map_err(|e| e.to_string())?;
        let text: String = state.as_iter().map(|s| s.to_string()).collect();
        Ok(text.trim().to_string())
    }
}

/// `model` in `dir`, fetched first if it is not there. A download goes to a
/// `.part` file and is renamed only once its SHA-256 matches.
pub fn ensure_model(dir: &Path, model: &Model) -> Result<PathBuf, String> {
    let path = dir.join(model.file);
    if path.is_file() {
        return Ok(path);
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let part = dir.join(format!("{}.part", model.file));
    let fetched = fetch(model, &part);
    if let Err(e) = fetched {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    std::fs::rename(&part, &path).map_err(|e| format!("cannot keep the model: {e}"))?;
    Ok(path)
}

fn fetch(model: &Model, part: &Path) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        // A plain client name: nothing about the user.
        .user_agent("functor-app")
        .timeout(None)
        .build()
        .map_err(|e| e.to_string())?;
    let mut response = client
        .get(model.url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("cannot download the speech model: {e}"))?;
    let mut file = std::fs::File::create(part).map_err(|e| e.to_string())?;
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
    let got: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    match got == model.sha256 {
        true => Ok(()),
        false => Err(format!(
            "the speech model download is corrupt (sha256 {got}, expected {})",
            model.sha256
        )),
    }
}
