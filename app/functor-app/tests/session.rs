//! A real game session: `--start-session --no-gpu` against `examples/counter`.
//! Needs a built `functor` in `FUNCTOR_BIN` (CI sets it after building the
//! CLI); skips without one.

use std::path::PathBuf;
use std::process::{Command, Stdio};

#[test]
fn start_session_polls_the_game_and_leaves_no_child() {
    let Some(functor) = std::env::var_os("FUNCTOR_BIN")
        .map(PathBuf::from)
        .filter(|p| p.is_file())
    else {
        eprintln!("SKIPPED: set FUNCTOR_BIN to a built `functor` to run a real session");
        return;
    };
    let counter = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/counter");
    let out = Command::new(env!("CARGO_BIN_EXE_functor-app"))
        .args([
            "--headless",
            "--print",
            "--start-session",
            "--no-gpu",
            "-d",
            counter,
        ])
        .arg("--functor")
        .arg(&functor)
        .output()
        .unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{text}{stderr}");

    // `session: running pid 4242 http://127.0.0.1:52011 frame 30`
    let line = text.lines().find(|l| l.starts_with("session: ")).unwrap();
    let words: Vec<&str> = line.split(' ').collect();
    assert_eq!(words[1..3], ["running", "pid"], "{line}");
    assert!(words[4].starts_with("http://127.0.0.1:"), "{line}");
    assert_eq!(words[5], "frame", "{line}");
    words[6].parse::<u64>().expect(line);
    let pid: u32 = words[3].parse().unwrap();
    assert!(!alive(pid), "functor child {pid} outlived functor-app");
}

#[cfg(unix)]
fn alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success()
}

#[cfg(windows)]
fn alive(pid: u32) -> bool {
    let filter = format!("PID eq {pid}");
    let out = Command::new("tasklist")
        .args(["/FI", &filter, "/NH"])
        .stderr(Stdio::null())
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).contains(&format!(" {pid} "))
}
