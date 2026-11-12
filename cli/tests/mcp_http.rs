//! `functor mcp --http`: the token gate, tool parity with stdio, `app_context`,
//! `--attach`, and shutdown — against the real binary, no GPU (docs/mcp.md).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

const TOKEN: &str = "test-token-0123456789";

fn functor() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_functor"));
    command.env_remove("FUNCTOR_MCP_TOKEN");
    command
}

/// `functor mcp --http 0 <extra>` with the token set; its URL from stderr.
struct HttpServer {
    child: Child,
    stdin: Option<ChildStdin>,
    url: String,
}

impl HttpServer {
    fn start(extra: &[&str]) -> Self {
        let mut child = functor()
            .args(["mcp", "--http", "0"])
            .args(extra)
            .env("FUNCTOR_MCP_TOKEN", TOKEN)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn functor mcp --http");
        let stdin = child.stdin.take();
        let (tx, rx) = mpsc::channel();
        let stderr = child.stderr.take().unwrap();
        // Forward lines (and keep draining so the child never blocks on a full pipe).
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                let _ = tx.send(line);
            }
        });
        const MARK: &str = "[mcp] listening on ";
        let deadline = Instant::now() + Duration::from_secs(30);
        let url = loop {
            let line = rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("the server printed its listening line");
            if let Some(url) = line.strip_prefix(MARK) {
                break url.to_string();
            }
        };
        assert!(url.starts_with("http://127.0.0.1:") && url.ends_with("/mcp"), "{url}");
        Self { child, stdin, url }
    }

    fn base(&self) -> &str {
        self.url.trim_end_matches("/mcp")
    }
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .unwrap()
}

/// POST one JSON-RPC message to `/mcp`; returns (status, session id, body).
fn post_mcp(
    url: &str,
    token: Option<&str>,
    session: Option<&str>,
    message: &Value,
) -> (u16, Option<String>, String) {
    let mut request = client()
        .post(url)
        .header("Accept", "application/json, text/event-stream")
        .header("Content-Type", "application/json")
        .body(message.to_string());
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    if let Some(session) = session {
        request = request.header("Mcp-Session-Id", session);
    }
    let response = request.send().unwrap();
    let status = response.status().as_u16();
    let session = response
        .headers()
        .get("mcp-session-id")
        .map(|value| value.to_str().unwrap().to_string());
    (status, session, response.text().unwrap())
}

/// The JSON-RPC response with `id` from an SSE (or plain JSON) body.
fn response_with_id(body: &str, id: u64) -> Value {
    body.lines()
        .map(|line| line.strip_prefix("data:").unwrap_or(line))
        .filter_map(|data| serde_json::from_str::<Value>(data.trim()).ok())
        .find(|message| message["id"] == id)
        .unwrap_or_else(|| panic!("no response {id} in {body}"))
}

/// An initialized MCP session over HTTP.
struct HttpMcp {
    url: String,
    session: String,
    next_id: u64,
}

impl HttpMcp {
    fn connect(url: &str) -> Self {
        let init = json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18", "capabilities": {},
            "clientInfo": {"name": "mcp-http-test", "version": "0"}}});
        let (status, session, body) = post_mcp(url, Some(TOKEN), None, &init);
        assert_eq!(status, 200, "{body}");
        assert!(response_with_id(&body, 0)["result"]["serverInfo"].is_object(), "{body}");
        let session = session.expect("an Mcp-Session-Id header");
        let initialized = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        let (status, _, _) = post_mcp(url, Some(TOKEN), Some(&session), &initialized);
        assert_eq!(status, 202);
        Self { url: url.to_string(), session, next_id: 1 }
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let message = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let (status, _, body) = post_mcp(&self.url, Some(TOKEN), Some(&self.session), &message);
        assert_eq!(status, 200, "{body}");
        let response = response_with_id(&body, id);
        response["result"].clone()
    }

    /// A tool call's text, asserting it succeeded.
    fn call(&mut self, tool: &str, arguments: Value) -> String {
        let result = self.request("tools/call", json!({"name": tool, "arguments": arguments}));
        assert_ne!(result["isError"], true, "{tool}: {result}");
        result["content"][0]["text"].as_str().unwrap().to_string()
    }
}

fn tool_names(list: &Value) -> Vec<String> {
    let mut names: Vec<String> = list["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_string())
        .collect();
    names.sort();
    names
}

/// Run `functor mcp <extra>` over stdio: initialize, then each request in
/// order; returns their results. Closing stdin ends the server.
fn stdio_requests(extra: &[&str], requests: &[(&str, Value)]) -> Vec<Value> {
    let mut child = functor()
        .arg("mcp")
        .args(extra)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut send = |message: Value| writeln!(stdin, "{message}").unwrap();
    send(json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {
        "protocolVersion": "2025-06-18", "capabilities": {},
        "clientInfo": {"name": "mcp-http-test", "version": "0"}}}));
    send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    for (id, (method, params)) in requests.iter().enumerate() {
        send(json!({"jsonrpc": "2.0", "id": id + 1, "method": method, "params": params}));
    }
    let mut results = Vec::new();
    let mut line = String::new();
    while results.len() < requests.len() {
        line.clear();
        assert!(stdout.read_line(&mut line).unwrap() > 0, "stdio server closed early");
        let message: Value = serde_json::from_str(&line).unwrap();
        if message["id"].as_u64().is_some_and(|id| id > 0) {
            results.push(message["result"].clone());
        }
    }
    drop(stdin);
    assert!(wait_exit(&mut child, Duration::from_secs(10)).is_some());
    results
}

fn wait_exit(child: &mut Child, timeout: Duration) -> Option<ExitStatus> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().unwrap() {
            return Some(status);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    None
}

/// A stand-in debug runtime answering `GET /` (discovery) and `GET /state`.
fn fake_debug_runtime() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for mut stream in listener.incoming().map_while(Result::ok) {
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap_or(0) == 1 {
                head.push(byte[0]);
            }
            let head = String::from_utf8_lossy(&head);
            let path = head.split_whitespace().nth(1).unwrap_or("");
            let body = match path {
                "/" => json!({"service": "functor debug runtime", "protocol_version": 8}),
                "/state" => json!({"frame": 42, "tts": 0.7, "model": {"count": 7}}),
                _ => json!({"error": "not found"}),
            }
            .to_string();
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    url
}

fn answers(url: &str) -> bool {
    client()
        .get(format!("{url}/state"))
        .timeout(Duration::from_secs(2))
        .send()
        .is_ok_and(|response| response.status().is_success())
}

#[test]
fn http_without_the_token_env_is_a_startup_error() {
    let output = functor().args(["mcp", "--http", "0"]).stdin(Stdio::null()).output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("FUNCTOR_MCP_TOKEN"), "{stderr}");
    assert_eq!(stderr.trim().lines().count(), 1, "{stderr}");
    // A token that could never match a header (here a trailing newline) too.
    let output = functor()
        .args(["mcp", "--http", "0"])
        .env("FUNCTOR_MCP_TOKEN", format!("{TOKEN}\n"))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!output.status.success() && String::from_utf8_lossy(&output.stderr).contains("FUNCTOR_MCP_TOKEN"));
}

#[test]
fn requests_without_the_exact_token_get_401() {
    let server = HttpServer::start(&[]);
    let ping = json!({"jsonrpc": "2.0", "id": 0, "method": "ping"});
    for token in [None, Some("wrong"), Some("test-token-012345678"), Some("test-token-01234567890")] {
        let (status, _, _) = post_mcp(&server.url, token, None, &ping);
        assert_eq!(status, 401, "token {token:?}");
    }
    // A non-Bearer scheme carrying the right secret is still refused.
    let status = client()
        .post(&server.url)
        .header("Authorization", format!("Basic {TOKEN}"))
        .send()
        .unwrap()
        .status();
    assert_eq!(status, 401);
    // The scheme is case-insensitive (RFC 9110); the token is not.
    let auth = |value: String| {
        let request = client().post(&server.url).header("Authorization", value);
        request.send().unwrap().status().as_u16()
    };
    assert_eq!(auth(format!("bearer {}", TOKEN.to_uppercase())), 401);
    assert_ne!(auth(format!("bearer {TOKEN}")), 401);
    // rmcp's loopback `Host` check stays on behind the token (DNS rebinding).
    let status = client()
        .post(&server.url)
        .bearer_auth(TOKEN)
        .header("Host", "attacker.example")
        .header("Accept", "application/json, text/event-stream")
        .header("Content-Type", "application/json")
        .body(ping.to_string())
        .send()
        .unwrap()
        .status();
    assert_eq!(status, 403);
}

#[test]
fn http_lists_the_same_tools_as_stdio_and_none_reserved() {
    let server = HttpServer::start(&[]);
    let http = tool_names(&HttpMcp::connect(&server.url).request("tools/list", json!({})));
    let stdio = tool_names(&stdio_requests(&[], &[("tools/list", json!({}))])[0]);
    assert_eq!(http, stdio);
    assert!(http.contains(&"app_context".to_string()) && http.contains(&"get_state".to_string()));
    // An embedding agent runner reserves these for its own tools.
    for reserved in ["report_status", "report_evidence", "report_result", "approve", "merge_pr"] {
        assert!(!http.contains(&reserved.to_string()), "{reserved}");
    }
}

#[test]
fn app_context_returns_what_was_last_put() {
    let server = HttpServer::start(&[]);
    let mut mcp = HttpMcp::connect(&server.url);
    let none: Value = serde_json::from_str(&mcp.call("app_context", json!({}))).unwrap();
    assert_eq!(none["context"], Value::Null, "{none}");

    let endpoint = format!("{}/app-context", server.base());
    let put = |token: Option<&str>, body: String| {
        let mut request = client().put(&endpoint).body(body);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        request.send().unwrap().status().as_u16()
    };
    let context = json!({"view": "game", "file": "game.fun", "session": "s1"});
    assert_eq!(put(None, context.to_string()), 401);
    assert_eq!(put(Some("wrong"), context.to_string()), 401);
    assert_eq!(put(Some(TOKEN), "[1, 2]".into()), 400);
    assert_eq!(put(Some(TOKEN), "{not json".into()), 400);
    assert_eq!(put(Some(TOKEN), context.to_string()), 204);
    let huge = json!({"file": "x".repeat(70 * 1024)}).to_string();
    assert_eq!(put(Some(TOKEN), huge), 413);

    let got: Value = serde_json::from_str(&mcp.call("app_context", json!({}))).unwrap();
    assert_eq!(got["context"], context, "the oversize body did not replace it");

    // Over stdio the tool exists and reports none set.
    let stdio = &stdio_requests(&[], &[("tools/call", json!({"name": "app_context", "arguments": {}}))])[0];
    let text: Value = serde_json::from_str(stdio["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text["context"], Value::Null);
}

#[test]
fn attach_makes_game_tools_work_without_a_launch() {
    let runtime = fake_debug_runtime();
    let attach = format!("{runtime}/");
    let server = HttpServer::start(&["--attach", &attach]);
    let mut mcp = HttpMcp::connect(&server.url);
    let sessions = mcp.call("list_sessions", json!({}));
    assert!(sessions.contains(&runtime) && sessions.contains("\"s1\""), "{sessions}");
    let state: Value = serde_json::from_str(&mcp.call("get_state", json!({"session": "s1"}))).unwrap();
    assert_eq!(state["model"]["count"], 7);

    // Same over stdio.
    let stdio = &stdio_requests(
        &["--attach", &runtime],
        &[("tools/call", json!({"name": "get_state", "arguments": {"session": "s1"}}))],
    )[0];
    assert!(stdio["content"][0]["text"].as_str().unwrap().contains("\"frame\":42"), "{stdio}");
}

#[test]
fn attach_to_a_dead_url_fails_at_startup() {
    let dead = {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}", listener.local_addr().unwrap())
    };
    let output = functor()
        .args(["mcp", "--http", "0", "--attach", &dead])
        .env("FUNCTOR_MCP_TOKEN", TOKEN)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(&dead) && !stderr.contains("listening"), "{stderr}");
}

/// Closing stdin (the child convention) kills launched games but never an
/// attached runtime. Launches a real HEADLESS game: no GPU needed.
#[test]
fn closing_stdin_stops_launched_games_and_spares_attached_ones() {
    let runtime = fake_debug_runtime();
    let mut server = HttpServer::start(&["--attach", &runtime]);
    let mut mcp = HttpMcp::connect(&server.url);
    let counter = concat!(env!("CARGO_MANIFEST_DIR"), "/../examples/counter");
    let launched: Value =
        serde_json::from_str(&mcp.call("launch_game", json!({"dir": counter, "mode": "headless"})))
            .unwrap();
    let game = launched["url"].as_str().unwrap().to_string();
    assert!(answers(&game));

    drop(server.stdin.take());
    let status = wait_exit(&mut server.child, Duration::from_secs(10)).expect("exits on stdin EOF");
    assert!(status.success(), "{status}");
    let deadline = Instant::now() + Duration::from_secs(10);
    while answers(&game) {
        assert!(Instant::now() < deadline, "the launched game outlived the server");
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(answers(&runtime), "the attached runtime was left alone");
}

/// SIGTERM is Unix-only; Windows hosts use the stdin convention above.
#[cfg(unix)]
#[test]
fn sigterm_exits_promptly_and_spares_attached_runtimes() {
    let runtime = fake_debug_runtime();
    let mut server = HttpServer::start(&["--attach", &runtime]);
    let started = Instant::now();
    let killed = Command::new("kill")
        .args(["-TERM", &server.child.id().to_string()])
        .status()
        .unwrap();
    assert!(killed.success());
    let status = wait_exit(&mut server.child, Duration::from_secs(5)).expect("exits on SIGTERM");
    assert!(status.success(), "{status} after {:?}", started.elapsed());
    assert!(answers(&runtime));
}
