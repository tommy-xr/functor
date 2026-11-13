//! The device link end to end over loopback sockets: `functor-app
//! --headless` with its link on 127.0.0.1 (no mDNS), driven through its UI
//! socket, against `functor-app device-stub` processes and the headset's
//! own client playing headsets. With `FUNCTOR_QUEST_E2E=1`, one test pairs a
//! Quest attached over adb instead, over Wi-Fi and mDNS.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use functor_device_link::client as headset;
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

/// No cable: `adb` is a path with nothing there, so no real headset is touched.
async fn app() -> App {
    app_with(&["--device-link", "127.0.0.1:0", "--no-mdns", "--adb", NO_ADB]).await
}

const NO_ADB: &str = "/nonexistent/adb";

async fn app_with(link: &[&str]) -> App {
    let project = tempfile::tempdir().unwrap();
    std::fs::write(
        project.path().join("functor.json"),
        r#"{"language": "functor-lang"}"#,
    )
    .unwrap();
    std::fs::write(project.path().join("game.fun"), "").unwrap();
    let home = tempfile::tempdir().unwrap();
    let mut child = Command::new(BIN)
        .args(["--headless", "--no-gpu", "--port", "0"])
        .args(link)
        .arg("-d")
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
    // Keep draining, so the app never blocks on a full pipe; shown with a
    // failure (or `--nocapture`).
    std::thread::spawn(move || {
        for line in stderr.lines().map_while(Result::ok) {
            eprintln!("{line}");
        }
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
}

/// `GET path` on `url` (`http://127.0.0.1:port`): status and body.
fn get(url: &str, path: &str) -> Result<(u16, String), std::io::Error> {
    get_as(url, path, url.strip_prefix("http://").unwrap())
}

/// `get` naming `host` in the `Host` header.
fn get_as(url: &str, path: &str, host: &str) -> Result<(u16, String), std::io::Error> {
    http(url, "GET", path, host, "")
}

/// `POST path` with a JSON `body`.
fn post(url: &str, path: &str, body: &str) -> Result<(u16, String), std::io::Error> {
    http(
        url,
        "POST",
        path,
        url.strip_prefix("http://").unwrap(),
        body,
    )
}

fn http(
    url: &str,
    method: &str,
    path: &str,
    host: &str,
    body: &str,
) -> Result<(u16, String), std::io::Error> {
    let addr = url.strip_prefix("http://").unwrap();
    let mut stream = TcpStream::connect(addr)?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )?;
    let mut text = String::new();
    stream.read_to_string(&mut text)?;
    // An adb forward with nothing behind it answers empty.
    let status = text
        .split(' ')
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or(std::io::ErrorKind::UnexpectedEof)?;
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
    // Until two connections are cut before any answer, however slow the
    // runner; it never links.
    let started = Instant::now();
    let deadline = started + Duration::from_secs(60);
    let (mut refused, mut cut, mut lines) = (0, 0, Vec::new());
    while cut < 2 {
        let left = deadline.saturating_duration_since(Instant::now());
        let Ok(line) = liar.lines.recv_timeout(left) else {
            panic!("not cut twice within 60 s: {lines:?}");
        };
        if line == "rejected NotPaired" {
            refused += 1;
        } else if line.starts_with("ended") && !line.contains("not paired") {
            cut += 1;
        }
        assert!(!line.starts_with("linked"), "it linked: {lines:?}");
        lines.push(line);
    }
    // The budget (10, refilling one per 6 s) was spent before the cuts.
    let refills = started.elapsed().as_secs() as usize / 6;
    assert!(
        (8..=12 + refills).contains(&refused),
        "{refused} refusals: {lines:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_second_app_on_one_home_leaves_the_headsets_to_the_first() {
    let first = app().await;
    let mut second = Command::new(BIN)
        .args(["--headless", "--no-gpu", "--port", "0"])
        .args(["--device-link", "127.0.0.1:0", "--no-mdns", "-d"])
        .arg(first._project.path())
        .env("FUNCTOR_APP_HOME", first.home.path())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stderr = BufReader::new(second.stderr.take().unwrap());
    let mut lines = String::new();
    for _ in 0..2 {
        stderr.read_line(&mut lines).unwrap();
    }
    let _ = second.kill();
    let _ = second.wait();
    assert!(lines.starts_with("[functor-app] listening on"), "{lines}");
    assert!(
        lines.contains("the device link is off: another Functor app"),
        "{lines}"
    );
}

/// A debug runtime stand-in for the headset client: `/state` answers JSON.
fn fake_runtime() -> std::net::SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut head = String::new();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            while reader.read_line(&mut head).unwrap_or(0) > 2 && !head.ends_with("\r\n\r\n") {}
            let path = head.split(' ').nth(1).unwrap_or("").to_string();
            let (status, body) = match path.as_str() {
                "/state" => (200, r#"{"fake":true}"#),
                _ => (404, "not found"),
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    addr
}

struct Battery;

impl headset::Host for Battery {
    fn status(&self) -> functor_device_link::wire::Status {
        functor_device_link::wire::Status {
            battery: Some(55),
            charging: true,
            build: "client".into(),
            project: None,
        }
    }
}

fn headset_client(app: &App, store: &Path, debug: std::net::SocketAddr) -> headset::Client {
    let config = headset::Config {
        store: store.join("device-link.json"),
        name: "Client Quest".into(),
        app_version: "0.1.0".into(),
        debug,
        desktop: Some(app.link.clone()),
    };
    headset::Client::spawn(config, Battery).unwrap()
}

/// Polls `f` until it has an answer, within 20s.
async fn until<T>(what: &str, f: impl Fn() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(t) = f() {
            return t;
        }
        assert!(Instant::now() < deadline, "never: {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn stored_pairing(store: &Path) -> bool {
    headset::Stored::load(&store.join("device-link.json"))
        .unwrap()
        .pairing()
        .is_some()
}

/// The headset's own client (what the Quest runs) against the real desktop:
/// pairs only when both sides agree, tunnels to its debug runtime, relinks
/// after a restart without pairing, and keeps its secret when refused.
#[tokio::test(flavor = "current_thread")]
async fn the_headset_client_pairs_tunnels_and_relinks() {
    let mut app = app().await;
    app.devices_where("the link starts", |d| !d["listening"].is_null())
        .await;
    let store = tempfile::tempdir().unwrap();
    let runtime = fake_runtime();
    let quest = headset_client(&app, store.path(), runtime);

    // Unpaired and not asked: it dials nobody.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(quest.view(), headset::View::default());
    assert!(quest.pairing().is_none());

    quest.send(headset::Command::Pair);
    let code = until("a code shows", || match quest.pairing() {
        Some(headset::Pairing::Code {
            code,
            confirmed: false,
            ..
        }) => Some(code),
        _ => None,
    })
    .await;
    let devices = app
        .devices_where("it asks to pair", |d| asking(d, "Client Quest").is_some())
        .await;
    let ask = asking(&devices, "Client Quest").unwrap();
    assert_eq!(ask["code"], code.as_str(), "both sides show one code");
    // The desktop allows first: nothing is stored until the headset agrees.
    app.send(json!({"type": "allow_pairing", "request": ask["request"].clone()}))
        .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!stored_pairing(store.path()));
    quest.send(headset::Command::Confirm);
    until("it links", || quest.view().linked.then_some(())).await;
    assert!(stored_pairing(store.path()));
    assert!(quest.view().paired.is_some());

    let devices = app
        .devices_where("its heartbeat arrives", |d| {
            card(d, "Client Quest").is_some_and(|c| c["state"]["battery"] == 55)
        })
        .await;
    let tunnel = card(&devices, "Client Quest").unwrap()["tunnel"]
        .as_str()
        .unwrap()
        .to_string();
    let (status, body) = get(&tunnel, "/state").unwrap();
    assert_eq!((status, body.as_str()), (200, r#"{"fake":true}"#));
    assert_eq!(get(&tunnel, "/nowhere").unwrap().0, 404);
    assert_eq!(get(&tunnel, "/stream").unwrap().0, 501);

    // Restarted: it links again by itself.
    drop(quest);
    app.devices_where("it is away", |d| state_of(d, "Client Quest") == "away")
        .await;
    let quest = headset_client(&app, store.path(), runtime);
    until("it relinks", || quest.view().linked.then_some(())).await;
    assert!(quest.pairing().is_none(), "no pairing asked");
    app.devices_where("it is back", |d| state_of(d, "Client Quest") == "connected")
        .await;
    assert_eq!(get(&tunnel, "/state").unwrap().0, 200);

    // Pairing again drops the link; cancelling it keeps the old pairing.
    quest.send(headset::Command::Pair);
    until("a new code", || {
        matches!(quest.pairing(), Some(headset::Pairing::Code { .. })).then_some(())
    })
    .await;
    quest.send(headset::Command::Cancel);
    until("the overlay closes", || {
        quest.pairing().is_none().then_some(())
    })
    .await;
    app.devices_where("the request goes", |d| asking(d, "Client Quest").is_none())
        .await;
    until("it relinks", || quest.view().linked.then_some(())).await;

    // Forgotten: refused, yet the secret stays (a clear refusal is forgeable).
    let devices = app
        .devices_where("connected", |d| state_of(d, "Client Quest") == "connected")
        .await;
    let id = card(&devices, "Client Quest").unwrap()["id"].clone();
    app.send(json!({"type": "forget_device", "device": id}))
        .await;
    app.devices_where("it is forgotten", |d| card(d, "Client Quest").is_none())
        .await;
    until("the link ends", || (!quest.view().linked).then_some(())).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(!quest.view().linked);
    assert!(stored_pairing(store.path()));
}

/// The Quest attached over adb: `npm run test:quest-link`.
struct Quest {
    serial: String,
    debug: String,
    /// The pairing file and properties before the test, put back after.
    pairing: Option<Vec<u8>>,
    props: Vec<(&'static str, String)>,
}

const PACKAGE: &str = "dev.functor.runner";
const PAIRING_FILE: &str = "files/device-link.json";

impl Quest {
    /// The one Quest on adb, its pairing file saved and the properties
    /// `props` set to 1 (both put back on drop), and its debug port
    /// forwarded. Guardian would hold the app at its boundary dialog.
    fn attach(props: &[&'static str]) -> Quest {
        let devices = Command::new("adb").arg("devices").output().unwrap();
        let serial = String::from_utf8_lossy(&devices.stdout)
            .lines()
            .skip(1)
            .find_map(|l| l.strip_suffix("\tdevice").map(str::to_string))
            .expect("a Quest attached over adb");
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let mut quest = Quest {
            serial,
            debug: format!("http://127.0.0.1:{port}"),
            pairing: None,
            props: Vec::new(),
        };
        let saved = quest.adb(&["exec-out", "run-as", PACKAGE, "cat", PAIRING_FILE]);
        quest.pairing = saved.status.success().then_some(saved.stdout);
        for &name in props {
            let before = quest.shell(&format!("getprop {name}"));
            let before = if before.is_empty() {
                "0".into()
            } else {
                before
            };
            quest.props.push((name, before));
            quest.shell(&format!("setprop {name} 1"));
        }
        quest.adb(&["forward", &format!("tcp:{port}"), "tcp:8123"]);
        quest
    }

    /// Replaces the pairing file with `bytes`, as the app does.
    fn put_pairing(&self, bytes: &[u8]) {
        let mut child = Command::new("adb")
            .args(["-s", &self.serial, "shell", functor_device_link::adb::WRITE_PAIRING])
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(bytes).unwrap();
        assert!(child.wait().unwrap().success());
    }

    /// The pairing file (empty when there is none).
    fn pairing_file(&self) -> headset::Stored {
        let out = self.adb(&["shell", functor_device_link::adb::READ_PAIRING]);
        assert!(out.status.success(), "a debuggable {PACKAGE} is installed");
        serde_json::from_slice(&out.stdout).unwrap_or_default()
    }

    fn adb(&self, args: &[&str]) -> std::process::Output {
        Command::new("adb")
            .args(["-s", &self.serial])
            .args(args)
            .output()
            .unwrap()
    }

    fn shell(&self, command: &str) -> String {
        let out = self.adb(&["shell", command]);
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// Launches the app afresh and waits until it renders.
    async fn launch(&self) {
        self.shell("am broadcast -a com.oculus.vrpowermanager.prox_close");
        self.shell(&format!(
            "am start -S -n {PACKAGE}/android.app.NativeActivity"
        ));
        let rendered = |url: &str| -> Option<u64> {
            let (_, body) = get(url, "/state").ok()?;
            serde_json::from_str::<Value>(&body).ok()?["rendered_frames"].as_u64()
        };
        let debug = self.debug.clone();
        let first = until("the app answers", || rendered(&debug)).await;
        until("it renders", || rendered(&debug).filter(|n| *n > first)).await;
    }

    fn link(&self) -> Value {
        let (_, body) = get(&self.debug, "/state").unwrap();
        serde_json::from_str::<Value>(&body).unwrap()["link"].clone()
    }

    /// Holds `controls` (an `xr` sample's hand fields) for `ms`, then lets go.
    async fn press(&self, url: &str, hand: &str, field: &str, ms: u64) {
        let sample =
            |on: bool| json!({"type": "xr", hand: {"active": true, field: on}}).to_string();
        let ok = |sent: (u16, String)| assert_eq!(sent.0, 200, "{}", sent.1);
        ok(post(url, "/input", &sample(true)).unwrap());
        tokio::time::sleep(Duration::from_millis(ms)).await;
        ok(post(url, "/input", &sample(false)).unwrap());
    }

    fn capture(&self, name: &str) {
        let Some(dir) = std::env::var_os("FUNCTOR_QUEST_CAPTURES") else {
            return;
        };
        let addr = self.debug.strip_prefix("http://").unwrap();
        let mut stream = TcpStream::connect(addr).unwrap();
        write!(stream, "POST /capture HTTP/1.1\r\nHost: {addr}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).unwrap();
        let split = bytes.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        std::fs::write(Path::new(&dir).join(name), &bytes[split + 4..]).unwrap();
    }
}

impl Drop for Quest {
    fn drop(&mut self) {
        let _ = post(&self.debug, "/input", r#"{"type":"xr_clear"}"#);
        match &self.pairing {
            Some(bytes) => self.put_pairing(bytes),
            None => {
                self.shell(&format!("run-as {PACKAGE} rm -f {PAIRING_FILE}"));
            }
        }
        for (name, value) in &self.props {
            self.shell(&format!("setprop {name} {value}"));
        }
        let port = self.debug.rsplit(':').next().unwrap().to_string();
        let _ = self.adb(&["forward", "--remove", &format!("tcp:{port}")]);
    }
}

/// A Quest attached over adb with a debuggable `dev.functor.runner`
/// installed (`npm run build:oculus:apk`), on the same Wi-Fi as this
/// machine. Pairs it by mDNS with the headset's overlay driven through
/// injected controller presses, then tunnels, relinks and forgets. The
/// headset's own pairing is put back afterwards. Captures land in
/// `FUNCTOR_QUEST_CAPTURES` when set.
#[tokio::test(flavor = "current_thread")]
async fn quest_pairs_over_wifi_tunnels_and_relinks() {
    if std::env::var_os("FUNCTOR_QUEST_E2E").is_none() {
        eprintln!("skipped: set FUNCTOR_QUEST_E2E=1 with a Quest attached");
        return;
    }
    // Injected presses drive the overlay only while `pair_input` is set.
    let quest = Quest::attach(&["debug.oculus.guardian_pause", "debug.functor.pair_input"]);
    assert!(
        quest
            .adb(&["shell", "run-as", PACKAGE, "rm", "-f", PAIRING_FILE])
            .status
            .success(),
        "a debuggable {PACKAGE} is installed"
    );

    let mut app = app_with(&["--device-link", "0.0.0.0:0"]).await;
    app.devices_where("the link starts", |d| !d["listening"].is_null())
        .await;
    quest.launch().await;
    assert_eq!(quest.link()["linked"], false);

    // Without the property, an injected press is the game's alone (as one
    // from game code reaching this port would be).
    quest.shell("setprop debug.functor.pair_input 0");
    quest
        .press(&quest.debug, "left", "menu_pressed", 1500)
        .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(quest.link()["pairing"].is_null(), "{}", quest.link());
    quest.shell("setprop debug.functor.pair_input 1");

    // Hold the menu button: the overlay looks for a desktop and shows a code.
    quest
        .press(&quest.debug, "left", "menu_pressed", 1500)
        .await;
    let code = until("the headset shows a code", || {
        let pairing = quest.link()["pairing"].clone();
        (pairing["stage"] == "code").then(|| pairing["code"].as_str().unwrap().to_string())
    })
    .await;
    quest.capture("pairing-code.png");
    let devices = app
        .devices_where("it asks to pair", |d| {
            d["requests"].as_array().is_some_and(|r| !r.is_empty())
        })
        .await;
    let ask = devices["requests"][0].clone();
    assert_eq!(ask["code"], code.as_str(), "both sides show one code");
    let name = ask["name"].as_str().unwrap().to_string();

    // A: Matches. Then the desktop allows.
    quest
        .press(&quest.debug, "right", "primary_pressed", 300)
        .await;
    until("the headset waits for the desktop", || {
        (quest.link()["pairing"]["confirmed"] == true).then_some(())
    })
    .await;
    quest.capture("pairing-waiting.png");
    app.send(json!({"type": "allow_pairing", "request": ask["request"].clone()}))
        .await;
    let devices = app
        .devices_where("it connects", |d| {
            state_of(d, &name) == "connected" && !card(d, &name).unwrap()["tunnel"].is_null()
        })
        .await;
    until("the headset is linked", || {
        (quest.link()["linked"] == true).then_some(())
    })
    .await;
    quest.capture("paired.png");

    // The tunnel reaches the headset's own debug runtime.
    let tunnel = card(&devices, &name).unwrap()["tunnel"]
        .as_str()
        .unwrap()
        .to_string();
    let (status, body) = get(&tunnel, "/state").unwrap();
    assert_eq!(status, 200, "{body}");
    let state: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(state["link"]["linked"], true);
    assert!(
        state["views"].as_array().is_some_and(|v| v.len() == 2),
        "both eyes"
    );

    // The tunnel's presses never reach the overlay: no pairing starts.
    quest.press(&tunnel, "left", "menu_pressed", 1500).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let pairing = quest.link()["pairing"].clone();
    // Only the outcome of the pairing above may still show.
    assert!(
        pairing.is_null() || pairing["stage"] == "paired",
        "{pairing}"
    );
    assert_eq!(
        post(&tunnel, "/input", r#"{"type":"xr_clear"}"#).unwrap().0,
        200
    );

    // Restarted, it relinks by itself: no new request on the desktop.
    quest.launch().await;
    app.devices_where("it relinks", |d| {
        state_of(d, &name) == "connected" && d["requests"].as_array().is_none_or(|r| r.is_empty())
    })
    .await;
    until("the tunnel answers again", || {
        get(&tunnel, "/state").ok().filter(|(s, _)| *s == 200)
    })
    .await;

    // Forget: the desktop drops it.
    let id = card(
        &app.devices_where("connected", |d| state_of(d, &name) == "connected")
            .await,
        &name,
    )
    .unwrap()["id"]
        .clone();
    app.send(json!({"type": "forget_device", "device": id}))
        .await;
    app.devices_where("it is forgotten", |d| card(d, &name).is_none())
        .await;
    until("the headset is unlinked", || {
        (quest.link()["linked"] == false).then_some(())
    })
    .await;
}

/// Cable pairing (task 2.4) against a fake `adb`: a shell script that plays
/// one headset from files in its own directory, with its app's private
/// storage as `app/` (where the headset's real client keeps its file).
#[cfg(unix)]
mod cable {
    use super::*;

    const SERIAL: &str = "2G0FAKE";

    struct FakeAdb {
        dir: tempfile::TempDir,
    }

    /// Shared with the pane's Playwright suite (`app/ui/e2e/cable.mjs`).
    const SCRIPT: &str = include_str!("../../ui/e2e/fixtures/fake-adb.sh");

    impl FakeAdb {
        fn new() -> FakeAdb {
            use std::os::unix::fs::PermissionsExt;
            let dir = tempfile::tempdir().unwrap();
            let adb = dir.path().join("adb");
            std::fs::write(&adb, SCRIPT).unwrap();
            std::fs::set_permissions(&adb, std::fs::Permissions::from_mode(0o755)).unwrap();
            std::fs::create_dir_all(dir.path().join("app/files")).unwrap();
            let fake = FakeAdb { dir };
            fake.devices("device usb:1-1 product:eureka model:Quest_3 transport_id:1");
            fake.set("pm", "package:dev.functor.runner\n");
            fake
        }

        fn program(&self) -> String {
            self.dir.path().join("adb").to_string_lossy().into_owned()
        }

        /// The one headset's `adb devices -l` line after its serial.
        fn devices(&self, rest: &str) {
            self.set("devices", &format!("{SERIAL}\t{rest}\n"));
        }

        fn set(&self, name: &str, contents: &str) {
            std::fs::write(self.dir.path().join(name), contents).unwrap();
        }

        fn flag(&self, name: &str, on: bool) {
            match on {
                true => self.set(name, ""),
                false => {
                    let _ = std::fs::remove_file(self.dir.path().join(name));
                }
            }
        }

        /// The headset app's storage, as the client is given it.
        fn store(&self) -> std::path::PathBuf {
            self.dir.path().join("app/files")
        }

        fn file(&self) -> headset::Stored {
            let text = std::fs::read_to_string(self.store().join("device-link.json")).unwrap();
            serde_json::from_str(&text).unwrap()
        }

        fn calls(&self) -> String {
            std::fs::read_to_string(self.dir.path().join("calls")).unwrap_or_default()
        }
    }

    async fn app_on(adb: &FakeAdb) -> App {
        let program = adb.program();
        app_with(&["--device-link", "127.0.0.1:0", "--no-mdns", "--adb", &program]).await
    }

    fn attached(devices: &Value) -> Value {
        devices["attached"][0].clone()
    }

    impl App {
        /// The next refusal's reason, within 20s.
        async fn rejected(&mut self) -> String {
            let deadline = Instant::now() + Duration::from_secs(20);
            loop {
                let left = deadline.saturating_duration_since(Instant::now());
                let msg = tokio::time::timeout(left, self.socket.next())
                    .await
                    .expect("no refusal")
                    .unwrap()
                    .unwrap();
                let json: Value = serde_json::from_str(msg.to_text().unwrap()).unwrap();
                if json["type"] == "rejected" {
                    return json["reason"].as_str().unwrap().to_string();
                }
            }
        }

        async fn cable_state(&mut self, want: &str) -> Value {
            let devices = self
                .devices_where(want, |d| attached(d)["state"]["state"] == want)
                .await;
            attached(&devices)
        }
    }

    fn client_to(app: &App, fake: &FakeAdb, debug: std::net::SocketAddr) -> headset::Client {
        headset_client(app, &fake.store(), debug)
    }

    /// Plugged in: shown, never paired by itself. One click pairs it with
    /// no code; the headset's own client then links by itself. The secret
    /// is in no command line and not in the page's state. Forget clears
    /// the headset's pairing but keeps its id.
    #[tokio::test(flavor = "current_thread")]
    async fn one_click_pairs_a_headset_on_the_cable_and_it_links() {
        let fake = FakeAdb::new();
        let runtime = fake_runtime();
        let mut app = app_on(&fake).await;
        let quest = client_to(&app, &fake, runtime);
        let id = fake.file().device;
        let devices = app
            .devices_where("the headset is on the cable", |d| {
                attached(d)["state"]["state"] == "ready" && attached(d)["state"]["device"] == id
            })
            .await;
        assert_eq!(devices["adb"], true);
        let a = attached(&devices);
        assert_eq!(a["name"], "Quest 3");
        assert_eq!(a["state"]["paired"]["with"], "nobody");
        assert!(card(&devices, "Quest 3").is_none());
        // Never by itself: several looks later, nothing is paired.
        tokio::time::sleep(Duration::from_secs(5)).await;
        assert!(fake.file().secret.is_none() && !app.pairings().contains(&id));

        app.send(json!({"type": "pair_by_cable", "serial": SERIAL}))
            .await;
        let devices = app
            .devices_where("it links over the link", |d| {
                state_of(d, "Client Quest") == "connected"
                    && !card(d, "Client Quest").unwrap()["tunnel"].is_null()
            })
            .await;
        until("the headset is linked", || quest.view().linked.then_some(()))
            .await;
        let file = fake.file();
        assert_eq!(file.device, id, "the headset keeps its id");
        let secret = file.secret.clone().unwrap();
        assert!(app.pairings().contains(&secret), "one secret on both sides");
        assert!(!fake.calls().contains(&secret), "{}", fake.calls());
        assert!(!devices.to_string().contains(&secret));
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = std::fs::metadata(fake.store().join("device-link.json")).unwrap();
            assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        }
        let devices = app
            .devices_where("the cable shows it paired here", |d| {
                attached(d)["state"]["paired"]["with"] == "this"
            })
            .await;
        let tunnel = card(&devices, "Client Quest").unwrap()["tunnel"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(get(&tunnel, "/state").unwrap().0, 200);
        // Ours on both sides: nothing more to pair.
        app.send(json!({"type": "pair_by_cable", "serial": SERIAL}))
            .await;
        assert!(app.rejected().await.contains("cannot be paired"));

        let device = card(&devices, "Client Quest").unwrap()["id"].clone();
        app.send(json!({"type": "forget_device", "device": device}))
            .await;
        app.devices_where("it is forgotten", |d| card(d, "Client Quest").is_none())
            .await;
        until("the headset's pairing is gone", || {
            fake.file().secret.is_none().then_some(())
        })
        .await;
        assert_eq!(fake.file().device, id);
        until("the link ends", || (!quest.view().linked).then_some(())).await;
        app.devices_where("it can pair again", |d| {
            attached(d)["state"]["paired"]["with"] == "nobody"
        })
        .await;
    }

    /// Each state that cannot pair says so, and pairing it is refused.
    #[tokio::test(flavor = "current_thread")]
    async fn a_headset_that_cannot_pair_says_why() {
        let fake = FakeAdb::new();
        let mut app = app_on(&fake).await;
        app.cable_state("ready").await;
        fake.devices("unauthorized usb:1-1 transport_id:1");
        let a = app.cable_state("unauthorized").await;
        assert_eq!(a["name"], "Headset", "unauthorised: adb gives no model");
        fake.devices("offline usb:1-1 transport_id:1");
        app.cable_state("offline").await;
        fake.devices("device usb:1-1 model:Quest_3 transport_id:1");
        fake.set("pm", "");
        app.cable_state("no_app").await;
        fake.set("pm", "package:dev.functor.runner\n");
        fake.flag("not-debuggable", true);
        app.cable_state("not_debuggable").await;
        app.send(json!({"type": "pair_by_cable", "serial": SERIAL}))
            .await;
        assert!(app.rejected().await.contains("cannot be paired"));
        // Over the network is not a cable.
        fake.devices("device product:eureka model:Quest_3 transport_id:1");
        app.devices_where("nothing on the cable", |d| {
            d["attached"].as_array().is_some_and(|a| a.is_empty())
        })
        .await;
        // No adb at all: no cable, said as such.
        let mut none = super::app().await;
        none.devices_where("no adb", |d| d["adb"] == false && !d["listening"].is_null())
            .await;
    }

    /// A write that fails, or a file that reads back differently, leaves
    /// nothing paired on this desktop and says why.
    #[tokio::test(flavor = "current_thread")]
    async fn a_failed_write_or_read_back_stores_nothing() {
        let fake = FakeAdb::new();
        let mut app = app_on(&fake).await;
        app.cable_state("ready").await;
        for (flag, why) in [
            ("write-fails", "Read-only file system"),
            ("mangle", "read back differently"),
        ] {
            fake.flag(flag, true);
            app.send(json!({"type": "pair_by_cable", "serial": SERIAL}))
                .await;
            let reason = app.rejected().await;
            assert!(reason.contains(why), "{flag}: {reason}");
            assert!(!app.pairings().contains("secret"), "{}", app.pairings());
            let devices = app
                .devices_where("not pairing", |d| attached(d)["pairing"] == false)
                .await;
            assert!(devices["list"].as_array().unwrap().is_empty());
            fake.flag(flag, false);
        }
    }

    /// Desktop B finds the headset paired with desktop A: it asks before
    /// replacing. Once replaced, the headset closes A's link saying why (A
    /// keeps its pairing, but its card shows the reason) and links to B.
    #[tokio::test(flavor = "current_thread")]
    async fn replacing_another_desktops_pairing_is_asked_and_the_first_is_told() {
        let fake = FakeAdb::new();
        let runtime = fake_runtime();
        let mut a = app_on(&fake).await;
        let quest = client_to(&a, &fake, runtime);
        a.cable_state("ready").await;
        a.send(json!({"type": "pair_by_cable", "serial": SERIAL}))
            .await;
        a.devices_where("A has it", |d| state_of(d, "Client Quest") == "connected")
            .await;

        let mut b = app_on(&fake).await;
        let devices = b
            .devices_where("B sees it paired elsewhere", |d| {
                attached(d)["state"]["paired"]["with"] == "other"
            })
            .await;
        assert!(attached(&devices)["state"]["paired"]["name"].is_string());
        b.send(json!({"type": "pair_by_cable", "serial": SERIAL}))
            .await;
        assert!(b.rejected().await.contains("is paired with"));
        assert!(fake.file().secret.is_some_and(|s| a.pairings().contains(&s)));
        b.send(json!({"type": "pair_by_cable", "serial": SERIAL, "replace": true}))
            .await;
        b.devices_where("B has it", |d| card(d, "Quest 3").is_some())
            .await;
        let devices = a
            .devices_where("A is told", |d| state_of(d, "Client Quest") == "cannot_reach")
            .await;
        let reason = card(&devices, "Client Quest").unwrap()["state"]["reason"].clone();
        assert!(
            reason.as_str().unwrap().contains(functor_device_link::REPAIRED),
            "{reason}"
        );
        // This client dials A by address; the real one finds B by mDNS.
        drop(quest);
        let quest = client_to(&b, &fake, runtime);
        b.devices_where("it links to B", |d| state_of(d, "Client Quest") == "connected")
            .await;
        until("the headset is linked", || quest.view().linked.then_some(()))
            .await;
    }
}

/// The Quest attached over adb (`npm run test:quest-cable`), paired over
/// the cable with no code: the app writes the pairing file, the headset
/// links over Wi-Fi by itself, the tunnel reaches its debug runtime. A
/// pairing written meanwhile by "another desktop" makes it close this
/// link saying why; pairing again replaces it. Forget clears the file
/// (keeping the headset's id) and the link. The headset's own pairing and
/// properties are put back afterwards.
#[tokio::test(flavor = "current_thread")]
async fn quest_pairs_over_the_cable_and_links_over_wifi() {
    if std::env::var_os("FUNCTOR_QUEST_E2E").is_none() {
        eprintln!("skipped: set FUNCTOR_QUEST_E2E=1 with a Quest attached");
        return;
    }
    let quest = Quest::attach(&["debug.oculus.guardian_pause"]);
    // Unpaired, keeping its id (the desktop knows it by it).
    let mut file = quest.pairing_file();
    let id = file.device.clone();
    file.desktop = None;
    file.desktop_name = None;
    file.secret = None;
    quest.put_pairing(file.to_json().as_bytes());

    let mut app = app_with(&["--device-link", "0.0.0.0:0"]).await;
    quest.launch().await;
    let serial = quest.serial.clone();
    let on_cable = |d: &Value| -> Option<Value> {
        d["attached"]
            .as_array()?
            .iter()
            .find(|a| a["serial"] == serial.as_str())
            .cloned()
    };
    let devices = app
        .devices_where("the Quest is on the cable, ready", |d| {
            on_cable(d).is_some_and(|a| a["pairable"] == true)
        })
        .await;
    let a = on_cable(&devices).unwrap();
    assert_eq!(a["state"]["device"], id.as_str());
    assert_eq!(a["state"]["paired"]["with"], "nobody");
    assert!(app.pairings().is_empty() || !app.pairings().contains(&id));

    // One click; the headset links over Wi-Fi by itself.
    let started = Instant::now();
    app.send(json!({"type": "pair_by_cable", "serial": serial}))
        .await;
    let devices = app
        .devices_where("it connects over Wi-Fi", |d| {
            card_by_id(d, &id).is_some_and(|c| {
                c["state"]["state"] == "connected" && !c["tunnel"].is_null()
            })
        })
        .await;
    eprintln!("cable pairing → connected in {:?}", started.elapsed());
    assert_eq!(card_by_id(&devices, &id).unwrap()["state"]["link"], "wifi");
    let written = quest.pairing_file();
    assert_eq!(written.device, id, "the headset keeps its id");
    let secret = written.secret.clone().unwrap();
    assert!(app.pairings().contains(&secret), "one secret on both sides");
    let mode = quest.shell(&format!("run-as {PACKAGE} ls -l {PAIRING_FILE}"));
    assert!(mode.starts_with("-rw-------"), "{mode}");
    until("the headset says it is linked", || {
        (quest.link()["linked"] == true).then_some(())
    })
    .await;

    // The tunnel reaches the headset's own debug runtime.
    let tunnel = card_by_id(&devices, &id).unwrap()["tunnel"]
        .as_str()
        .unwrap()
        .to_string();
    let (status, body) = get(&tunnel, "/state").unwrap();
    assert_eq!(status, 200, "{body}");
    let state: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(state["link"]["linked"], true);
    assert!(state["views"].as_array().is_some_and(|v| v.len() == 2));

    // Another desktop's pairing lands in its file: it closes this link
    // saying why, and the cable shows whose it is now.
    let elsewhere = headset::Stored {
        device: id.clone(),
        desktop: Some("ee".repeat(16)),
        desktop_name: Some("Studio Mac".into()),
        secret: Some("11".repeat(32)),
    };
    quest.put_pairing(elsewhere.to_json().as_bytes());
    let devices = app
        .devices_where("this desktop is told", |d| {
            card_by_id(d, &id).is_some_and(|c| c["state"]["state"] == "cannot_reach")
                && on_cable(d).is_some_and(|a| a["state"]["paired"]["name"] == "Studio Mac")
        })
        .await;
    let reason = card_by_id(&devices, &id).unwrap()["state"]["reason"].clone();
    assert!(
        reason.as_str().unwrap().contains(functor_device_link::REPAIRED),
        "{reason}"
    );
    app.send(json!({"type": "pair_by_cable", "serial": serial, "replace": true}))
        .await;
    app.devices_where("it is back", |d| {
        card_by_id(d, &id).is_some_and(|c| c["state"]["state"] == "connected")
    })
    .await;
    assert_ne!(quest.pairing_file().desktop_name.as_deref(), Some("Studio Mac"));

    // Forget: gone here, its pairing gone from the headset, its id kept.
    app.send(json!({"type": "forget_device", "device": id}))
        .await;
    app.devices_where("it is forgotten", |d| card_by_id(d, &id).is_none())
        .await;
    until("the headset's pairing is gone", || {
        quest.pairing_file().secret.is_none().then_some(())
    })
    .await;
    assert_eq!(quest.pairing_file().device, id);
    until("the headset is unlinked", || {
        (quest.link()["linked"] == false).then_some(())
    })
    .await;
}

/// The card for device `id`, if any.
fn card_by_id<'a>(devices: &'a Value, id: &str) -> Option<&'a Value> {
    devices["list"]
        .as_array()?
        .iter()
        .find(|d| d["id"] == id)
}
