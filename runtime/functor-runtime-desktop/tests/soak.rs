//! Memory soak regression test.
//!
//! Runs the `synthwave` sample headless for ~20s and asserts its resident-set
//! size (RSS) does not trend upward — a guard against per-frame GPU/CPU leaks
//! like the heightmap-mesh leak (a fresh VAO/VBO/EBO built and cached, never
//! evicted, every frame — the terrain animates, so the cache key changed each
//! frame). `synthwave` is the worst case: its heightmap is a pure function of
//! time, so every frame reuploads the terrain.
//!
//! The fix gives each `(rows, cols)` a single persistent mesh, re-uploaded in
//! place, so RSS is flat. Before the fix, RSS grew ~linearly (measured ~8 MB/s
//! on Linux). This test fails if the slope of a least-squares fit over the
//! sampled RSS exceeds [`MAX_SLOPE_MB_PER_S`].
//!
//! Ignored by default: it needs a GL display and runs for ~20s. Run it with:
//!
//! ```sh
//! cargo test -p functor-runtime-desktop --test soak -- --ignored --nocapture
//! ```
//!
//! Note: some GL drivers (notably Apple's legacy GL-over-Metal) periodically
//! reclaim orphaned buffers, so a *leaking* build shows a large RSS sawtooth
//! rather than a clean ramp there; the least-squares slope still stays well
//! under the threshold once the leak is fixed (the per-frame GPU-buffer growth is
//! gone — only bounded, immediately-freed CPU scratch remains). The threshold is
//! set to catch the ~MB/s-class linear leak this test guards.

use std::path::PathBuf;
use std::process::Command;
use std::thread::sleep;
use std::time::Duration;

/// Fail if RSS trends upward faster than this. The leak this guards against was
/// ~8 MB/s; a fixed build is flat (measured ~0.02 MB/s of allocator jitter).
const MAX_SLOPE_MB_PER_S: f64 = 0.5;

/// Total wall-clock the sample runs headless before capturing its final frame
/// and exiting.
const RUN_SECS: u64 = 22;
/// Skip the first few seconds (asset load, first-frame hydration) before sampling.
const WARMUP_SECS: u64 = 3;
/// RSS sample period.
const SAMPLE_MS: u64 = 500;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("resolve repo root")
}

/// Path to the `functor` CLI binary, next to this test binary
/// (`target/<profile>/functor`). Build it first (`cargo build --bin functor`).
fn functor_bin() -> PathBuf {
    let exe = std::env::current_exe().expect("locate test binary");
    let target_dir = exe
        .parent()
        .and_then(|deps| deps.parent())
        .expect("target/<profile> dir");
    let name = if cfg!(windows) { "functor.exe" } else { "functor" };
    let bin = target_dir.join(name);
    assert!(
        bin.exists(),
        "functor CLI not found at {} — build it first (`cargo build --bin functor`)",
        bin.display()
    );
    bin
}

/// Resident-set size of `pid` in KB via `ps`, or `None` if the process is gone.
fn rss_kb(pid: u32) -> Option<u64> {
    let out = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.trim().parse::<u64>().ok()
}

/// Least-squares slope of `(t_seconds, rss_mb)` samples, in MB/s.
fn slope_mb_per_s(samples: &[(f64, f64)]) -> f64 {
    let n = samples.len() as f64;
    let sx: f64 = samples.iter().map(|(x, _)| x).sum();
    let sy: f64 = samples.iter().map(|(_, y)| y).sum();
    let sxx: f64 = samples.iter().map(|(x, _)| x * x).sum();
    let sxy: f64 = samples.iter().map(|(x, y)| x * y).sum();
    (n * sxy - sx * sy) / (n * sxx - sx * sx)
}

#[test]
#[ignore = "needs a GL display; runs ~20s. Run with --ignored"]
fn synthwave_rss_is_flat() {
    let sample_dir = repo_root().join("examples").join("synthwave");
    let out = std::env::temp_dir().join("functor-soak-synthwave.png");
    let _ = std::fs::remove_file(&out);

    // Headless run (`--capture-frame` implies `--hidden`), captured/exited at RUN_SECS.
    let mut child = Command::new(functor_bin())
        .args([
            "-d",
            sample_dir.to_str().unwrap(),
            "run",
            "native",
            "--",
            "--capture-frame",
            out.to_str().unwrap(),
            "--capture-time",
            &RUN_SECS.to_string(),
        ])
        .spawn()
        .expect("spawn functor");
    let pid = child.id();

    sleep(Duration::from_secs(WARMUP_SECS));

    let mut samples: Vec<(f64, f64)> = Vec::new();
    let sample_count = ((RUN_SECS - WARMUP_SECS - 1) * 1000 / SAMPLE_MS) as usize;
    let mut exited_early = false;
    for i in 0..sample_count {
        match rss_kb(pid) {
            Some(kb) => {
                let t = WARMUP_SECS as f64 + (i as f64) * (SAMPLE_MS as f64 / 1000.0);
                samples.push((t, kb as f64 / 1024.0));
            }
            None => {
                exited_early = true; // process gone before the window ended — a crash
                break;
            }
        }
        sleep(Duration::from_millis(SAMPLE_MS));
    }

    let status = child.wait().expect("wait for functor");

    // Trust the slope only if the process ran the whole window and exited cleanly
    // with the expected capture — otherwise a crash could masquerade as "flat".
    assert!(
        !exited_early,
        "functor exited before the {RUN_SECS}s window ended ({} samples) — treat as a failure, not flat RSS",
        samples.len()
    );
    assert!(status.success(), "functor exited with {status}");
    assert!(
        out.exists(),
        "capture {} was not written — the run did not complete normally",
        out.display()
    );
    assert!(
        samples.len() >= sample_count - 1,
        "too few RSS samples ({} of {sample_count})",
        samples.len()
    );

    let slope = slope_mb_per_s(&samples);
    let first = samples.first().unwrap().1;
    let last = samples.last().unwrap().1;
    let peak = samples.iter().map(|(_, y)| *y).fold(0.0_f64, f64::max);
    println!(
        "soak: {} samples, first={first:.1}MB last={last:.1}MB peak={peak:.1}MB slope={slope:.3} MB/s",
        samples.len()
    );

    assert!(
        slope < MAX_SLOPE_MB_PER_S,
        "RSS trends up at {slope:.3} MB/s (max {MAX_SLOPE_MB_PER_S} MB/s) — a per-frame leak may have regressed \
         (first={first:.1}MB last={last:.1}MB peak={peak:.1}MB)"
    );
}
