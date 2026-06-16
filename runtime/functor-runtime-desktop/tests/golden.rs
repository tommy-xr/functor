//! Golden-image regression tests for the `hello` sample.
//!
//! Each test renders the sample at a fixed frame time (so the pose is
//! deterministic) via `functor-runner --fixed-time`, captures the framebuffer
//! to a PNG, and compares it to a committed reference with a small tolerance.
//! `hello_matches_golden` covers default shading; `hello_normals_matches_golden`
//! covers the `--debug-render normals` view (the regression guard for vertex
//! normals + the normal-debug shaders).
//!
//! Ignored by default: they need a GL display and the game dylib built first
//! (`functor build native`). Run them with:
//!
//! ```sh
//! ./target/debug/functor -d examples/hello build native
//! cargo test -p functor-runtime-desktop --test golden -- --ignored --nocapture
//! ```
//!
//! Goldens are renderer/display-specific (GPU, driver, HiDPI scale). To
//! regenerate the references on your machine:
//!
//! ```sh
//! cd examples/hello
//! ../../target/debug/functor-runner --game-path build-native/target/debug/libgame_native.dylib \
//!   --fixed-time 2.0 --capture-frame golden/hello-t2.png --capture-time 1.0
//! ../../target/debug/functor-runner --game-path build-native/target/debug/libgame_native.dylib \
//!   --fixed-time 2.0 --debug-render normals --capture-frame golden/hello-normals-t2.png --capture-time 1.0
//! ```

use std::path::{Path, PathBuf};
use std::process::Command;

// A pixel "differs" if any channel is off by more than this (0-255). Absorbs
// minor antialiasing/driver wobble between runs on the same machine.
const TOLERANCE: u8 = 16;
// Allow this fraction of pixels to exceed the tolerance before failing.
const MAX_DIFF_FRACTION: f64 = 0.01;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("resolve repo root")
}

/// Render `hello` at a fixed time (plus any `extra_args`, e.g.
/// `--debug-render normals`), capture to `out_name`, and assert it matches the
/// committed `golden/<golden_name>` within tolerance.
fn assert_hello_matches(out_name: &str, golden_name: &str, extra_args: &[&str]) {
    let hello = repo_root().join("examples/hello");
    let dylib = format!(
        "{}game_native{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    let dylib_rel = format!("build-native/target/debug/{}", dylib);
    assert!(
        hello.join(&dylib_rel).exists(),
        "game dylib not found at {} — run `functor build native` first",
        hello.join(&dylib_rel).display()
    );

    let out = std::env::temp_dir().join(out_name);
    let _ = std::fs::remove_file(&out);

    let mut args = vec![
        "--game-path",
        &dylib_rel,
        "--fixed-time",
        "2.0",
        "--capture-frame",
        out.to_str().unwrap(),
        "--capture-time",
        "1.0",
    ];
    args.extend_from_slice(extra_args);

    let status = Command::new(env!("CARGO_BIN_EXE_functor-runner"))
        .current_dir(&hello)
        .args(&args)
        .status()
        .expect("spawn functor-runner");
    assert!(status.success(), "functor-runner exited with {status}");

    assert_images_match(&out, &hello.join("golden").join(golden_name));
}

fn assert_images_match(actual_path: &Path, golden_path: &Path) {
    let actual = image::open(actual_path).expect("open captured png").to_rgba8();
    let golden = image::open(golden_path)
        .unwrap_or_else(|e| panic!("open golden {}: {e}", golden_path.display()))
        .to_rgba8();

    assert_eq!(
        actual.dimensions(),
        golden.dimensions(),
        "dimensions differ: captured {:?} vs golden {:?} — goldens are display-specific; \
         regenerate with the commands in tests/golden.rs",
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
#[ignore = "needs a GL display and a built game dylib; run after `functor build native` with --ignored"]
fn hello_matches_golden() {
    assert_hello_matches("functor-golden-hello.png", "hello-t2.png", &[]);
}

#[test]
#[ignore = "needs a GL display and a built game dylib; run after `functor build native` with --ignored"]
fn hello_normals_matches_golden() {
    assert_hello_matches(
        "functor-golden-hello-normals.png",
        "hello-normals-t2.png",
        &["--debug-render", "normals"],
    );
}
