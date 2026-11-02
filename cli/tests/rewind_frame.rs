//! `/state.frame` and `/rewind` share one numbering, end to end through the
//! real binary (headless): rewinding to a frame `/state` reported restores
//! that frame's model, and `/state.frame` then reads that frame.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

// The model counts sim steps.
const GAME: &str = r#"
let init = 0.0
let tick = (model, dt: float, tts: float) => model + 1.0
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
    let dir = std::env::temp_dir().join(format!("functor-rewind-frame-{}", std::process::id()));
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

/// `/state`'s `(frame, model)`.
fn state(port: u16) -> (u64, f64) {
    let s: serde_json::Value = serde_json::from_str(&http(port, "GET", "/state", "")).unwrap();
    assert_eq!(s["pending_steps"], 0, "{s}");
    (s["frame"].as_u64().unwrap(), s["model"].as_f64().unwrap())
}

/// Run one paused step and wait for it.
fn step(port: u16) {
    http(port, "POST", "/time", r#"{"type":"advance","dts":0.016}"#);
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let s: serde_json::Value = serde_json::from_str(&http(port, "GET", "/state", "")).unwrap();
        if s["pending_steps"] == 0 {
            return;
        }
        assert!(Instant::now() < deadline, "the step never ran");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn rewind_lands_on_the_frame_state_reported() {
    let (_session, port) = launch();
    http(port, "POST", "/time", r#"{"type":"set","tts":0}"#);
    // Each step is one frame on, from the first.
    let mut seen = vec![state(port)];
    for _ in 0..6 {
        step(port);
        seen.push(state(port));
        let [.., (before, _), (after, _)] = seen[..] else {
            unreachable!()
        };
        assert_eq!(after, before + 1, "{seen:?}");
    }

    let (frame, model) = seen[3];
    let reply = http(port, "POST", "/rewind", &format!(r#"{{"frame":{frame}}}"#));
    assert_eq!(reply, format!("rewound to frame {frame}"));
    assert_eq!(
        state(port),
        (frame, model),
        "the frame /state reported, exactly"
    );
    step(port);
    assert_eq!(state(port), seen[4], "and on from there");

    // Frame 0 (before any step) isn't recorded: the oldest frame stands in.
    let reply = http(port, "POST", "/rewind", r#"{"frame":0}"#);
    assert_eq!(
        reply,
        "rewound to frame 1 (requested 0, clamped to the recorded window)"
    );
    assert_eq!(state(port).0, 1);
}
