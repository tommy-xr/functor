//! Golden-image regression tests for the sample games.
//!
//! The scenarios are defined once in `golden-scenarios.json` at the repo root
//! and **shared** with the wasm Playwright test (`e2e/golden-wasm.spec.mjs`), so
//! a scenario added there is validated on both targets. Each scenario renders a
//! sample at a fixed frame time (so the pose is deterministic) via
//! `functor-runner --fixed-time`, optionally with a `--debug-render` mode,
//! captures the framebuffer to a PNG, and compares it to a committed reference
//! with a small tolerance. This test runs every scenario whose `targets`
//! includes `"native"`.
//!
//! The runner is producer-agnostic: an MLE sample (`functor.json` with
//! `"language": "mle"`) renders via `--mle --game-path <entry>`; an F# sample
//! renders from its game dylib (`--game-path <dylib>`). MLE samples need no
//! build step (the runner interprets the `.mle` in place); F# samples need
//! their dylib built first.
//!
//! Ignored by default: it needs a GL display (and, for any F# scenario, its
//! game dylib). Run it with:
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

/// If `sample_dir` is an MLE project (`functor.json` has `"language": "mle"`),
/// return its entry file (default `game.mle`); otherwise `None` (an F# sample
/// rendered from its game dylib). The runner is producer-agnostic — MLE and F#
/// render the same protocol `Frame` through the same shell — so a scenario just
/// needs to point the runner at the right producer.
fn mle_entry(sample_dir: &Path) -> Option<String> {
    let content = std::fs::read_to_string(sample_dir.join("functor.json")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;
    if json.get("language").and_then(|v| v.as_str()) != Some("mle") {
        return None;
    }
    Some(
        json.get("entry")
            .and_then(|v| v.as_str())
            .unwrap_or("game.mle")
            .to_string(),
    )
}

/// Render `scenario`'s sample at its fixed time (plus any debug-render mode),
/// capture to a temp PNG, and assert it matches the committed
/// `examples/<sample>/golden/<name>.png` within tolerance.
fn assert_scenario_matches(scenario: &Scenario) {
    let sample_dir = repo_root().join("examples").join(&scenario.sample);

    // Point the runner at the sample's producer: `--mle --game-path <entry>`
    // for an MLE project, or `--game-path <dylib>` for an F# game dylib.
    let (producer_args, game_path): (Vec<&str>, String) = match mle_entry(&sample_dir) {
        Some(entry) => (vec!["--mle"], entry),
        None => {
            let dylib = format!(
                "{}game_native{}",
                std::env::consts::DLL_PREFIX,
                std::env::consts::DLL_SUFFIX
            );
            let dylib_rel = format!("build-native/target/debug/{}", dylib);
            assert!(
                sample_dir.join(&dylib_rel).exists(),
                "game dylib not found at {} — run `functor -d examples/{} build native` first",
                sample_dir.join(&dylib_rel).display(),
                scenario.sample
            );
            (vec![], dylib_rel)
        }
    };

    let out = std::env::temp_dir().join(format!("functor-golden-{}.png", scenario.name));
    let _ = std::fs::remove_file(&out);

    let fixed_time = scenario.fixed_time.to_string();
    let mut args = producer_args;
    args.extend_from_slice(&[
        "--game-path",
        &game_path,
        "--fixed-time",
        &fixed_time,
        "--capture-frame",
        out.to_str().unwrap(),
        "--capture-time",
        "1.0",
    ]);
    if let Some(mode) = &scenario.debug_render {
        args.extend_from_slice(&["--debug-render", mode]);
    }

    let status = Command::new(env!("CARGO_BIN_EXE_functor-runner"))
        .current_dir(&sample_dir)
        .args(&args)
        .status()
        .expect("spawn functor-runner");
    assert!(
        status.success(),
        "functor-runner exited with {status} for scenario '{}'",
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
#[ignore = "needs a GL display and built game dylibs; run after `functor build native` with --ignored"]
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
