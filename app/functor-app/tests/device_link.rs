//! The device link end to end over loopback sockets: `functor-app
//! --headless` with its link on 127.0.0.1 (no mDNS), driven through its UI
//! socket, against `functor-app device-stub` processes playing headsets.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;

const BIN: &str = env!("CARGO_BIN_EXE_functor-app");

struct App {
    child: Child,
    link: String,
    socket: Socket,
    home: tempfile::TempDir,
    _project: tempfile::TempDir,
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn app() -> App {
    let project = tempfile::tempdir().unwrap();
    std::fs::write(
        project.path().join("functor.json"),
        r#"{"language": "functor-lang"}"#,
    )
    .unwrap();
    std::fs::write(project.path().join("game.fun"), "").unwrap();
    let home = tempfile::tempdir().unwrap();
    let mut child = Command::new(BIN)
        .args([
            "--headless",
            "--no-gpu",
            "--port",
            "0",
            "--device-link",
            "127.0.0.1:0",
            "--no-mdns",
            "-d",
        ])
        .arg(project.path())
        .env("FUNCTOR_APP_HOME", home.path())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stderr = BufReader::new(child.stderr.take().unwrap());
    let mut line = String::new();
    stderr.read_line(&mut line).unwrap();
    let (addr, token) = line
        .trim()
        .strip_prefix("[functor-app] listening on http://")
        .and_then(|url| url.split_once("/#token="))
        .unwrap_or_else(|| panic!("unexpected first line: {line:?}"));
    let ws = format!("ws://{addr}/ws?token={token}");
    line.clear();
    stderr.read_line(&mut line).unwrap();
    let link = line
        .trim()
        .strip_prefix("[functor-app] device link on ")
        .unwrap_or_else(|| panic!("unexpected second line: {line:?}"))
        .to_string();
    // Keep draining, so the app never blocks on a full pipe.
    std::thread::spawn(move || {
        let mut sink = Vec::new();
        let _ = stderr.read_to_end(&mut sink);
    });
    let socket = tokio_tungstenite::connect_async(&ws).await.unwrap().0;
    App {
        child,
        link,
        socket,
        home,
        _project: project,
    }
}

impl App {
    /// Reads states until `done` holds for the devices, within 20s.
    async fn devices_where(&mut self, what: &str, done: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let msg = tokio::time::timeout(left, self.socket.next())
                .await
                .unwrap_or_else(|_| panic!("no state where {what}"))
                .unwrap()
                .unwrap();
            let json: Value = serde_json::from_str(msg.to_text().unwrap()).unwrap();
            if json["type"] != "state" {
                continue;
            }
            let devices = json["state"]["devices"].clone();
            if done(&devices) {
                return devices;
            }
        }
    }

    async fn send(&mut self, msg: Value) {
        self.socket
            .send(Message::Text(msg.to_string().into()))
            .await
            .unwrap();
    }

    fn pairings(&self) -> String {
        std::fs::read_to_string(self.home.path().join("devices.json")).unwrap_or_default()
    }
}

/// The card for `name`, if any.
fn card<'a>(devices: &'a Value, name: &str) -> Option<&'a Value> {
    devices["list"]
        .as_array()?
        .iter()
        .find(|d| d["name"] == name)
}

/// `name`'s request to pair, if any.
fn asking<'a>(devices: &'a Value, name: &str) -> Option<&'a Value> {
    devices["requests"]
        .as_array()?
        .iter()
        .find(|r| r["name"] == name)
}

fn state_of(devices: &Value, name: &str) -> String {
    card(devices, name).map_or("gone".into(), |d| {
        d["state"]["state"].as_str().unwrap().to_string()
    })
}

struct Stub {
    child: Child,
    lines: mpsc::Receiver<String>,
}

impl Drop for Stub {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn stub(link: &str, store: &Path, name: &str, extra: &[&str]) -> Stub {
    let mut child = Command::new(BIN)
        .args(["device-stub", "--desktop", link, "--name", name, "--store"])
        .arg(store)
        .args(extra)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let out = BufReader::new(child.stdout.take().unwrap());
    let (tx, lines) = mpsc::channel();
    std::thread::spawn(move || {
        for line in out.lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });
    Stub { child, lines }
}

impl Stub {
    /// The next line starting with `prefix`, within 20s; earlier lines go.
    fn line(&self, prefix: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(left) {
                Ok(line) if line.starts_with(prefix) => return line,
                Ok(_) => {}
                Err(_) => panic!("the stub never printed {prefix:?}"),
            }
        }
    }

    /// Every line within `window`.
    fn lines_for(&self, window: Duration) -> Vec<String> {
        let deadline = Instant::now() + window;
        let mut out = Vec::new();
        while let Ok(line) = self
            .lines
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        {
            out.push(line);
        }
        out
    }
}

/// `GET path` on `url` (`http://127.0.0.1:port`): status and body.
fn get(url: &str, path: &str) -> Result<(u16, String), std::io::Error> {
    get_as(url, path, url.strip_prefix("http://").unwrap())
}

/// `get` naming `host` in the `Host` header.
fn get_as(url: &str, path: &str, host: &str) -> Result<(u16, String), std::io::Error> {
    let addr = url.strip_prefix("http://").unwrap();
    let mut stream = TcpStream::connect(addr)?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
    )?;
    let mut text = String::new();
    stream.read_to_string(&mut text)?;
    let status = text.split(' ').nth(1).unwrap().parse().unwrap();
    let body = text
        .split_once("\r\n\r\n")
        .map_or("", |(_, b)| b)
        .to_string();
    Ok((status, body))
}

/// Pairs a stub named `name` into `app`, allowing it from the UI, and
/// waits for its link; the stub keeps running.
async fn pair(app: &mut App, store: &Path, name: &str) -> Stub {
    let s = stub(&app.link, store, name, &[]);
    let code = s.line("code ")["code ".len()..].to_string();
    let devices = app
        .devices_where("it asks to pair", |d| asking(d, name).is_some())
        .await;
    let ask = asking(&devices, name).unwrap();
    // The code the desktop shows is the one the device shows.
    assert_eq!(ask["code"], code.as_str());
    assert_eq!(devices["headset"], "asking_to_pair");
    let request = ask["request"].clone();
    app.send(json!({"type": "allow_pairing", "request": request}))
        .await;
    s.line("paired ");
    s.line("linked ");
    app.devices_where("it connects", |d| {
        state_of(d, name) == "connected" && !card(d, name).unwrap()["tunnel"].is_null()
    })
    .await;
    s
}

#[tokio::test(flavor = "current_thread")]
async fn pair_link_tunnel_reconnect_drop_and_forget() {
    let mut app = app().await;
    let devices = app
        .devices_where("the link starts", |d| !d["listening"].is_null())
        .await;
    assert_eq!(devices["headset"], "not_paired");
    let store = tempfile::tempdir().unwrap();
    let quest = pair(&mut app, store.path(), "Test Quest").await;
    let pairings = app.pairings();
    assert!(pairings.contains("Test Quest") && pairings.contains("secret"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(app.home.path().join("devices.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    // The tunnel reaches the stub's canned debug runtime.
    let devices = app
        .devices_where("its heartbeat arrives", |d| {
            card(d, "Test Quest").is_some_and(|c| c["state"]["battery"] == 80)
        })
        .await;
    let c = card(&devices, "Test Quest").unwrap();
    assert_eq!(c["state"]["build"], "stub-0.1.0");
    assert_eq!(c["state"]["project"], "examples/hello");
    assert_eq!(c["state"]["link"], "wifi");
    let tunnel = c["tunnel"].as_str().unwrap().to_string();
    assert!(tunnel.starts_with("http://127.0.0.1:"), "{tunnel}");
    let (status, body) = get(&tunnel, "/state").unwrap();
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("\"stub\":true"), "{body}");
    quest.line("request GET /state");
    let (status, _) = get(&tunnel, "/nowhere").unwrap();
    assert_eq!(status, 404);
    // A page on a rebound DNS name is not this port's own host.
    let (status, _) = get_as(&tunnel, "/state", "evil.example:80").unwrap();
    assert_eq!(status, 403);
    // The pairing secret never reaches the page.
    let secret = pairings
        .split("\"secret\":\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    assert_eq!(secret.len(), 64);
    assert!(!devices.to_string().contains(secret));
    // Pings measure the round trip.
    app.devices_where("a ping is answered", |d| {
        card(d, "Test Quest").is_some_and(|c| c["state"]["latency_ms"].is_u64())
    })
    .await;

    // The stub stops: away, and the tunnel says so.
    drop(quest);
    app.devices_where("it is away", |d| state_of(d, "Test Quest") == "away")
        .await;
    let (status, _) = get(&tunnel, "/state").unwrap();
    assert_eq!(status, 503);

    // It comes back without pairing, on the same tunnel port.
    let quest = stub(&app.link, store.path(), "Test Quest", &[]);
    quest.line("linked ");
    app.devices_where("it reconnects", |d| {
        state_of(d, "Test Quest") == "connected"
    })
    .await;
    assert_eq!(get(&tunnel, "/").unwrap().0, 200);
    drop(quest);
    app.devices_where("it is away again", |d| state_of(d, "Test Quest") == "away")
        .await;

    // Dropped without a close: away too.
    let dropper = stub(
        &app.link,
        store.path(),
        "Test Quest",
        &["--misbehave", "drop", "--once"],
    );
    dropper.line("linked ");
    app.devices_where("it connects", |d| state_of(d, "Test Quest") == "connected")
        .await;
    app.devices_where("it drops", |d| state_of(d, "Test Quest") == "away")
        .await;

    // A message over the size limit ends the link: cannot reach.
    let big = stub(
        &app.link,
        store.path(),
        "Test Quest",
        &["--misbehave", "oversize", "--once"],
    );
    big.line("linked ");
    let devices = app
        .devices_where("the oversize message ends it", |d| {
            state_of(d, "Test Quest") == "cannot_reach"
        })
        .await;
    assert_eq!(devices["headset"], "cannot_reach");
    drop(big);

    // An old protocol: needs an update, saying which side.
    let old = stub(
        &app.link,
        store.path(),
        "Test Quest",
        &["--protocol", "0", "--once"],
    );
    old.line("rejected Version(1)");
    let devices = app
        .devices_where("it needs an update", |d| {
            state_of(d, "Test Quest") == "needs_update"
        })
        .await;
    assert_eq!(
        card(&devices, "Test Quest").unwrap()["state"]["update"],
        "headset"
    );

    // An unpaired peer: a prompt, never a tunnel; "Not mine" stores nothing.
    let other = tempfile::tempdir().unwrap();
    let stranger = stub(&app.link, other.path(), "Stranger", &["--once"]);
    stranger.line("code ");
    let devices = app
        .devices_where("the stranger asks", |d| asking(d, "Stranger").is_some())
        .await;
    // A request is no device: no card, no tunnel.
    assert!(card(&devices, "Stranger").is_none());
    let request = asking(&devices, "Stranger").unwrap()["request"].clone();
    app.send(json!({"type": "deny_pairing", "request": request}))
        .await;
    stranger.line("ended pairing was refused");
    app.devices_where("its request goes", |d| asking(d, "Stranger").is_none())
        .await;
    assert!(!app.pairings().contains("Stranger"));

    // Forget: the secret goes on this side, its link and tunnel close.
    let quest = stub(&app.link, store.path(), "Test Quest", &[]);
    quest.line("linked ");
    let devices = app
        .devices_where("it is back", |d| state_of(d, "Test Quest") == "connected")
        .await;
    let id = card(&devices, "Test Quest").unwrap()["id"].clone();
    app.send(json!({"type": "forget_device", "device": id}))
        .await;
    app.devices_where("it is forgotten", |d| card(d, "Test Quest").is_none())
        .await;
    assert!(!app.pairings().contains("Test Quest"));
    quest.line("ended");
    quest.line("rejected NotPaired");
    assert!(get(&tunnel, "/").is_err(), "the tunnel port is closed");
}

#[tokio::test(flavor = "current_thread")]
async fn a_wrong_secret_is_refused_then_rate_limited() {
    let mut app = app().await;
    let store = tempfile::tempdir().unwrap();
    drop(pair(&mut app, store.path(), "Test Quest").await);
    app.devices_where("it is away", |d| state_of(d, "Test Quest") == "away")
        .await;
    // Retrying every half second with a secret the desktop never gave.
    let liar = stub(
        &app.link,
        store.path(),
        "Test Quest",
        &["--misbehave", "wrong-secret"],
    );
    let lines = liar.lines_for(Duration::from_secs(9));
    let refused = lines.iter().filter(|l| *l == "rejected NotPaired").count();
    // The budget (10) is spent, then connections are cut before any answer.
    assert!((8..=12).contains(&refused), "{refused} refusals: {lines:?}");
    let cut = lines
        .iter()
        .filter(|l| l.starts_with("ended") && !l.contains("not paired"))
        .count();
    assert!(cut >= 2, "{lines:?}");
    // It never linked.
    assert!(!lines.iter().any(|l| l.starts_with("linked")));
}
