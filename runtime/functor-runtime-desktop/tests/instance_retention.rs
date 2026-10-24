//! Retained instance buffers, end to end: a `Scene.instanced` list kept in
//! the model (or rebuilt with identical content) uploads once, so `GET
//! /state`'s `instance_uploads` reads 0 per frame afterwards; a list whose
//! content changes every frame still uploads every frame.
//!
//! Ignored by default: it needs a GL display (the hidden window renders).
//!
//! ```sh
//! cargo build --bin functor
//! cargo test -p functor-runtime-desktop --test instance_retention -- --ignored --nocapture
//! ```

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Path to the `functor` CLI binary, next to this test binary.
fn functor_bin() -> PathBuf {
    let exe = std::env::current_exe().expect("locate test binary");
    let target_dir = exe
        .parent()
        .and_then(|deps| deps.parent())
        .expect("target/<profile> dir");
    let bin = target_dir.join(if cfg!(windows) {
        "functor.exe"
    } else {
        "functor"
    });
    assert!(bin.exists(), "build the CLI first: {}", bin.display());
    bin
}

/// A 50×50 cube grid; `trees` names the instance-list expression `draw` uses.
fn game(trees: &str) -> String {
    format!(
        r#"
let grid = (offset: float) =>
  List.range(50) |> List.concatMap((ix) =>
    List.range(50) |> List.map((iz) =>
      Instance.at(Vec3.make(ix - 25.0 + offset, 0.0, iz - 25.0)) |> Instance.scale(0.4)))

let init = {{ trees: grid(0.0) }}
let tick = (model, dt: float, tts: float) => model
let template = Scene.cube() |> Scene.lit(Color.rgb(0.3, 0.8, 0.4))
let draw = (model, tts: float) =>
  Frame.create(
    Camera3D.lookAt(Vec3.make(0.0, 30.0, -40.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.instanced({trees}, template))
"#
    )
}

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

fn launch(name: &str, trees: &str) -> Session {
    let dir = std::env::temp_dir().join(format!("functor-instance-retention-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("functor.json"),
        r#"{"language": "functor-lang", "entry": "game.fun"}"#,
    )
    .unwrap();
    std::fs::write(dir.join("game.fun"), game(trees)).unwrap();
    let mut child = Command::new(functor_bin())
        .args([
            "-d",
            dir.to_str().unwrap(),
            "run",
            "native",
            "--hidden",
            "--debug-port",
            "0",
        ])
        .stderr(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .expect("spawn functor");
    // The stderr line reports the actual bound port.
    let stderr = BufReader::new(child.stderr.take().unwrap());
    let mut port = None;
    for line in stderr.lines() {
        let line = line.unwrap();
        if let Some(rest) = line.split("listening on http://127.0.0.1:").nth(1) {
            port = rest
                .trim_end_matches(|c: char| !c.is_ascii_digit())
                .parse()
                .ok();
            break;
        }
    }
    Session {
        child,
        port: port.expect("debug server port on stderr"),
    }
}

fn state(port: u16) -> serde_json::Value {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    write!(stream, "GET /state HTTP/1.0\r\nHost: localhost\r\n\r\n").unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let body = response.split("\r\n\r\n").nth(1).expect("http body");
    serde_json::from_str(body).expect("state json")
}

/// The per-frame upload counts of `samples` reads, once 30 frames have run.
fn uploads_per_frame(session: &Session, samples: usize) -> Vec<u64> {
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut counts = Vec::new();
    let mut last_frame = 0;
    while counts.len() < samples {
        assert!(Instant::now() < deadline, "the session stopped advancing");
        let state = state(session.port);
        let frame = state["frame"].as_u64().unwrap();
        if frame >= 30 && frame != last_frame {
            last_frame = frame;
            counts.push(state["instance_uploads"]["count"].as_u64().unwrap());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    counts
}

#[test]
#[ignore = "needs a GL display. Run with --ignored"]
fn retained_instance_lists_upload_once() {
    // Kept in the model: the same list `Rc` → the same buffer, no upload.
    let held = uploads_per_frame(&launch("held", "model.trees"), 5);
    // Rebuilt in draw with identical content: the content hash matches.
    let rebuilt = uploads_per_frame(&launch("rebuilt", "grid(0.0)"), 5);
    // Rebuilt with content that changes every frame: uploads every frame.
    let moving = uploads_per_frame(&launch("moving", "grid(tts * 0.01)"), 5);
    println!("uploads/frame — held {held:?}, rebuilt {rebuilt:?}, moving {moving:?}");
    assert!(held.iter().all(|n| *n == 0), "held: {held:?}");
    assert!(rebuilt.iter().all(|n| *n == 0), "rebuilt: {rebuilt:?}");
    assert!(moving.iter().all(|n| *n >= 1), "moving: {moving:?}");
}
