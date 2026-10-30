//! `functor fuzz` end to end through the real binary, on a tiny budget: a
//! clean run summarizes what it verified, a printed repro replays, and a bad
//! op list is an error.

use std::path::Path;
use std::process::Command;

/// Run `functor -d examples/<example> fuzz <args>`; its output either way.
fn fuzz(example: &str, args: &[&str]) -> Result<String, String> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples").join(example);
    let run = Command::new(env!("CARGO_BIN_EXE_functor"))
        .arg("-d")
        .arg(dir)
        .arg("fuzz")
        .args(args)
        .output()
        .unwrap();
    let text =
        String::from_utf8_lossy(&run.stdout).into_owned() + &String::from_utf8_lossy(&run.stderr);
    if run.status.success() {
        Ok(text)
    } else {
        Err(text)
    }
}

#[test]
fn fuzz_runs_seeds_and_summarizes() {
    let out = fuzz("platformer", &["--seeds", "1", "--ops", "15"]).unwrap();
    assert!(out.contains("keys ["), "{out}");
    assert!(out.contains("1 seed(s) x 15 ops"), "{out}");
    assert!(out.contains("frames recorded"), "{out}");
    assert!(!out.contains("frames recorded, 0 seeks"), "{out}");
}

#[test]
fn fuzz_replays_an_op_list() {
    let out = fuzz(
        "physics",
        &["--replay", "play:20:1 pause seek:back:5 resume play:3:2 rewind:oldest reload"],
    )
    .unwrap();
    assert!(out.contains("7 replayed ops"), "{out}");
}

#[test]
fn fuzz_rejects_an_unknown_op() {
    let err = fuzz("physics", &["--replay", "play:2:1 jump"]).unwrap_err();
    assert!(err.contains("unknown fuzz op `jump`"), "{err}");
}
