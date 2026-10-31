//! Golden-image regression tests for the sample games.
//!
//! The scenarios live in each example's `examples/<sample>/goldens.json`
//! (docs/goldens.md) and are **shared** with the wasm Playwright test
//! (`e2e/golden-wasm.spec.mjs`), so a scenario added there is validated on both
//! targets. Each scenario renders a
//! sample at a fixed frame time (so the pose is deterministic) via
//! `functor ... run native --fixed-time` — or, for a state only input reaches,
//! at a fixed frame of an input script (native only) — optionally with a
//! `--debug-render` mode, captures the framebuffer to a PNG, and compares it
//! to a reference with a small tolerance. This test runs every scenario whose `targets`
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
//! Every scenario also runs with `--pipelined` (the game logic on its own sim
//! thread), which must render the same image. A second test drives scripted
//! samples under both modes and requires byte-identical captures: the fixed
//! step sim must not observe which mode it runs in.
//!
//! Goldens are renderer/display-specific (GPU, driver, HiDPI scale). To
//! regenerate the native references on your machine, run the same scenarios with
//! `--capture-frame examples/<sample>/golden/<name>.png --capture-size 1600x1200`
//! (the runner invocation below, pointed at the golden path instead of a temp
//! file). The size must be pinned: without it the framebuffer follows the
//! display's HiDPI scale and the references only match the machine that made
//! them.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

// A pixel "differs" if any channel is off by more than this (0-255). Absorbs
// minor antialiasing/driver wobble between runs on the same machine.
/// Framebuffer every golden is captured at. Changing this invalidates every
/// committed reference, so treat it as a format version.
const GOLDEN_CAPTURE_SIZE: &str = "1600x1200";
const TOLERANCE: u8 = 16;
// Allow this fraction of pixels to exceed the tolerance before failing. (The
// wasm harness uses the analogous `maxDiffPixelRatio` in playwright.config.mjs.)
const MAX_DIFF_FRACTION: f64 = 0.01;

/// One golden scenario, deserialized from an `examples/<sample>/goldens.json`.
/// Mirrors the shape the wasm harness validates: a new field must also be added
/// to `KEYS` in `scripts/goldens.mjs`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Scenario {
    /// Reference-image basename (native: `examples/<sample>/golden/<name>.png`).
    name: String,
    /// Sample directory under `examples/`: the directory holding the file.
    #[serde(skip)]
    sample: String,
    /// Frame time to pin (seconds) for a deterministic pose; absent for a
    /// scripted scenario.
    #[serde(rename = "fixedTime", default)]
    fixed_time: Option<f64>,
    /// Optional debug-render mode (e.g. `"normals"`); `None` = default shading.
    #[serde(rename = "debugRender")]
    debug_render: Option<String>,
    /// Harnesses that run this scenario (`"native"` / `"wasm"`).
    targets: Vec<String>,
    /// Native only: drive the sample with this input script (relative to the
    /// sample) and capture at `captureAtFrame` instead of at `fixedTime` —
    /// for states only input reaches, like a toggled mode.
    #[serde(rename = "inputScript", default)]
    input_script: Option<String>,
    #[serde(rename = "captureAtFrame", default)]
    capture_at_frame: Option<u64>,
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("resolve repo root")
}

/// Every `examples/*/goldens.json` scenario, ordered by sample directory then
/// file order. Panics on a duplicate name or the retired root manifest.
fn load_scenarios() -> Vec<Scenario> {
    let root = repo_root();
    assert!(
        !root.join("golden-scenarios.json").exists(),
        "golden-scenarios.json is retired: move its scenarios to examples/<sample>/goldens.json"
    );
    let mut files: Vec<PathBuf> = std::fs::read_dir(root.join("examples"))
        .expect("read examples/")
        .map(|entry| entry.expect("examples/ entry").path().join("goldens.json"))
        .filter(|path| path.is_file())
        .collect();
    files.sort();

    let mut scenarios: Vec<Scenario> = Vec::new();
    for path in files {
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let list: Vec<Scenario> = serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
        let sample = path.parent().unwrap().file_name().unwrap().to_string_lossy();
        for mut scenario in list {
            assert!(
                !scenarios.iter().any(|s| s.name == scenario.name),
                "duplicate golden scenario name '{}' ({})",
                scenario.name,
                path.display()
            );
            scenario.sample = sample.to_string();
            scenarios.push(scenario);
        }
    }
    scenarios
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

/// Render `scenario`'s sample at its fixed time or scripted frame (plus any
/// debug-render mode),
/// capture to a temp PNG, and assert it matches the committed
/// `examples/<sample>/golden/<name>.png` within tolerance.
fn assert_scenario_matches(scenario: &Scenario, extra: &[&str]) {
    let sample_dir = repo_root().join("examples").join(&scenario.sample);

    let out = std::env::temp_dir().join(format!(
        "functor-golden-{}{}.png",
        scenario.name,
        extra.join("")
    ));
    let _ = std::fs::remove_file(&out);

    // `functor -d <sample_dir> run native` interprets the sample's Functor Lang entry
    // (from its functor.json) in place — no build step — and forwards the
    // capture flags after `--` to the in-process desktop run loop.
    let (fixed_time, script, capture_frame) = match (
        scenario.fixed_time,
        &scenario.input_script,
        scenario.capture_at_frame,
    ) {
        (Some(time), None, None) => (time.to_string(), None, String::new()),
        (None, Some(script), Some(frame)) => {
            (String::new(), Some(sample_dir.join(script)), frame.to_string())
        }
        _ => panic!(
            "scenario '{}': set fixedTime, or inputScript with captureAtFrame",
            scenario.name
        ),
    };
    let mut args = vec![
        "-d",
        sample_dir.to_str().unwrap(),
        "run",
        "native",
        "--",
        "--capture-frame",
        out.to_str().unwrap(),
        // Pin the framebuffer so a reference depends on the renderer, not on
        // the monitor. Without this the same scene captures at 800x600 on a 1x
        // display and 1600x1200 on a Retina one, and every golden fails on a
        // size mismatch the moment a machine is docked or undocked.
        "--capture-size",
        GOLDEN_CAPTURE_SIZE,
    ];
    match &script {
        Some(script) => args.extend_from_slice(&[
            "--input-script",
            script.to_str().unwrap(),
            "--capture-at-frame",
            &capture_frame,
        ]),
        None => args.extend_from_slice(&["--fixed-time", &fixed_time, "--capture-time", "1.0"]),
    }
    if let Some(mode) = &scenario.debug_render {
        args.extend_from_slice(&["--debug-render", mode]);
    }
    args.extend_from_slice(extra);

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

    // Inline, then with the game logic on its own sim thread.
    for extra in [&[][..], &["--pipelined"]] {
        for scenario in &scenarios {
            println!("--- golden scenario: {} {extra:?} ---", scenario.name);
            assert_scenario_matches(scenario, extra);
        }
    }
}

/// Scripted runs (`<sample>/<script>` captured at sim frame N) exercising
/// input, physics, terrain heightfield hydration, and audio effects.
const SCRIPTED: &[(&str, &str, u64)] = &[
    ("physics-controller", "ride-and-jump.input", 400),
    ("terrain", "launch-balls.input", 300),
    ("shooting-range", "firing.input", 120),
];

fn scripted_capture(sample: &str, script: &str, frame: u64, extra: &[&str]) -> Vec<u8> {
    let sample_dir = repo_root().join("examples").join(sample);
    let out = std::env::temp_dir().join(format!("functor-scripted-{sample}{}.png", extra.join("")));
    let _ = std::fs::remove_file(&out);
    let frame = frame.to_string();
    let script = sample_dir.join(script);
    let mut args = vec![
        "-d",
        sample_dir.to_str().unwrap(),
        "run",
        "native",
        "--",
        "--input-script",
        script.to_str().unwrap(),
        "--script-dt",
        "0.0166667",
        "--capture-frame",
        out.to_str().unwrap(),
        "--capture-at-frame",
        &frame,
        "--capture-size",
        "640x480",
    ];
    args.extend_from_slice(extra);
    let status = Command::new(functor_bin())
        .args(&args)
        .status()
        .expect("spawn functor");
    assert!(status.success(), "functor exited with {status} for '{sample}'");
    std::fs::read(&out).unwrap_or_else(|e| panic!("read {}: {e}", out.display()))
}

#[test]
#[ignore = "needs a GL display; run with --ignored"]
fn pipelined_scripted_captures_are_byte_identical() {
    for (sample, script, frame) in SCRIPTED {
        let inline = scripted_capture(sample, script, *frame, &[]);
        let pipelined = scripted_capture(sample, script, *frame, &["--pipelined"]);
        assert!(
            inline == pipelined,
            "'{sample}' frame {frame}: the --pipelined capture differs from inline"
        );
        println!("scripted {sample} @{frame}: identical ({} bytes)", inline.len());
    }
}
