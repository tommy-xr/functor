//! Speech-to-text timing on this machine: model check and load, then each
//! clip's first and warm transcriptions, with the words it got wrong.
//!
//!   cargo run --release -p functor-stt --features whisper --example stt_bench -- \
//!     [--model <file>] [--repeat N] [--cpu] [--threads N] [<clip.wav>=<expected text> ...]
//!
//! With no clips, the checked-in fixtures (macOS `say`, 16 kHz mono). The
//! model defaults to the app's, fetched on first use; the rest of
//! [`Tuning`] to the app's, `--cpu` keeping a GPU build off the GPU. Prints one JSON line
//! per clip, then a table. Load is the process's first: run it again for
//! another cold sample (the OS file cache stays warm).

use std::path::PathBuf;
use std::time::Instant;

use functor_stt::{
    ensure_model, models_dir, read_wav, Transcriber, Tuning, Whisper, DEFAULT_MODEL, SAMPLE_RATE,
};

const FIXTURES: [(&str, &str); 3] = [
    ("jump-twice-as-high.wav", "Please make the player jump twice as high as before."),
    (
        "make-the-ball-red.wav",
        "Make the ball red, twice as big, and have it bounce higher every time it hits the floor.",
    ),
    (
        "crates-sky-and-coin.wav",
        "Add three red crates near the spawn point, make the player run a little faster, \
change the sky to a warm orange sunset, and play a sound when a coin is picked up.",
    ),
];

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let (mut model, mut repeat, mut clips) = (None, 3, Vec::new());
    let mut tuning = Tuning::default();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--model" => model = args.next().map(PathBuf::from),
            "--repeat" => repeat = args.next().and_then(|n| n.parse().ok()).ok_or("--repeat N")?,
            "--cpu" => tuning.gpu = false,
            "--threads" => {
                tuning.threads = args.next().and_then(|n| n.parse().ok()).ok_or("--threads N")?
            }
            clip => {
                let (path, text) = clip.split_once('=').ok_or("a clip is <file.wav>=<text>")?;
                clips.push((PathBuf::from(path), text.to_string()));
            }
        }
    }
    if clips.is_empty() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures");
        clips = FIXTURES.iter().map(|(f, t)| (dir.join(f), t.to_string())).collect();
    }

    let started = Instant::now();
    let model = match model {
        Some(path) => path,
        None => {
            let dir = models_dir(std::env::var_os("FUNCTOR_MODELS"), std::env::home_dir())
                .ok_or("no models dir")?;
            ensure_model(&dir, &DEFAULT_MODEL)?
        }
    };
    let check_ms = ms(started);
    let started = Instant::now();
    let whisper = Whisper::load_tuned(&model, tuning)?;
    let load_ms = ms(started);
    eprintln!(
        "model {}: checked {check_ms:.0} ms, loaded {load_ms:.0} ms; {tuning:?}",
        model.display()
    );

    let mut rows = Vec::new();
    for (path, expected) in &clips {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let pcm = read_wav(&bytes)?;
        let audio_s = pcm.len() as f64 / SAMPLE_RATE as f64;
        let mut times = Vec::new();
        let mut text = String::new();
        for _ in 0..=repeat {
            let started = Instant::now();
            text = whisper.transcribe(&pcm)?;
            times.push(ms(started));
        }
        let first = times[0];
        let warm = times[1..].iter().copied().fold(f64::INFINITY, f64::min);
        let wrong = word_errors(expected, &text);
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        println!(
            "{}",
            serde_json_line(&name, audio_s, check_ms, load_ms, first, warm, wrong, &text)
        );
        rows.push((name, audio_s, first, warm, wrong, text));
    }
    println!("\n| clip | audio s | first ms | warm ms (best of {repeat}) | words wrong | text |");
    println!("|---|---|---|---|---|---|");
    for (name, audio, first, warm, wrong, text) in rows {
        println!("| {name} | {audio:.1} | {first:.0} | {warm:.0} | {wrong} | {text} |");
    }
    Ok(())
}

fn ms(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1000.0
}

/// Word-level edit distance, ignoring case and punctuation.
fn word_errors(expected: &str, got: &str) -> usize {
    let words = |s: &str| -> Vec<String> {
        s.split_whitespace()
            .map(|w| w.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase())
            .filter(|w| !w.is_empty())
            .collect()
    };
    let (a, b) = (words(expected), words(got));
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, wa) in a.iter().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, wb) in b.iter().enumerate() {
            let next = (row[j + 1] + 1).min(row[j] + 1).min(prev + usize::from(wa != wb));
            prev = row[j + 1];
            row[j + 1] = next;
        }
    }
    row[b.len()]
}

#[allow(clippy::too_many_arguments)]
fn serde_json_line(
    clip: &str,
    audio_s: f64,
    check_ms: f64,
    load_ms: f64,
    first_ms: f64,
    warm_ms: f64,
    wrong: usize,
    text: &str,
) -> String {
    format!(
        "{{\"clip\":{clip:?},\"audio_s\":{audio_s:.2},\"check_ms\":{check_ms:.0},\"load_ms\":{load_ms:.0},\
\"first_ms\":{first_ms:.0},\"warm_ms\":{warm_ms:.0},\"words_wrong\":{wrong},\"text\":{text:?}}}"
    )
}
