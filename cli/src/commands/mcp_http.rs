//! `functor mcp --http <port>`: the same MCP tools over streamable HTTP, for a
//! host (e.g. a desktop app) that runs this server as a child and hands its URL
//! to an agent. Loopback only, bearer-token gated (docs/mcp.md).

use std::future::Future as _;
use std::io::{self, IsTerminal as _, Read as _};
use std::sync::Arc;
use std::task::{Context, Waker};

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::put;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
use serde_json::Value;
use subtle::ConstantTimeEq as _;

use super::mcp::{shutdown_signal, FunctorMcp};

/// The environment variable holding the bearer token (never argv: other local
/// users can read a process's arguments).
pub const TOKEN_ENV: &str = "FUNCTOR_MCP_TOKEN";

/// `PUT /app-context` bodies above this are refused (413).
const MAX_APP_CONTEXT_BYTES: usize = 64 * 1024;

pub fn token_from_env() -> io::Result<String> {
    secret_from_env(TOKEN_ENV).ok_or_else(|| io::Error::other(format!(
        "`functor mcp --http` needs a bearer token: set {TOKEN_ENV} to a secret of visible ASCII characters that clients send as `Authorization: Bearer <token>`"
    )))
}

/// The secret in `name`, if set. Visible ASCII only: header parsers trim
/// whitespace, so a secret with a trailing newline would silently never match.
pub fn secret_from_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_graphic()))
}

/// Serve `/mcp` and `/app-context` on `127.0.0.1:port` until a shutdown
/// signal or stdin EOF; the caller then runs the registry shutdown.
pub async fn serve(server: FunctorMcp, port: u16, token: String) -> io::Result<()> {
    let factory = server.clone();
    // Every connection gets a clone sharing one session registry. The default
    // config keeps rmcp's loopback-only `Host` check (DNS-rebinding guard).
    let mcp: StreamableHttpService<FunctorMcp, LocalSessionManager> = StreamableHttpService::new(
        move || Ok(factory.clone()),
        Default::default(),
        StreamableHttpServerConfig::default(),
    );
    let app = axum::Router::new()
        .nest_service("/mcp", mcp)
        .route(
            "/app-context",
            put(put_app_context).layer(DefaultBodyLimit::max(MAX_APP_CONTEXT_BYTES)),
        )
        .with_state(server)
        .layer(middleware::from_fn_with_state(Arc::<str>::from(token), require_token));

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|error| io::Error::other(format!("cannot listen on 127.0.0.1:{port}: {error}")))?;
    let address = listener.local_addr()?;
    // Poll once so the signal handlers exist before the URL is announced: a
    // host may signal as soon as it reads the line.
    let mut signal = std::pin::pin!(shutdown_signal());
    if signal.as_mut().poll(&mut Context::from_waker(Waker::noop())).is_ready() {
        return Ok(());
    }
    eprintln!("[mcp] listening on http://{address}/mcp");

    tokio::select! {
        result = axum::serve(listener, app) => result,
        _ = signal => Ok(()),
        _ = stdin_closed() => Ok(()),
    }
}

/// 401 unless `Authorization: Bearer <token>` matches (scheme case-insensitive,
/// token exact and compared in constant time).
async fn require_token(State(token): State<Arc<str>>, request: Request, next: Next) -> Response {
    let presented = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.as_bytes().split_at_checked(7))
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case(b"bearer "))
        .map(|(_, presented)| presented);
    match presented {
        Some(presented) if bool::from(presented.ct_eq(token.as_bytes())) => next.run(request).await,
        _ => (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer")],
            "unauthorized",
        )
            .into_response(),
    }
}

/// Store the app's context: any JSON object, replaced wholesale.
async fn put_app_context(State(server): State<FunctorMcp>, body: Bytes) -> Response {
    match serde_json::from_slice::<Value>(&body) {
        Ok(context @ Value::Object(_)) => {
            server.set_app_context(context);
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(_) => (StatusCode::BAD_REQUEST, "the app context must be a JSON object").into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, format!("invalid JSON: {error}")).into_response(),
    }
}

/// Resolve when stdin reaches EOF — the parent closed it or died — so a child
/// server never outlives its host — so a host must pipe stdin and hold it
/// open (an inherited `/dev/null` stops the server at once). Not watched on a terminal (interactive use
/// stops with Ctrl-C). A plain thread, since a tokio stdin read cannot be
/// cancelled and would stall runtime shutdown after a signal.
async fn stdin_closed() {
    if io::stdin().is_terminal() {
        return std::future::pending().await;
    }
    let (closed, wait) = tokio::sync::oneshot::channel::<()>();
    std::thread::spawn(move || {
        let mut buffer = [0u8; 512];
        loop {
            match io::stdin().read(&mut buffer) {
                Ok(0) => break,
                Err(error) if error.kind() != io::ErrorKind::Interrupted => break,
                _ => {}
            }
        }
        let _ = closed.send(());
    });
    let _ = wait.await;
    eprintln!("[mcp] stdin closed; shutting down");
}
