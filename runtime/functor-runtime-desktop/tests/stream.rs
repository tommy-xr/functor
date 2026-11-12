//! `GET /stream` against a real session: a hidden window streams JPEG parts
//! at the requested size while `/state` and `/input` stay answerable, and
//! `--headless` refuses with a 503.
//!
//! Drives the `functor` CLI next to this test binary (`cargo build --bin
//! functor` first; skipped without it). The hidden-window test needs a GL
//! display, so it is ignored by default:
//!
//! ```sh
//! cargo test -p functor-runtime-desktop --test stream -- --include-ignored
//! ```

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
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

fn launch(mode: &str) -> Option<Session> {
    let functor = functor_bin()?;
    let primitives = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/primitives");
    let mut child = Command::new(functor)
        .args(["-d", primitives, "run", "native", mode, "--debug-port", "0"])
        .args(if mode == "--hidden" {
            &["--capture-size", "320x180"][..]
        } else {
            &[]
        })
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn functor");
    let stderr = BufReader::new(child.stderr.take().unwrap());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in stderr.lines().map_while(Result::ok) {
            if let Some(address) = line.split("listening on http://").nth(1) {
                let _ = tx.send(
                    address
                        .rsplit(':')
                        .next()
                        .unwrap()
                        .trim()
                        .parse::<u16>()
                        .unwrap(),
                );
            }
        }
    });
    let port = rx
        .recv_timeout(Duration::from_secs(60))
        .expect("debug server port");
    Some(Session { child, port })
}

fn request(port: u16, method: &str, path: &str, body: &str) -> TcpStream {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    stream
}

fn whole(port: u16, method: &str, path: &str, body: &str) -> String {
    let mut response = String::new();
    request(port, method, path, body)
        .read_to_string(&mut response)
        .unwrap();
    response
}

fn read_head(reader: &mut impl BufRead) -> String {
    let mut head = String::new();
    while !head.ends_with("\r\n\r\n") {
        assert!(
            reader.read_line(&mut head).unwrap() > 0,
            "closed mid-head: {head}"
        );
    }
    head
}

fn header<'a>(head: &'a str, name: &str) -> &'a str {
    head.lines()
        .find_map(|line| line.strip_prefix(name)?.strip_prefix(": "))
        .unwrap_or_else(|| panic!("no {name} in {head}"))
}

#[test]
fn headless_refuses_the_stream() {
    let Some(session) = launch("--headless") else {
        return;
    };
    let response = whole(session.port, "GET", "/stream", "");
    assert!(
        response.starts_with("HTTP/1.1 503 Service Unavailable\r\n"),
        "{response}"
    );
    assert!(
        response.ends_with("streaming is unavailable in --headless mode"),
        "{response}"
    );
}

#[test]
#[ignore = "needs a GL display"]
fn hidden_session_streams_parts_while_other_routes_answer() {
    let Some(session) = launch("--hidden") else {
        return;
    };
    let mut stream = BufReader::new(request(session.port, "GET", "/stream?w=160", ""));
    let head = read_head(&mut stream);
    assert!(head.starts_with("HTTP/1.1 200 OK\r\n"), "{head}");
    assert!(head.contains("Content-Type: multipart/x-mixed-replace; boundary=functor-frame\r\n"));

    let mut last_frame = None;
    for _ in 0..5 {
        let part = read_head(&mut stream);
        assert!(part.starts_with("--functor-frame\r\n"), "{part}");
        assert_eq!(header(&part, "Content-Type"), "image/jpeg");
        let rendered: u64 = header(&part, "X-Rendered-Frame").parse().unwrap();
        assert!(
            last_frame < Some(rendered),
            "parts advance: {rendered} after {last_frame:?}"
        );
        last_frame = Some(rendered);
        let captured: f64 = header(&part, "X-Capture-Ms").parse().unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap();
        assert!(
            (now.as_secs_f64() * 1000.0 - captured).abs() < 10_000.0,
            "capture time is unix ms"
        );
        let mut jpeg = vec![0; header(&part, "Content-Length").parse().unwrap()];
        stream.read_exact(&mut jpeg).unwrap();
        let mut crlf = [0; 2];
        stream.read_exact(&mut crlf).unwrap();
        assert_eq!(&crlf, b"\r\n");
        let image = image::load_from_memory_with_format(&jpeg, image::ImageFormat::Jpeg).unwrap();
        assert_eq!(
            (image.width(), image.height()),
            (160, 90),
            "fitted to w=160, aspect kept"
        );
    }

    // With the stream open, other requests are answered promptly.
    let started = Instant::now();
    let state = whole(session.port, "GET", "/state", "");
    assert!(state.starts_with("HTTP/1.1 200 OK\r\n"), "{state}");
    let input = whole(
        session.port,
        "POST",
        "/input",
        r#"{"type":"key","key":"w","down":true}"#,
    );
    assert!(input.starts_with("HTTP/1.1 200 OK\r\n"), "{input}");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    let state: serde_json::Value =
        serde_json::from_str(state.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert!(state["rendered_frames"].as_u64() >= last_frame);
}
