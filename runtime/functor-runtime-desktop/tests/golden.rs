//! Golden-image regression tests for the sample games.
//!
//! The scenarios are defined once in `golden-scenarios.json` at the repo root
//! and **shared** with the wasm Playwright test (`e2e/golden-wasm.spec.mjs`), so
//! a scenario added there is validated on both targets. Each scenario renders a
//! sample at a fixed frame time (so the pose is deterministic) via
//! `functor ... run native --fixed-time`, optionally with a `--debug-render`
//! mode, captures the framebuffer to a PNG, and compares it to a reference
//! with a small tolerance. This test runs every scenario whose `targets`
//! includes `"native"`.
//!
//! Each sample is a Functor Lang project (`functor.json` with `"language": "functor-lang"`) and
//! renders via `--functor-lang --game-path <entry>`; Functor Lang needs no build step (the runner
//! interprets the `.fun` in place).
//!
//! Ignored by default: it needs a GL display. Run it with:
//!
//! ```sh
//! cargo test -p functor-runtime-desktop --test golden -- --ignored --nocapture
//! ```
//!
//! Goldens are renderer/display-specific (GPU, driver, HiDPI scale). To
//! regenerate the native references on your machine, run the same scenarios with
//! `--capture-frame examples/<sample>/golden/<name>.png` (the runner invocation
//! below, pointed at the golden path instead of a temp file).

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

// A pixel "differs" if any channel is off by more than this (0-255). Absorbs
// minor antialiasing/driver wobble between runs on the same machine.
const TOLERANCE: u8 = 16;
// Allow this fraction of pixels to exceed the tolerance before failing. (The
// wasm harness uses the analogous `maxDiffPixelRatio` in playwright.config.mjs.)
const MAX_DIFF_FRACTION: f64 = 0.01;

/// One golden scenario, deserialized from `golden-scenarios.json`. Mirrors the
/// shape consumed by the wasm Playwright harness.
#[derive(Debug, Deserialize)]
struct Scenario {
    /// Reference-image basename (native: `examples/<sample>/golden/<name>.png`).
    name: String,
    /// Sample directory under `examples/`.
    sample: String,
    /// Frame time to pin (seconds) for a deterministic pose.
    #[serde(rename = "fixedTime")]
    fixed_time: f64,
    /// Optional debug-render mode (e.g. `"normals"`); `None` = default shading.
    #[serde(rename = "debugRender")]
    debug_render: Option<String>,
    /// Harnesses that run this scenario (`"native"` / `"wasm"`).
    targets: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Manifest {
    scenarios: Vec<Scenario>,
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("resolve repo root")
}

fn load_scenarios() -> Vec<Scenario> {
    let manifest_path = repo_root().join("golden-scenarios.json");
    let raw = std::fs::read_to_string(&manifest_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", manifest_path.display()));
    let manifest: Manifest = serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("parse {}: {e}", manifest_path.display()));
    manifest.scenarios
}

/// Path to the `functor` CLI binary, next to this test binary
/// (`target/<profile>/functor`, alongside `target/<profile>/deps/golden-*`).
/// Post-E3 there is a single binary: the golden captures drive it via
/// `functor ... run native`, so the CLI must be built first
/// (`cargo build --bin functor`; `npm run test:golden` does this).
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

/// Render `scenario`'s sample at its fixed time (plus any debug-render mode),
/// capture to a temp PNG, and assert it matches the committed
/// `examples/<sample>/golden/<name>.png` within tolerance.
fn assert_scenario_matches(scenario: &Scenario) {
    let sample_dir = repo_root().join("examples").join(&scenario.sample);

    let out = std::env::temp_dir().join(format!("functor-golden-{}.png", scenario.name));
    let _ = std::fs::remove_file(&out);

    // `functor -d <sample_dir> run native` interprets the sample's Functor Lang entry
    // (from its functor.json) in place — no build step — and forwards the
    // capture flags after `--` to the in-process desktop run loop.
    let fixed_time = scenario.fixed_time.to_string();
    let mut args = vec![
        "-d",
        sample_dir.to_str().unwrap(),
        "run",
        "native",
        "--",
        "--fixed-time",
        &fixed_time,
        "--capture-frame",
        out.to_str().unwrap(),
        "--capture-time",
        "1.0",
    ];
    if let Some(mode) = &scenario.debug_render {
        args.extend_from_slice(&["--debug-render", mode]);
    }

    let status = Command::new(functor_bin())
        .args(&args)
        .status()
        .expect("spawn functor");
    assert!(
        status.success(),
        "functor exited with {status} for scenario '{}'",
        scenario.name
    );

    let golden = sample_dir
        .join("golden")
        .join(format!("{}.png", scenario.name));
    assert_images_match(&out, &golden);
}

fn assert_images_match(actual_path: &Path, golden_path: &Path) {
    let actual = image::open(actual_path)
        .expect("open captured png")
        .to_rgba8();
    let golden = image::open(golden_path)
        .unwrap_or_else(|e| panic!("open golden {}: {e}", golden_path.display()))
        .to_rgba8();

    assert_eq!(
        actual.dimensions(),
        golden.dimensions(),
        "dimensions differ: captured {:?} vs golden {:?} — goldens are display-specific; \
         regenerate them on this machine",
        actual.dimensions(),
        golden.dimensions()
    );

    let differing = actual
        .pixels()
        .zip(golden.pixels())
        .filter(|(a, g)| {
            (0..4)
                .map(|i| (a[i] as i16 - g[i] as i16).abs() as u8)
                .max()
                .unwrap()
                > TOLERANCE
        })
        .count();
    let total = (actual.width() * actual.height()) as f64;
    let fraction = differing as f64 / total;
    println!(
        "golden diff ({}): {differing} / {total} pixels exceed tolerance ({:.3}%)",
        golden_path.file_name().unwrap().to_string_lossy(),
        fraction * 100.0
    );
    assert!(
        fraction <= MAX_DIFF_FRACTION,
        "rendering drifted from {}: {:.3}% of pixels exceed tolerance (max {:.3}%)",
        golden_path.display(),
        fraction * 100.0,
        MAX_DIFF_FRACTION * 100.0
    );
}

#[test]
#[ignore = "needs a GL display; run with --ignored"]
fn native_scenarios_match_golden() {
    let scenarios: Vec<_> = load_scenarios()
        .into_iter()
        .filter(|s| s.targets.iter().any(|t| t == "native"))
        .collect();
    assert!(
        !scenarios.is_empty(),
        "no native golden scenarios in manifest"
    );

    for scenario in &scenarios {
        println!("--- golden scenario: {} ---", scenario.name);
        assert_scenario_matches(scenario);
    }
}
