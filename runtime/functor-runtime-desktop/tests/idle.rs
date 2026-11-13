//! A paused hidden session renders only when something can change its
//! picture: a quiet second, reads and captures included, renders nothing,
//! and each wake source — a step, a seek, the extrapolation's fade, a stream
//! client, a project edit — brings its frame promptly.
//!
//! Drives the `functor` CLI next to this test binary (`cargo build --bin
//! functor` first; skipped without it). Needs a GL display, so it is ignored
//! by default:
//!
//! ```sh
//! cargo test -p functor-runtime-desktop --test idle -- --include-ignored
//! ```

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The CLI next to this test binary; `None` (the test skips) before it is built.
fn functor_bin() -> Option<PathBuf> {
    let exe = std::env::current_exe().expect("locate test binary");
    let target_dir = exe
        .parent()
        .and_then(|deps| deps.parent())
        .expect("target/<profile>");
    let bin = target_dir.join(if cfg!(windows) {
        "functor.exe"
    } else {
        "functor"
    });
    bin.exists().then_some(bin).or_else(|| {
        eprintln!("SKIPPED: build the CLI first (`cargo build --bin functor`)");
        None
    })
}

/// A game session on an OS-assigned debug port, killed on drop.
struct Session {
    child: Child,
    port: u16,
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn launch(functor: &Path, project: &Path) -> Session {
    let mut child = Command::new(functor)
        .arg("-d")
        .arg(project)
        .args(["run", "native", "--hidden", "--paused", "--debug-port", "0"])
        .args(["--capture-size", "320x180"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn functor");
    let stderr = BufReader::new(child.stderr.take().unwrap());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in stderr.lines().map_while(Result::ok) {
            if let Some(address) = line.split("listening on http://").nth(1) {
                let _ = tx.send(address.rsplit(':').next().unwrap().trim().parse::<u16>().unwrap());
            }
        }
    });
    let port = rx.recv_timeout(Duration::from_secs(60)).expect("debug server port");
    Session { child, port }
}

fn request(port: u16, method: &str, path: &str, body: &str) -> TcpStream {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    stream
}

/// The response body; panics on a non-200.
fn call(port: u16, method: &str, path: &str, body: &str) -> Vec<u8> {
    let mut response = Vec::new();
    request(port, method, path, body).read_to_end(&mut response).unwrap();
    let split = response.windows(4).position(|w| w == b"\r\n\r\n").expect("a head");
    let head = String::from_utf8_lossy(&response[..split]);
    assert!(head.starts_with("HTTP/1.1 200"), "{method} {path}: {head}");
    response[split + 4..].to_vec()
}

fn state(port: u16) -> serde_json::Value {
    serde_json::from_slice(&call(port, "GET", "/state?model=0", "")).unwrap()
}

fn rendered(port: u16) -> u64 {
    state(port)["rendered_frames"].as_u64().unwrap()
}

/// Frames rendered over `wait`.
fn rendered_over(port: u16, wait: Duration) -> u64 {
    let before = rendered(port);
    std::thread::sleep(wait);
    rendered(port) - before
}

/// `X-Rendered-Frame` and arrival of each `/stream` part, read on a thread.
fn stream_parts(port: u16) -> Arc<Mutex<Vec<(Instant, u64)>>> {
    let parts = Arc::new(Mutex::new(Vec::new()));
    let sink = parts.clone();
    let mut reader = BufReader::new(request(port, "GET", "/stream?w=160", ""));
    std::thread::spawn(move || loop {
        let mut head = String::new();
        while !head.ends_with("\r\n\r\n") {
            if reader.read_line(&mut head).unwrap_or(0) == 0 {
                return;
            }
        }
        let header = |name: &str| {
            head.lines()
                .find_map(|line| line.strip_prefix(name)?.strip_prefix(": "))
                .map(|value| value.parse::<u64>().unwrap())
        };
        let (Some(length), Some(frame)) = (header("Content-Length"), header("X-Rendered-Frame")) else {
            continue; // the response head
        };
        let mut body = vec![0; length as usize + 2];
        if reader.read_exact(&mut body).is_err() {
            return;
        }
        sink.lock().unwrap().push((Instant::now(), frame));
    });
    parts
}

/// Waits up to `within` for a part numbered above `after`.
fn part_after(parts: &Mutex<Vec<(Instant, u64)>>, after: u64, within: Duration) -> Option<u64> {
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        if let Some(&(_, frame)) = parts.lock().unwrap().iter().find(|(_, frame)| *frame > after) {
            return Some(frame);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    None
}

#[test]
#[ignore = "needs a GL display"]
fn a_paused_hidden_session_sleeps_until_its_picture_can_change() {
    let Some(functor) = functor_bin() else {
        return;
    };
    // A copy, so the edit below leaves the example alone.
    let project = std::env::temp_dir().join(format!("functor-idle-{}", std::process::id()));
    std::fs::create_dir_all(&project).unwrap();
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/primitives");
    for file in ["functor.json", "game.fun"] {
        std::fs::copy(example.join(file), project.join(file)).unwrap();
    }
    let session = launch(&functor, &project);
    let port = session.port;
    state(port);
    std::thread::sleep(Duration::from_secs(1));

    let quiet = rendered_over(port, Duration::from_secs(1));
    assert_eq!(quiet, 0, "a quiet second renders nothing");
    let before = rendered(port);
    for _ in 0..10 {
        state(port);
        call(port, "POST", "/capture", "");
    }
    assert_eq!(rendered(port), before, "reads and captures are answered from the frame shown");

    // A step shows its frame, and a seek back the earlier one: each capture
    // is answered on a frame rendered after the request before it.
    call(port, "POST", "/time", r#"{"type":"advance","dts":0.016}"#);
    let frame = state(port)["frame"].as_u64().unwrap();
    let earlier = call(port, "POST", "/capture", "");
    call(port, "POST", "/time", r#"{"type":"advance","dts":0.016}"#);
    assert_eq!(state(port)["frame"].as_u64(), Some(frame + 1));
    assert_ne!(call(port, "POST", "/capture", ""), earlier, "a step changes the picture");
    call(port, "POST", "/seek", &format!(r#"{{"frame":{frame}}}"#));
    assert_eq!(call(port, "POST", "/capture", ""), earlier, "a seek back shows the earlier picture");

    // The extrapolation fades in on its own frames, then the session sleeps.
    call(port, "POST", "/preview", r#"{"extrapolate":true}"#);
    let fade = rendered_over(port, Duration::from_millis(500));
    assert!(fade >= 8, "the fade renders, got {fade} frames");
    let settled = rendered_over(port, Duration::from_secs(1));
    assert_eq!(settled, 0, "after the fade a quiet second renders nothing");
    call(port, "POST", "/preview", r#"{"extrapolate":false}"#);
    std::thread::sleep(Duration::from_millis(500));

    // A stream client gets the current picture, then nothing until a step.
    let parts = stream_parts(port);
    let first = part_after(&parts, 0, Duration::from_secs(2)).expect("a first part");
    std::thread::sleep(Duration::from_secs(1));
    let sent = parts.lock().unwrap().len();
    // The first, sent again so a browser shows it.
    assert!(sent <= 2, "a quiet stream sent {sent} parts");
    // A client joining later gets it too, though it is no longer fresh.
    let late = stream_parts(port);
    assert!(part_after(&late, 0, Duration::from_secs(1)).is_some(), "a later client gets a part");
    call(port, "POST", "/time", r#"{"type":"advance","dts":0.016}"#);
    assert!(
        part_after(&parts, first, Duration::from_millis(500)).is_some(),
        "a step streams its frame"
    );

    // A saved edit reloads without any request to wake it.
    std::thread::sleep(Duration::from_millis(500));
    let before = state(port);
    let mut source = std::fs::read_to_string(project.join("game.fun")).unwrap();
    source.push_str("\n// edited\n");
    std::fs::write(project.join("game.fun"), source).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let after = state(port);
    let revision = |s: &serde_json::Value| s["clock"]["program_revision"].as_u64().unwrap();
    assert_eq!(revision(&after), revision(&before) + 1, "the edit reloaded");
    let woke = after["rendered_frames"].as_u64().unwrap() - before["rendered_frames"].as_u64().unwrap();
    assert!(woke >= 1, "the edit woke the session");

    // Resumed after a long wait, the clock goes on from where it was rather
    // than catching up on the wait (up to 8 steps at once).
    std::thread::sleep(Duration::from_secs(1));
    let paused_at = state(port)["frame"].as_u64().unwrap();
    call(port, "POST", "/time", r#"{"type":"resume"}"#);
    let resumed = state(port)["frame"].as_u64().unwrap() - paused_at;
    assert!(resumed <= 3, "resuming ran {resumed} steps at once");

    drop(session);
    let _ = std::fs::remove_dir_all(&project);
}
