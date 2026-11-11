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
    let home = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_functor-app"))
        .env("FUNCTOR_APP_HOME", home.path())
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

/// SIGTERM while a session is starting kills the child. The fake `functor`
/// never listens, so the app is still waiting when the signal lands.
#[cfg(unix)]
#[test]
fn sigterm_kills_the_child() {
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};

    let dir = tempfile::tempdir().unwrap();
    let fake = dir.path().join("functor");
    std::fs::write(&fake, "#!/bin/sh\nexec sleep 60\n").unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let counter = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/counter");
    let app = Command::new(env!("CARGO_BIN_EXE_functor-app"))
        .env("FUNCTOR_APP_HOME", dir.path())
        .args(["--headless", "--print", "--start-session", "-d", counter])
        .arg("--functor")
        .arg(&fake)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(30);
    let child = loop {
        let out = Command::new("pgrep")
            .args(["-P", &app.id().to_string()])
            .output()
            .unwrap();
        if let Some(pid) = String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .next()
        {
            break pid.parse::<u32>().unwrap();
        }
        assert!(
            Instant::now() < deadline,
            "functor-app never spawned its child"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    let term = Command::new("kill")
        .args(["-TERM", &app.id().to_string()])
        .status()
        .unwrap();
    assert!(term.success());
    let out = app.wait_with_output().unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert_eq!(out.status.code(), Some(143), "{text}");
    assert!(
        text.contains(&format!("\nsession: starting pid {child}\n")),
        "{text}"
    );
    assert!(
        !alive(child),
        "child {child} outlived functor-app's SIGTERM"
    );
}
