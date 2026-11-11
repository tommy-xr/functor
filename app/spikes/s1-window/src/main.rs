//! Spike S1: show a hidden `functor run native` child's frames in a wry page.
//!
//! `s1-window serve …` serves one page plus a same-origin proxy (`/game/*` →
//! the child's debug server) and opens a wry window on it (`--no-window` only
//! serves, for headless Chrome). The page posts its stats to `/stats`, which
//! this process prints, so a run is readable from the terminal.
//! `s1-window bench …` is the windowless baseline: N sequential `POST /capture`s
//! (or one `GET /stream` read for N frames).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const PAGE: &str = include_str!("page.html");

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("bench") => bench(&args),
        Some("serve") => serve(&args),
        _ => eprintln!(
            "usage: s1-window serve (--game-port N | --functor BIN --game-dir DIR [--size WxH]) \
             [--port P] [--mode capture|stream] [--no-window] [--exit-after SECS] [--selftest]\n\
             \x20      s1-window bench --game-port N [--n 300] [--route capture|stream]"
        ),
    }
}

// ---------------------------------------------------------------- game child

/// Spawn `functor -d DIR run native --hidden --debug-port 0 --capture-size WxH`
/// and parse the bound port from its `[debug-server] listening on` line.
fn spawn_game(functor: &str, dir: &str, size: &str) -> (Child, u16) {
    let mut child = Command::new(functor)
        .args(["-d", dir, "run", "native", "--hidden", "--debug-port", "0"])
        .args(["--capture-size", size])
        .stdout(Stdio::inherit())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn functor");
    let stderr = child.stderr.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if let Some(rest) = line.split("listening on http://").nth(1) {
                let port = rest
                    .split(|c: char| !c.is_ascii_digit() && c != ':' && c != '.')
                    .next()
                    .and_then(|hp| hp.rsplit(':').next())
                    .and_then(|p| p.parse::<u16>().ok());
                if let Some(port) = port {
                    let _ = tx.send(port);
                }
            }
            eprintln!("[game] {line}");
        }
    });
    let port = rx
        .recv_timeout(Duration::from_secs(60))
        .expect("game never reported its debug port");
    (child, port)
}

// ---------------------------------------------------------------- server

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

fn read_request(reader: &mut BufReader<TcpStream>) -> Option<Request> {
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    let mut headers = Vec::new();
    let mut len = 0usize;
    loop {
        let mut h = String::new();
        reader.read_line(&mut h).ok()?;
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            if k.eq_ignore_ascii_case("content-length") {
                len = v.trim().parse().unwrap_or(0);
            }
            headers.push((k.to_string(), v.trim().to_string()));
        }
    }
    let mut body = vec![0; len];
    reader.read_exact(&mut body).ok()?;
    Some(Request { method, path, headers, body })
}

fn respond(stream: &mut TcpStream, code: u16, ctype: &str, body: &[u8]) {
    let _ = write!(
        stream,
        "HTTP/1.1 {code} X\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(body);
}

/// Forward one request to the game's debug server and pipe its response back
/// byte for byte (the debug server closes after each response, so EOF ends
/// it — and a streaming response just keeps flowing).
fn proxy(client: &mut TcpStream, game_port: u16, req: &Request) {
    let Ok(mut game) = TcpStream::connect(("127.0.0.1", game_port)) else {
        return respond(client, 502, "text/plain", b"game unreachable");
    };
    let _ = game.set_nodelay(true);
    let path = &req.path["/game".len()..];
    let mut head = format!("{} {} HTTP/1.1\r\nHost: 127.0.0.1\r\n", req.method, path);
    for (k, v) in &req.headers {
        if ["origin", "content-type"].contains(&k.to_ascii_lowercase().as_str()) {
            head.push_str(&format!("{k}: {v}\r\n"));
        }
    }
    head.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        req.body.len()
    ));
    if game.write_all(head.as_bytes()).is_err() || game.write_all(&req.body).is_err() {
        return respond(client, 502, "text/plain", b"game write failed");
    }
    let _ = std::io::copy(&mut game, client);
}

fn handle(mut stream: TcpStream, game_port: u16, mode: &str) {
    let _ = stream.set_nodelay(true);
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let Some(req) = read_request(&mut reader) else { return };
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", p) if p == "/" || p.starts_with("/?") => {
            let page = PAGE.replace("__MODE__", mode);
            respond(&mut stream, 200, "text/html", page.as_bytes())
        }
        ("POST", "/stats") => {
            println!("[stats] {}", String::from_utf8_lossy(&req.body));
            respond(&mut stream, 204, "text/plain", b"")
        }
        (_, p) if p.starts_with("/game/") => proxy(&mut stream, game_port, &req),
        _ => respond(&mut stream, 404, "text/plain", b"not found"),
    }
}

fn serve(args: &[String]) {
    let mode = arg(args, "--mode").unwrap_or_else(|| "capture".into());
    let selftest = args.iter().any(|a| a == "--selftest");
    let no_window = args.iter().any(|a| a == "--no-window");
    let exit_after = arg(args, "--exit-after").and_then(|s| s.parse::<f64>().ok());

    let child: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
    let game_port = match arg(args, "--game-port") {
        Some(p) => p.parse().expect("--game-port"),
        None => {
            let functor = arg(args, "--functor").expect("--functor or --game-port");
            let dir = arg(args, "--game-dir").expect("--game-dir");
            let size = arg(args, "--size").unwrap_or_else(|| "1280x720".into());
            let (c, port) = spawn_game(&functor, &dir, &size);
            println!("[s1] game pid {} debug port {port}", c.id());
            *child.lock().unwrap() = Some(c);
            port
        }
    };

    let listener = TcpListener::bind(("127.0.0.1", arg(args, "--port").map_or(0, |p| p.parse().unwrap())))
        .expect("bind");
    let port = listener.local_addr().unwrap().port();
    let url = format!(
        "http://127.0.0.1:{port}/?mode={mode}{}",
        if selftest { "&selftest=1" } else { "" }
    );
    println!("[s1] serving {url}");
    let mode2 = mode.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mode = mode2.clone();
            std::thread::spawn(move || handle(stream, game_port, &mode));
        }
    });

    let started = Instant::now();
    let kill_child = {
        let child = child.clone();
        move || {
            if let Some(mut c) = child.lock().unwrap().take() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
    };

    if no_window {
        while exit_after.is_none_or(|s| started.elapsed().as_secs_f64() < s) {
            std::thread::sleep(Duration::from_millis(100));
        }
        kill_child();
        return;
    }
    run_window(&url, started, exit_after, kill_child);
}

fn run_window(url: &str, started: Instant, exit_after: Option<f64>, kill_child: impl Fn() + 'static) {
    use tao::event::{Event, WindowEvent};
    use tao::event_loop::{ControlFlow, EventLoop};
    use tao::window::WindowBuilder;

    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("S1 game view")
        .with_inner_size(tao::dpi::LogicalSize::new(1300.0, 800.0))
        .build(&event_loop)
        .unwrap();
    let _webview = wry::WebViewBuilder::new()
        .with_url(url)
        .with_devtools(true)
        .build(&window)
        .expect("webview");

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(200));
        let timed_out = exit_after.is_some_and(|s| started.elapsed().as_secs_f64() >= s);
        let closed = matches!(
            event,
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. }
        );
        if timed_out || closed {
            kill_child();
            *control_flow = ControlFlow::Exit;
        }
    });
}

// ---------------------------------------------------------------- bench

fn percentile(sorted: &[f64], p: f64) -> f64 {
    sorted[((sorted.len() as f64 - 1.0) * p).round() as usize]
}

fn report(label: &str, mut ms: Vec<f64>, bytes: usize, wall: Duration) {
    let n = ms.len();
    ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!(
        "[bench] {label}: n={n} fps={:.1} p50={:.1}ms p95={:.1}ms max={:.1}ms bytes/frame={}",
        n as f64 / wall.as_secs_f64(),
        percentile(&ms, 0.5),
        percentile(&ms, 0.95),
        ms[n - 1],
        bytes / n
    );
}

/// Read an HTTP response head; returns the header block.
fn read_head(r: &mut BufReader<TcpStream>) -> String {
    let mut head = String::new();
    loop {
        let mut l = String::new();
        if r.read_line(&mut l).unwrap_or(0) == 0 || l == "\r\n" {
            return head;
        }
        head.push_str(&l);
    }
}

fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.eq_ignore_ascii_case(name).then(|| v.trim())
    })
}

fn bench(args: &[String]) {
    let port: u16 = arg(args, "--game-port").expect("--game-port").parse().unwrap();
    let n: usize = arg(args, "--n").map_or(300, |s| s.parse().unwrap());
    let route = arg(args, "--route").unwrap_or_else(|| "capture".into());
    if route == "stream" {
        return bench_stream(port, n);
    }
    let mut ms = Vec::with_capacity(n);
    let mut bytes = 0;
    let start = Instant::now();
    for _ in 0..n {
        let t = Instant::now();
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_nodelay(true).unwrap();
        s.write_all(b"POST /capture HTTP/1.1\r\nHost: x\r\nContent-Length: 0\r\n\r\n")
            .unwrap();
        let mut body = Vec::new();
        s.read_to_end(&mut body).unwrap();
        ms.push(t.elapsed().as_secs_f64() * 1000.0);
        let split = body.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        assert!(body.starts_with(b"HTTP/1.1 200"), "{}", String::from_utf8_lossy(&body[..split]));
        bytes += body.len() - split;
    }
    report("POST /capture", ms, bytes, start.elapsed());
}

/// Read N parts of `GET /stream`: per-part interval, and capture→received
/// latency from the part's `X-Capture-Ms` (unix ms at readback).
fn bench_stream(port: u16, n: usize) {
    let s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let mut w = s.try_clone().unwrap();
    w.write_all(b"GET /stream HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
    let mut r = BufReader::new(s);
    let head = read_head(&mut r);
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    let (mut gaps, mut lat, mut bytes) = (Vec::new(), Vec::new(), 0);
    let start = Instant::now();
    let mut last = Instant::now();
    for i in 0..n + 1 {
        // Each part: `--frame` line, headers, blank line, body, CRLF.
        let mut boundary = String::new();
        while boundary.trim().is_empty() {
            boundary.clear();
            r.read_line(&mut boundary).unwrap();
        }
        let part = read_head(&mut r);
        let len: usize = header(&part, "content-length").unwrap().parse().unwrap();
        let mut body = vec![0; len];
        r.read_exact(&mut body).unwrap();
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64()
            * 1000.0;
        let captured: f64 = header(&part, "x-capture-ms").unwrap().parse().unwrap();
        if i > 0 {
            // Skip the first part: it measures connection setup, not cadence.
            gaps.push(last.elapsed().as_secs_f64() * 1000.0);
            lat.push(now_ms - captured);
            bytes += len;
        }
        last = Instant::now();
    }
    let wall = start.elapsed();
    report("GET /stream interval", gaps, bytes, wall);
    report("GET /stream capture->recv", lat, bytes, wall);
}
