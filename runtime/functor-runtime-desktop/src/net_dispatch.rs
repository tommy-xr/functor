//! Native HTTP dispatch for the desktop runtime (Phase 1 host side,
//! docs/multiplayer.md).
//!
//! The game queues plain-data `NetCommand`s through `Effect.httpGet`/`httpPost`;
//! the main loop drains them (as JSON) once per frame and hands each to
//! [`perform_http`], which runs on a tokio task so a slow request never stalls
//! the frame loop. The completed [`NetResult`] is sent back over a channel and
//! pushed into the game's async inbox on the main thread next frame.

use functor_runtime_common::net::{HttpMethod, NetCommand};

/// A finished request, ready to push back into the game. `token` echoes the
/// request so the game's `Sub.httpResponses` decoder can correlate it.
pub enum NetResult {
    Response {
        token: i32,
        status: i32,
        body: String,
    },
    Error {
        token: i32,
        message: String,
    },
}

/// Install the desktop remote-asset fetcher: URL asset paths (`Scene.model` /
/// `Texture.file` on an http(s) locator) download through this client on tokio
/// tasks, landing in the asset system's channel-backed future. Must be called
/// from inside the tokio runtime — the handle is captured here so fetches can
/// be spawned from whichever thread polls assets.
pub fn install_remote_asset_fetcher(client: reqwest::Client) {
    let handle = tokio::runtime::Handle::current();
    functor_runtime_common::io::set_remote_fetcher(move |url, tx| {
        let client = client.clone();
        handle.spawn(async move {
            let _ = tx.send(fetch_asset_bytes(&client, &url).await);
        });
    });
}

/// GET one remote asset. Mirrors the wasm fetch rule: an HTTP error status is a
/// FAILED load — the 404 page's body must not reach the glTF/PNG parser — so
/// the asset system serves the fallback instead.
async fn fetch_asset_bytes(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
    // Generous total timeout: big assets on slow links are legitimate, but a
    // stalled endpoint must not leave the asset Loading (fallback) forever
    // with no error event.
    let resp = client
        .get(url)
        .timeout(std::time::Duration::from_secs(300))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("HTTP {}", status.as_u16()));
    }
    resp.bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| format!("reading body: {e}"))
}

fn to_method(method: HttpMethod) -> reqwest::Method {
    match method {
        HttpMethod::Get => reqwest::Method::GET,
        HttpMethod::Post => reqwest::Method::POST,
        HttpMethod::Put => reqwest::Method::PUT,
        HttpMethod::Delete => reqwest::Method::DELETE,
    }
}

/// Perform one networking command. Network/transport failures (and body-read
/// failures) come back as `NetResult::Error`; an HTTP error status (404, 500, …)
/// is still a `Response` — the game decides what to do with the status.
pub async fn perform_http(client: &reqwest::Client, cmd: NetCommand) -> NetResult {
    let NetCommand::HttpRequest {
        token,
        method,
        url,
        headers,
        body,
    } = cmd;
    let token = token as i32;

    let mut req = client.request(to_method(method), &url);
    for (name, value) in headers {
        req = req.header(name, value);
    }
    if !body.is_empty() {
        req = req.body(body);
    }

    match req.send().await {
        Ok(resp) => {
            let status = resp.status().as_u16() as i32;
            match resp.bytes().await {
                Ok(bytes) => NetResult::Response {
                    token,
                    status,
                    body: String::from_utf8_lossy(&bytes).to_string(),
                },
                Err(e) => NetResult::Error {
                    token,
                    message: format!("reading body: {e}"),
                },
            }
        }
        Err(e) => NetResult::Error {
            token,
            message: e.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Start a localhost HTTP server on an OS-assigned port that echoes the
    /// request path back in the body. Returns the port. Hermetic — no external
    /// network, no GL.
    fn spawn_echo_server() -> u16 {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        std::thread::spawn(move || {
            for req in server.incoming_requests() {
                let body = format!("you said {}", req.url());
                let _ = req.respond(tiny_http::Response::from_string(body));
            }
        });
        port
    }

    fn get(token: u64, url: String) -> NetCommand {
        NetCommand::HttpRequest {
            token,
            method: HttpMethod::Get,
            url,
            headers: vec![],
            body: vec![],
        }
    }

    #[tokio::test]
    async fn get_returns_status_and_body() {
        let port = spawn_echo_server();
        let client = reqwest::Client::new();
        let cmd = get(7, format!("http://127.0.0.1:{port}/hello"));
        match perform_http(&client, cmd).await {
            NetResult::Response {
                token,
                status,
                body,
            } => {
                assert_eq!(token, 7);
                assert_eq!(status, 200);
                assert!(body.contains("/hello"), "echoed body was: {body}");
            }
            NetResult::Error { message, .. } => panic!("unexpected error: {message}"),
        }
    }

    /// A server that answers every request with the given status and body.
    fn spawn_status_server(status: u16, body: &'static str) -> u16 {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        std::thread::spawn(move || {
            for req in server.incoming_requests() {
                let _ =
                    req.respond(tiny_http::Response::from_string(body).with_status_code(status));
            }
        });
        port
    }

    #[tokio::test]
    async fn asset_fetch_returns_body_bytes() {
        let port = spawn_status_server(200, "glb-bytes");
        let client = reqwest::Client::new();
        let bytes = fetch_asset_bytes(&client, &format!("http://127.0.0.1:{port}/a.glb"))
            .await
            .expect("fetch should succeed");
        assert_eq!(bytes, b"glb-bytes");
    }

    #[tokio::test]
    async fn asset_fetch_fails_on_http_error_status() {
        // The 404 body must NOT come back as asset bytes (it would be fed to
        // the glTF parser); an error routes to the fallback asset instead.
        let port = spawn_status_server(404, "<html>not found</html>");
        let client = reqwest::Client::new();
        let err = fetch_asset_bytes(&client, &format!("http://127.0.0.1:{port}/a.glb"))
            .await
            .expect_err("404 must fail the load");
        assert!(err.contains("404"), "error was: {err}");
    }

    #[tokio::test]
    async fn connection_refused_is_an_error() {
        let client = reqwest::Client::new();
        // Port 1 is privileged and almost certainly closed: reqwest fails to
        // connect, which must surface as a NetResult::Error carrying the token.
        match perform_http(&client, get(9, "http://127.0.0.1:1/".to_string())).await {
            NetResult::Error { token, .. } => assert_eq!(token, 9),
            NetResult::Response { status, .. } => panic!("expected error, got status {status}"),
        }
    }
}
