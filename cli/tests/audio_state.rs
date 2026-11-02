//! `GET /state.audio` end to end through the real binary, headless (no audio
//! device, so every one-shot counts as dropped): a key press plays one; the
//! paused inspector, a rewind and quiet steps after it play none; the same
//! press again does.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const GAME: &str = r#"
let init = 0.0
let tick = (model, dt: float, tts: float) => model
let input = (model, key, isDown) =>
  match key == Key.E && isDown with
  | true => (model + 1.0, Effect.play(Asset.sound("beep.wav")))
  | false => model
let draw = (model, tts: float) =>
  Frame.create(Camera3D.lookAt(Vec3.make(0.0, 3.0, -6.0), Vec3.make(0.0, 0.0, 0.0)), Scene.cube())
"#;

/// The runtime, killed (and its project removed) on drop.
struct Session(Child, std::path::PathBuf);

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
        let _ = std::fs::remove_dir_all(&self.1);
    }
}

/// Start the game headless; its debug port.
fn launch() -> (Session, u16) {
    let dir = std::env::temp_dir().join(format!("functor-audio-state-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("functor.json"),
        r#"{"language":"functor-lang","entry":"game.fun"}"#,
    )
    .unwrap();
    std::fs::write(dir.join("game.fun"), GAME).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_functor"))
        .arg("-d")
        .arg(&dir)
        .args(["run", "native", "--headless", "--debug-port", "0"])
        .stderr(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let mut session = Session(child, dir);
    // The debug server prints its bound port; keep draining stderr after, or
    // the runtime's next log line hits a closed pipe.
    let mut lines = BufReader::new(session.0.stderr.take().unwrap()).lines();
    let port = lines
        .by_ref()
        .map_while(Result::ok)
        .find_map(|line| {
            let rest = line
                .split("listening on http://127.0.0.1:")
                .nth(1)?
                .to_string();
            rest.trim_end_matches(|c: char| !c.is_ascii_digit())
                .parse()
                .ok()
        })
        .expect("debug server port on stderr");
    std::thread::spawn(move || lines.for_each(drop));
    (session, port)
}

fn http(port: u16, method: &str, path: &str, body: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    write!(
        stream,
        "{method} {path} HTTP/1.0\r\nHost: localhost\r\nContent-Type: application/json\r\n\
Content-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let (head, body) = response.split_once("\r\n\r\n").unwrap();
    assert!(head.contains(" 200 "), "{method} {path}: {response}");
    body.to_string()
}

fn state(port: u16) -> serde_json::Value {
    serde_json::from_str(&http(port, "GET", "/state", "")).unwrap()
}

/// Run `n` paused steps and wait for them.
fn step(port: u16, n: u32) {
    http(
        port,
        "POST",
        "/time",
        &format!(r#"{{"type":"advance","dts":0.016,"frames":{n}}}"#),
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    while state(port)["pending_steps"].as_u64() != Some(0) {
        assert!(Instant::now() < deadline, "{n} steps never ran");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn press_e(port: u16) {
    http(
        port,
        "POST",
        "/input",
        r#"{"type":"key","key":"E","down":true}"#,
    );
    step(port, 1);
    http(
        port,
        "POST",
        "/input",
        r#"{"type":"key","key":"E","down":false}"#,
    );
    step(port, 1);
}

#[test]
fn state_audio_counts_one_shots_and_a_rewind_replays_none() {
    let (_session, port) = launch();
    http(port, "POST", "/time", r#"{"type":"set","tts":0}"#);
    step(port, 3);
    assert_eq!(
        state(port)["audio"],
        serde_json::json!({ "played": 0, "dropped": 0, "recent": [] })
    );

    press_e(port);
    step(port, 3);
    let s = state(port);
    assert_eq!(s["model"], 1.0);
    assert_eq!(
        s["audio"],
        serde_json::json!({ "played": 0, "dropped": 1, "recent": ["beep.wav"] })
    );

    // The paused inspector re-runs the last frame's handlers: no effects.
    http(port, "GET", "/trace", "");
    // Back to a frame after the press, then on without input.
    let frame = s["frame"].as_u64().unwrap();
    http(
        port,
        "POST",
        "/rewind",
        &format!(r#"{{"frame":{}}}"#, frame - 2),
    );
    step(port, 10);
    let s = state(port);
    assert_eq!(s["model"], 1.0, "rewound after the press");
    assert_eq!(
        s["audio"]["dropped"], 1,
        "no sound re-emitted: {}",
        s["audio"]
    );

    // Before the press, and the press again: one more.
    http(port, "POST", "/rewind", r#"{"frame":0}"#);
    press_e(port);
    let s = state(port);
    assert_eq!(s["model"], 1.0);
    assert_eq!(s["audio"]["dropped"], 2);
}
