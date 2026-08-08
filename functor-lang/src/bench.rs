//! `functor-lang bench` — a dependency-light interpreter microbenchmark harness.
//!
//! # What it measures
//!
//! The Functor Lang *interpreter*, isolated from parsing and lowering. A benchmark file
//! is an ordinary `.fun` project whose entry defines a zero-arg `let main`; the
//! harness parses + lowers + loads it **once** (untimed), then times *repeated
//! evaluation of `main()`*. So the reported number is per-`main()`-evaluation
//! cost — the tree-walking interpreter's hot path — not startup, parse, or
//! typecheck. (Each timed call goes through [`functor_lang::Session::call`], which spins
//! up a fresh interpreter over the session's globals exactly as the runtime
//! does per frame, so the number is representative of live per-frame cost.)
//!
//! # Methodology (reproducibility)
//!
//! 1. **Warmup** — evaluate `main()` for [`WARMUP`] wall-clock, discarding
//!    results (warms caches / branch predictor / any lazy init).
//! 2. **Auto-calibrate** — grow the iteration count until one timed batch takes
//!    at least [`MIN_BATCH`], so short benches aren't dominated by clock noise.
//! 3. **Median of [`SAMPLES`]** — take that many timed batches and report the
//!    median ns/op, plus the min/max spread as a stability signal.
//!
//! Absolute numbers are **machine-dependent**: this is a diagnostic / A-B tool,
//! not a pass/fail gate. To evaluate a change, run it on the base ref and on
//! your branch *on the same machine* and compare. See `functor-lang/benches/README.md`.

use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::process::exit;
use std::time::{Duration, Instant};

/// Warmup wall-clock before timing begins.
const WARMUP: Duration = Duration::from_millis(50);
/// Floor duration for one timed batch; the iteration count is calibrated up to
/// meet it so the clock's resolution isn't the thing being measured.
const MIN_BATCH: Duration = Duration::from_millis(100);
/// Number of timed batches; the reported ns/op is their median.
const SAMPLES: usize = 5;

/// One benchmark's result. `*_ns` are per-`main()`-evaluation nanoseconds.
pub struct BenchResult {
    pub name: String,
    pub iters: u64,
    pub samples: usize,
    pub median_ns: f64,
    pub min_ns: f64,
    pub max_ns: f64,
}

/// CLI entry: `functor-lang bench [--all] [--json] [<file.fun> | <dir>]`.
pub fn main(args: &[String]) -> ! {
    let mut json = false;
    let mut all = false;
    let mut path: Option<&str> = None;
    for arg in args {
        match arg.as_str() {
            "--json" => json = true,
            "--all" => all = true,
            other if other.starts_with('-') => {
                eprintln!("functor-lang bench: unknown flag `{other}`");
                exit(2);
            }
            other => {
                if path.is_some() {
                    eprintln!("functor-lang bench: expected a single file or directory");
                    exit(2);
                }
                path = Some(other);
            }
        }
    }

    // Resolve the set of benchmark files to run.
    let files = match (all, path) {
        (true, _) | (false, None) => collect_dir(&default_corpus()),
        (false, Some(p)) => {
            let p = Path::new(p);
            if p.is_dir() {
                collect_dir(p)
            } else {
                Ok(vec![p.to_path_buf()])
            }
        }
    };
    let files = files.unwrap_or_else(|err| {
        eprintln!("functor-lang bench: {err}");
        exit(1);
    });
    if files.is_empty() {
        eprintln!("functor-lang bench: no .fun benchmarks found");
        exit(1);
    }

    let mut results = Vec::with_capacity(files.len());
    for file in &files {
        match run_one(file) {
            Ok(result) => results.push(result),
            Err(err) => {
                eprintln!("{err}");
                exit(1);
            }
        }
    }

    if json {
        print!("{}", render_json(&results));
    } else {
        print!("{}", render_table(&results));
    }
    exit(0);
}

/// Parse + lower + load `path` once, then time repeated `main()` evaluation.
pub fn run_one(path: &Path) -> Result<BenchResult, String> {
    let project = functor_lang::project::load(path).map_err(|err| err.render())?;
    let mut host = functor_lang::NoHost;
    let session =
        functor_lang::Session::load(&project.module, &mut host).map_err(|f| render_error(&project, &f.error))?;
    if session.global("main").is_none() {
        return Err(format!(
            "{}: no zero-arg `let main` to benchmark (see functor-lang/benches/README.md)",
            path.display()
        ));
    }
    // One real evaluation up front: surfaces a runtime error (or an arity
    // mismatch if `main` takes args) as a clean diagnostic before timing.
    session
        .call("main", Vec::new(), &mut host)
        .map_err(|err| render_error(&project, &err))?;

    // Warmup.
    let warm_start = Instant::now();
    while warm_start.elapsed() < WARMUP {
        black_box(time_batch(&session, 1));
    }

    // Calibrate the iteration count up to the batch floor.
    let iters = calibrate(&session);

    // Median of SAMPLES timed batches.
    let mut per_op: Vec<f64> = (0..SAMPLES)
        .map(|_| time_batch(&session, iters).as_nanos() as f64 / iters as f64)
        .collect();
    per_op.sort_by(|a, b| a.partial_cmp(b).expect("finite timings"));

    Ok(BenchResult {
        name: bench_name(path),
        iters,
        samples: SAMPLES,
        median_ns: per_op[per_op.len() / 2],
        min_ns: per_op[0],
        max_ns: per_op[per_op.len() - 1],
    })
}

/// Time `n` back-to-back `main()` evaluations; `black_box` the result so the
/// optimizer can't elide the work.
fn time_batch(session: &functor_lang::Session, n: u64) -> Duration {
    let mut host = functor_lang::NoHost;
    let start = Instant::now();
    for _ in 0..n {
        let value = session
            .call("main", Vec::new(), &mut host)
            .expect("main validated before timing");
        black_box(value);
    }
    start.elapsed()
}

/// Grow the iteration count until a batch takes at least [`MIN_BATCH`].
fn calibrate(session: &functor_lang::Session) -> u64 {
    let mut iters: u64 = 1;
    loop {
        let elapsed = time_batch(session, iters);
        if elapsed >= MIN_BATCH || iters >= 1_000_000_000 {
            return iters.max(1);
        }
        // Scale toward the floor with headroom; always at least double so we
        // converge quickly even when `elapsed` is near zero.
        let factor = (MIN_BATCH.as_secs_f64() / elapsed.as_secs_f64().max(1e-9)).max(2.0);
        iters = ((iters as f64 * factor).ceil() as u64).max(iters + 1);
    }
}

fn bench_name(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// The committed corpus directory, resolved relative to the crate at build time
/// so `functor-lang bench --all` works from any cwd.
fn default_corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("benches/corpus")
}

/// All `*.fun` files in `dir`, sorted by name (stable table order).
fn collect_dir(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|err| format!("cannot read {}: {err}", dir.display()))?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|ext| ext == "fun"))
        .collect();
    files.sort();
    Ok(files)
}

fn render_error(project: &functor_lang::project::Project, err: &functor_lang::RunError) -> String {
    let (file, line, col) = project.sources.resolve(err.span.start);
    format!("{}:{line}:{col}: error: {}", file.path.display(), err.message)
}

/// Pretty-print a per-op nanosecond count with an appropriate unit.
fn fmt_time(ns: f64) -> String {
    if ns >= 1_000_000_000.0 {
        format!("{:.2} s", ns / 1_000_000_000.0)
    } else if ns >= 1_000_000.0 {
        format!("{:.2} ms", ns / 1_000_000.0)
    } else if ns >= 1_000.0 {
        format!("{:.2} us", ns / 1_000.0)
    } else {
        format!("{ns:.1} ns")
    }
}

fn render_table(results: &[BenchResult]) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "functor-lang interpreter benchmarks — median of {SAMPLES} samples, \u{2265}{}ms/sample, after {}ms warmup.\n",
        MIN_BATCH.as_millis(),
        WARMUP.as_millis()
    ));
    out.push_str("time/op is per main() evaluation (parse + lower done once, untimed).\n");
    out.push_str("Machine-dependent — A-B on the SAME machine; not a CI gate.\n\n");
    out.push_str(&format!(
        "{:<18} {:>10} {:>13} {:>13} {:>9}\n",
        "benchmark", "iters", "time/op", "ops/s", "spread"
    ));
    for r in results {
        let ops = 1_000_000_000.0 / r.median_ns;
        let spread = (r.max_ns - r.min_ns) / r.median_ns * 100.0;
        out.push_str(&format!(
            "{:<18} {:>10} {:>13} {:>13.1} {:>8.1}%\n",
            r.name,
            r.iters,
            fmt_time(r.median_ns),
            ops,
            spread
        ));
    }
    out
}

fn render_json(results: &[BenchResult]) -> String {
    let mut items = Vec::with_capacity(results.len());
    for r in results {
        items.push(format!(
            "  {{\"name\": {:?}, \"iters\": {}, \"samples\": {}, \"ns_per_op\": {:.3}, \"min_ns_per_op\": {:.3}, \"max_ns_per_op\": {:.3}, \"ops_per_sec\": {:.3}}}",
            r.name,
            r.iters,
            r.samples,
            r.median_ns,
            r.min_ns,
            r.max_ns,
            1_000_000_000.0 / r.median_ns
        ));
    }
    format!("[\n{}\n]\n", items.join(",\n"))
}
