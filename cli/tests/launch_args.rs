//! `--arg key=value` end to end through the real binary, headless: the game's
//! `Launch.args()` holds what `functor run native` was given, the last value
//! of a repeated key, and nothing when none is given.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

const GAME: &str = r#"
let init = Launch.args() |> Map.toList |> List.map((pair) => let (key, value) = pair in $"{key}={value}")
let tick = (model, dt: float, tts: float) => model
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

const HEADLESS: [&str; 3] = ["--headless", "--debug-port", "0"];

/// The model of the game run with `args` (which include [`HEADLESS`]).
fn model(name: &str, args: &[&str]) -> serde_json::Value {
    let dir = std::env::temp_dir().join(format!("functor-launch-args-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("functor.json"), r#"{"language":"functor-lang","entry":"game.fun"}"#).unwrap();
    std::fs::write(dir.join("game.fun"), GAME).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_functor"))
        .arg("-d")
        .arg(&dir)
        .args(["run", "native"])
        .args(args)
        .stderr(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let mut session = Session(child, dir);
    // The debug server prints its bound port; keep draining stderr after, or
    // the runtime's next log line hits a closed pipe.
    let mut lines = BufReader::new(session.0.stderr.take().unwrap()).lines();
    let port: u16 = lines
        .by_ref()
        .map_while(Result::ok)
        .find_map(|line| {
            let rest = line.split("listening on http://127.0.0.1:").nth(1)?.to_string();
            rest.trim_end_matches(|c: char| !c.is_ascii_digit()).parse().ok()
        })
        .expect("debug server port on stderr");
    std::thread::spawn(move || lines.for_each(drop));

    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    write!(stream, "GET /state HTTP/1.0\r\nHost: localhost\r\n\r\n").unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let (head, body) = response.split_once("\r\n\r\n").unwrap();
    assert!(head.contains(" 200 "), "{response}");
    serde_json::from_str::<serde_json::Value>(body).unwrap()["model"].take()
}

#[test]
fn the_game_reads_its_launch_args() {
    let given = [&HEADLESS[..], &["--arg", "server=http://h:9180/?a=b", "--arg", "scan=1", "--arg=scan=latest"]];
    let given = model("given", &given.concat());
    assert_eq!(given, serde_json::json!(["scan=latest", "server=http://h:9180/?a=b"]));
    // Behind the leading `--` runner args may take.
    let dashed = [&["--"][..], &HEADLESS[..], &["--arg", "scan=2"]].concat();
    assert_eq!(model("dashed", &dashed), serde_json::json!(["scan=2"]));
    assert_eq!(model("none", &HEADLESS), serde_json::json!([]));
}
