//! `functor reads` through the real binary: each top-level def's inferred
//! read classes for `fixtures/reads`, pinned to `fixtures/reads/reads.golden`.
//! Regenerate with `UPDATE_GOLDENS=1 cargo test -p functor-cli --test reads`.

use std::path::Path;
use std::process::Command;

#[test]
fn reads_matches_the_golden_dump() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/reads");
    let run = Command::new(env!("CARGO_BIN_EXE_functor"))
        .arg("-d")
        .arg(&dir)
        .args(["--quiet", "reads"])
        .output()
        .unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    // The report, without the run's status lines (`✓ done in …`).
    let actual: String = String::from_utf8(run.stdout)
        .unwrap()
        .lines()
        .filter(|line| line.contains(" : "))
        .map(|line| format!("{line}\n"))
        .collect();
    let golden = dir.join("reads.golden");
    if std::env::var_os("UPDATE_GOLDENS").is_some() {
        std::fs::write(&golden, &actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&golden).unwrap();
    assert_eq!(
        actual, expected,
        "regenerate with UPDATE_GOLDENS=1 cargo test -p functor-cli --test reads"
    );
}
