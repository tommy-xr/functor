//! Plain `http://` HTTP/1.1 over std's `TcpStream`, for shells without an
//! async HTTP stack: Quest performs `Effect.httpGet`/`httpPost` with it
//! ([`HttpWorkers`]) and `Xr.streamDepth` uploads frames with it. No TLS, no
//! redirects (a 3xx is the answer), one deadline over the whole exchange.

use std::io::{Read, Write};
use std::net::ToSocketAddrs;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use super::{HttpMethod, NetCommand, NetInbound};
use crate::protocol::GameProducer;

/// An `http://host[:port][/path]` URL.
#[derive(Clone, Debug, PartialEq)]
pub struct Destination {
    /// As written in the URL, brackets kept for IPv6: the `Host` header.
    pub(crate) authority: String,
    /// Lowercase, brackets stripped: what to connect to.
    pub(crate) host: String,
    pub(crate) port: u16,
    /// Path and query; `/` when absent.
    pub(crate) path: String,
}

impl Destination {
    pub fn parse(url: &str) -> Result<Destination, String> {
        let rest = url
            .get(..7)
            .filter(|scheme| scheme.eq_ignore_ascii_case("http://"))
            .map(|_| &url[7..])
            .ok_or_else(|| format!("{url:?} must be an http:// URL"))?;
        let rest = rest.split('#').next().unwrap_or_default();
        let (authority, path) = match rest.find(['/', '?']) {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, ""),
        };
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) if !port.contains(']') => (
                host,
                port.parse::<u16>()
                    .map_err(|_| format!("{url:?}: bad port {port:?}"))?,
            ),
            _ => (authority, 80),
        };
        let host = host.trim_start_matches('[').trim_end_matches(']');
        if host.is_empty() || authority.contains('@') || authority.contains(bad) || path.contains(bad) {
            return Err(format!("{url:?} must be http://host[:port][/path]"));
        }
        Ok(Destination {
            authority: authority.to_string(),
            host: host.to_ascii_lowercase(),
            port,
            path: match path {
                "" => "/".to_string(),
                p if p.starts_with('?') => format!("/{p}"),
                p => p.to_string(),
            },
        })
    }

    /// `http://host:port`, the form `--allow-sensor-upload` compares.
    pub fn origin(&self) -> String {
        match self.host.contains(':') {
            true => format!("http://[{}]:{}", self.host, self.port),
            false => format!("http://{}:{}", self.host, self.port),
        }
    }

    pub(crate) fn is_loopback(&self) -> bool {
        self.host == "localhost"
            || self
                .host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    }
}

/// A character no URL or header may carry (request smuggling).
fn bad(c: char) -> bool {
    c.is_whitespace() || c.is_control()
}

/// A status and body.
#[derive(Debug, PartialEq)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

/// Responses past this many bytes fail rather than exhaust device memory.
const MAX_RESPONSE: usize = 64 << 20;

/// The connection and what has been read from it.
struct Conn {
    stream: std::net::TcpStream,
    buf: Vec<u8>,
    deadline: Instant,
    timeout: Duration,
}

impl Conn {
    fn remaining(&self) -> Result<Duration, String> {
        self.deadline
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())
            .ok_or_else(|| format!("timed out after {:?}", self.timeout))
    }

    /// Read more; false at end of stream.
    fn fill(&mut self) -> Result<bool, String> {
        if self.buf.len() > MAX_RESPONSE {
            return Err(format!("response over {MAX_RESPONSE} bytes"));
        }
        self.stream
            .set_read_timeout(Some(self.remaining()?))
            .map_err(|e| e.to_string())?;
        let mut chunk = [0u8; 16 * 1024];
        match self.stream.read(&mut chunk) {
            Ok(0) => Ok(false),
            Ok(n) => {
                self.buf.extend_from_slice(&chunk[..n]);
                Ok(true)
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                Err(format!("timed out after {:?}", self.timeout))
            }
            Err(e) => Err(e.to_string()),
        }
    }

    /// Read until `len` bytes are buffered.
    fn fill_to(&mut self, len: usize) -> Result<(), String> {
        while self.buf.len() < len {
            if !self.fill()? {
                return Err("connection closed mid-response".to_string());
            }
        }
        Ok(())
    }

    /// The index of the first `needle` at or after `from`, reading as needed.
    fn find(&mut self, from: usize, needle: &[u8]) -> Result<usize, String> {
        loop {
            if let Some(i) = self.buf.get(from..).and_then(|rest| {
                rest.windows(needle.len()).position(|w| w == needle)
            }) {
                return Ok(from + i);
            }
            if !self.fill()? {
                return Err("connection closed mid-response".to_string());
            }
        }
    }
}

/// Send one request and read its response within `timeout`, connecting
/// within `connect_timeout`. Any status is a response; only transport and
/// protocol failures are errors.
pub fn request(
    method: &str,
    destination: &Destination,
    headers: &[(String, String)],
    body: &[u8],
    connect_timeout: Duration,
    timeout: Duration,
) -> Result<Response, String> {
    let deadline = Instant::now() + timeout;
    let remaining = || {
        deadline
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())
            .ok_or_else(|| format!("timed out after {timeout:?}"))
    };
    let mut head = format!(
        "{method} {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n",
        destination.path, destination.authority
    );
    for (name, value) in headers {
        if name.is_empty() || name.contains(|c| bad(c) || c == ':') || value.contains(|c: char| c.is_control()) {
            return Err(format!("bad header {name:?}"));
        }
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    if !body.is_empty() || matches!(method, "POST" | "PUT") {
        head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    head.push_str("\r\n");

    let addrs = (destination.host.as_str(), destination.port)
        .to_socket_addrs()
        .map_err(|e| format!("resolve {}: {e}", destination.host))?;
    let mut last = format!("{} resolved to no address", destination.host);
    let mut stream = None;
    for addr in addrs {
        match std::net::TcpStream::connect_timeout(&addr, connect_timeout.min(remaining()?)) {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(e) => last = format!("connect {addr}: {e}"),
        }
    }
    let mut stream = stream.ok_or(last)?;
    let _ = stream.set_nodelay(true);
    // Chunked so a server reading slowly cannot stretch past the deadline.
    for chunk in std::iter::once(head.as_bytes()).chain(body.chunks(64 * 1024)) {
        stream.set_write_timeout(Some(remaining()?)).map_err(|e| e.to_string())?;
        stream.write_all(chunk).map_err(|e| e.to_string())?;
    }

    let mut conn = Conn {
        stream,
        buf: Vec::new(),
        deadline,
        timeout,
    };
    // Skip interim 1xx responses to the final head.
    let mut start = 0;
    let (status, head_end, length, chunked) = loop {
        let end = conn.find(start, b"\r\n\r\n")?;
        let text = String::from_utf8_lossy(&conn.buf[start..end]).into_owned();
        let mut lines = text.split("\r\n");
        let line = lines.next().unwrap_or_default();
        let status = line
            .split_whitespace()
            .nth(1)
            .and_then(|code| code.parse::<u16>().ok())
            .filter(|_| line.starts_with("HTTP/"))
            .ok_or_else(|| format!("bad response {line:?}"))?;
        start = end + 4;
        if (100..200).contains(&status) {
            continue;
        }
        let mut length = None;
        let mut chunked = false;
        for header in lines {
            let Some((name, value)) = header.split_once(':') else { continue };
            let value = value.trim();
            if name.eq_ignore_ascii_case("content-length") {
                length = Some(value.parse::<usize>().map_err(|_| format!("bad Content-Length {value:?}"))?);
            } else if name.eq_ignore_ascii_case("transfer-encoding") {
                chunked = value.to_ascii_lowercase().ends_with("chunked");
            }
        }
        break (status, start, length, chunked);
    };

    let body = if method == "HEAD" || status == 204 || status == 304 {
        Vec::new()
    } else if chunked {
        read_chunked(&mut conn, head_end)?
    } else if let Some(length) = length {
        if length > MAX_RESPONSE {
            return Err(format!("response over {MAX_RESPONSE} bytes"));
        }
        conn.fill_to(head_end + length)?;
        conn.buf[head_end..head_end + length].to_vec()
    } else {
        while conn.fill()? {}
        conn.buf[head_end..].to_vec()
    };
    Ok(Response { status, body })
}

/// Decode a chunked body starting at `pos`; trailers are ignored.
fn read_chunked(conn: &mut Conn, mut pos: usize) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    loop {
        let line_end = conn.find(pos, b"\r\n")?;
        let line = String::from_utf8_lossy(&conn.buf[pos..line_end]).into_owned();
        let size = line.split(';').next().unwrap_or_default().trim();
        let size = usize::from_str_radix(size, 16).map_err(|_| format!("bad chunk size {size:?}"))?;
        pos = line_end + 2;
        if size == 0 {
            return Ok(body);
        }
        if body.len() + size > MAX_RESPONSE {
            return Err(format!("response over {MAX_RESPONSE} bytes"));
        }
        conn.fill_to(pos + size + 2)?;
        body.extend_from_slice(&conn.buf[pos..pos + size]);
        pos += size + 2;
    }
}

/// `Effect.httpGet`/`httpPost` deadlines: generous for a large POST, finite
/// so a dead endpoint becomes a `Net.Failure` instead of a request pending
/// forever.
const EFFECT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const EFFECT_TIMEOUT: Duration = Duration::from_secs(60);

/// Perform one game request, blocking. Transport failures — and URLs this
/// client cannot serve, such as `https://` — come back as
/// [`NetInbound::HttpError`]; any HTTP status is a response.
pub fn perform(cmd: NetCommand) -> NetInbound {
    let NetCommand::HttpRequest {
        token,
        method,
        url,
        headers,
        body,
    } = cmd;
    let method = match method {
        HttpMethod::Get => "GET",
        HttpMethod::Post => "POST",
        HttpMethod::Put => "PUT",
        HttpMethod::Delete => "DELETE",
    };
    let result = if url.get(..8).is_some_and(|s| s.eq_ignore_ascii_case("https://")) {
        let message = format!(
            "https is not supported on this runtime (no TLS): {url} — serve it over http:// \
(a headset reaches its host's http server through `adb reverse`)"
        );
        super::warn_once("https", &message);
        Err(message)
    } else {
        Destination::parse(&url).and_then(|destination| {
            request(method, &destination, &headers, &body, EFFECT_CONNECT_TIMEOUT, EFFECT_TIMEOUT)
        })
    };
    match result {
        Ok(response) => NetInbound::HttpResponse {
            token,
            status: response.status,
            body: response.body,
        },
        Err(message) => NetInbound::HttpError { token, message },
    }
}

/// Requests performed at once; more queue behind them.
const WORKERS: usize = 4;

/// Performs game requests on background threads, so a slow request never
/// stalls a frame. Each result goes to `results` through the counted
/// [`super::send_inbound`] hand-off; the shell then [`deliver`]s it. The
/// threads end when this is dropped.
pub struct HttpWorkers {
    jobs: mpsc::Sender<NetCommand>,
}

impl HttpWorkers {
    pub fn spawn(results: mpsc::Sender<NetInbound>) -> HttpWorkers {
        let (jobs, queue) = mpsc::channel::<NetCommand>();
        let queue = Arc::new(Mutex::new(queue));
        for i in 0..WORKERS {
            let queue = queue.clone();
            let results = results.clone();
            std::thread::Builder::new()
                .name(format!("functor-http-{i}"))
                .spawn(move || loop {
                    let next = queue.lock().expect("http queue").recv();
                    let Ok(cmd) = next else { return };
                    super::send_inbound(&results, perform(cmd));
                })
                .expect("spawn an HTTP worker");
        }
        HttpWorkers { jobs }
    }

    pub fn submit(&self, cmd: NetCommand) {
        super::note_http_issued();
        // The workers hold the queue for as long as this sender lives.
        let _ = self.jobs.send(cmd);
    }
}

/// Push a finished request into the game, which applies its tagger next
/// frame, and count it in [`super::http_stats`].
pub fn deliver(game: &mut dyn GameProducer, result: NetInbound) {
    match result {
        NetInbound::HttpResponse {
            token,
            status,
            body,
        } => {
            super::note_http_delivered(true);
            game.net_push_http_response(token as i32, status as i32, String::from_utf8_lossy(&body).into_owned());
        }
        NetInbound::HttpError { token, message } => {
            super::note_http_delivered(false);
            game.net_push_http_error(token as i32, message);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn destinations_parse() {
        let d = Destination::parse("http://127.0.0.1:9000/frames?x=1#f").unwrap();
        assert_eq!((d.host.as_str(), d.port, d.path.as_str()), ("127.0.0.1", 9000, "/frames?x=1"));
        assert_eq!(d.origin(), "http://127.0.0.1:9000");
        let d = Destination::parse("HTTP://LocalHost").unwrap();
        assert_eq!((d.host.as_str(), d.port, d.path.as_str()), ("localhost", 80, "/"));
        let d = Destination::parse("http://[::1]:8080/f").unwrap();
        assert_eq!((d.host.as_str(), d.authority.as_str()), ("::1", "[::1]:8080"));
        assert_eq!(d.origin(), "http://[::1]:8080");
        for bad in [
            "https://host/",
            "ftp://x",
            "http://",
            "http://u@host/",
            "http://h:99999/",
            "http://evil\r\nX:1/",
        ] {
            assert!(Destination::parse(bad).is_err(), "{bad}");
        }
    }

    /// Serve one connection: read the request head (and a `Content-Length`
    /// body), hand both to `reply`, write what it returns. Returns the port.
    pub(crate) fn serve_once(reply: impl FnOnce(&str, &[u8]) -> Vec<u8> + Send + 'static) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            let head_end = loop {
                let n = stream.read(&mut chunk).unwrap();
                buf.extend_from_slice(&chunk[..n]);
                if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    break i + 4;
                }
            };
            let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
            let length = head
                .lines()
                .find_map(|l| l.strip_prefix("Content-Length: "))
                .map_or(0, |n| n.trim().parse::<usize>().unwrap());
            while buf.len() < head_end + length {
                let n = stream.read(&mut chunk).unwrap();
                buf.extend_from_slice(&chunk[..n]);
            }
            let out = reply(&head, &buf[head_end..head_end + length]);
            stream.write_all(&out).unwrap();
        });
        port
    }

    fn post(port: u16, body: &[u8]) -> NetInbound {
        perform(NetCommand::HttpRequest {
            token: 3,
            method: HttpMethod::Post,
            url: format!("http://127.0.0.1:{port}/scan?x=1"),
            headers: vec![],
            body: body.to_vec(),
        })
    }

    #[test]
    fn a_post_sends_its_body_and_reads_a_sized_response() {
        let port = serve_once(|head, body| {
            assert!(head.starts_with("POST /scan?x=1 HTTP/1.1\r\n"), "{head}");
            assert!(head.contains("\r\nHost: 127.0.0.1:"), "{head}");
            let echoed = format!("got {}", String::from_utf8_lossy(body));
            format!("HTTP/1.1 201 Created\r\nContent-Length: {}\r\n\r\n{echoed}", echoed.len()).into_bytes()
        });
        assert_eq!(
            post(port, b"{\"a\":1}"),
            NetInbound::HttpResponse {
                token: 3,
                status: 201,
                body: b"got {\"a\":1}".to_vec()
            }
        );
    }

    #[test]
    fn chunked_and_close_delimited_bodies_decode() {
        let port = serve_once(|_, _| {
            b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4;x=y\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n".to_vec()
        });
        assert_eq!(
            post(port, b""),
            NetInbound::HttpResponse { token: 3, status: 200, body: b"Wikipedia".to_vec() }
        );
        let port = serve_once(|_, _| b"HTTP/1.0 404 Not Found\r\n\r\nno such scan".to_vec());
        assert_eq!(
            post(port, b""),
            NetInbound::HttpResponse { token: 3, status: 404, body: b"no such scan".to_vec() }
        );
    }

    #[test]
    fn unservable_urls_and_dead_ports_fail_with_a_reason() {
        let failure = |url: &str| match perform(NetCommand::HttpRequest {
            token: 5,
            method: HttpMethod::Get,
            url: url.to_string(),
            headers: vec![],
            body: vec![],
        }) {
            NetInbound::HttpError { token: 5, message } => message,
            other => panic!("expected an error, got {other:?}"),
        };
        assert!(failure("https://example.com/").contains("https is not supported"));
        assert!(failure("ftp://x/").contains("must be an http:// URL"));
        assert!(failure("http://127.0.0.1:1/").contains("connect"));
    }
}
