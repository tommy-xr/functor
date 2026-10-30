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
fn a_failure_prints_a_replay_that_reproduces_it() {
    // The fuzzer's no-op reload appends `let fuzzReload0 = …`: a duplicate here.
    let dir = std::env::temp_dir().join(format!("functor-fuzz-broken-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("functor.json"), r#"{"language":"functor-lang","entry":"game.fun"}"#)
        .unwrap();
    let game = "let fuzzReload0 = 1.0\nlet init = 0.0\nlet tick = (model, dt, tts) => model + 1.0\n\
let draw = (model, tts) => Frame.create(Camera3D.lookAt(Vec3.make(0.0, 3.0, -6.0), Vec3.make(0.0, 0.0, 0.0)), Scene.cube())\n";
    std::fs::write(dir.join("game.fun"), game).unwrap();
    let run = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_functor"))
            .arg("-d")
            .arg(&dir)
            .arg("fuzz")
            .args(args)
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout).into_owned()
            + &String::from_utf8_lossy(&out.stderr);
        (out.status.success(), text)
    };
    let (ok, out) = run(&["--seeds", "1", "--ops", "40"]);
    assert!(!ok, "{out}");
    assert!(out.contains("Reload at op"), "{out}");
    let ops = out
        .lines()
        .find_map(|l| l.trim().strip_prefix("replay: "))
        .and_then(|l| l.split(" fuzz --replay '").nth(1))
        .map(|l| l.trim_end_matches('\''))
        .unwrap_or_else(|| panic!("no replay line: {out}"));
    assert!(ops.starts_with("reload") && !ops.contains(' '), "{ops}");
    let (ok, again) = run(&["--replay", ops]);
    assert!(!ok && again.contains("Reload at op 0"), "{again}");
    // The project is untouched: no saves, no regenerated files.
    let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
    files.sort();
    assert_eq!(files, ["functor.json", "game.fun"]);
    assert_eq!(std::fs::read_to_string(dir.join("game.fun")).unwrap(), game);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fuzz_rejects_an_unknown_op() {
    let err = fuzz("physics", &["--replay", "play:2:1 jump"]).unwrap_err();
    assert!(err.contains("unknown fuzz op `jump`"), "{err}");
}
