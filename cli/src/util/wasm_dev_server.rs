use std::io;

use warp::{http::Response, Filter};

pub struct WasmDevServer;

const INDEX_MLE_HTML: &str = include_str!("../../../runtime/functor-runtime-web/index-mle.html");
const WASM_FILE: &[u8] =
    include_bytes!("../../../runtime/functor-runtime-web/pkg/functor_runtime_web_bg.wasm");
const JS_FILE_1: &[u8] =
    include_bytes!("../../../runtime/functor-runtime-web/pkg/functor_runtime_web.js");

/// Substitute the project's MLE entry file into the MLE index page: the quoted
/// placeholder `"__MLE_ENTRY__"` becomes a JSON string literal (valid JS for
/// any entry path, quotes and backslashes included). The page assigns it to
/// `window.__mleGamePath`, which tells the web runtime to fetch + interpret
/// that source (docs/mle.md Track C5).
fn render_mle_index(entry: &str) -> String {
    let literal = serde_json::to_string(entry)
        .expect("a string always serializes")
        // JSON escaping is not HTML escaping: `</script>` inside the literal
        // would terminate the page's script block. `<\/` is the standard
        // inline-script defense (`\/` is `/` in a JS string).
        .replace("</", "<\\/");
    INDEX_MLE_HTML.replace("\"__MLE_ENTRY__\"", &literal)
}

impl WasmDevServer {
    /// Serve an MLE project (docs/mle.md Track C5): same embedded runtime
    /// bundle + filesystem routes, but the index page is the MLE one — there
    /// is no game wasm module; the runtime fetches the entry file, which the
    /// filesystem route serves straight from the project directory.
    pub async fn start_mle(working_directory: &str, entry: &str) -> Result<(), io::Error> {
        Self::serve(working_directory, render_mle_index(entry).into_bytes()).await
    }

    async fn serve(working_directory: &str, index_html: Vec<u8>) -> Result<(), io::Error> {
        let wd = working_directory.to_owned();

        // Define routes for each file
        let route_index = warp::path::end()
            .map(move || {
                Response::builder()
                    .header("Content-Type", "text/html")
                    .body(index_html.clone())
            })
            .boxed();

        let route_js1 = warp::path!("pkg" / "functor_runtime_web.js")
            .map(|| {
                Response::builder()
                    .header("Content-Type", "application/javascript")
                    .body(JS_FILE_1.to_vec())
            })
            .boxed();
        let route_wasm = warp::path!("pkg" / "functor_runtime_web_bg.wasm")
            .map(|| {
                Response::builder()
                    .header("Content-Type", "application/wasm")
                    .body(WASM_FILE.to_vec())
            })
            .boxed();

        // Route to serve files from the specified working directory
        let route_filesystem = warp::fs::dir(wd);

        // Combine all routes
        let static_routes = route_index.or(route_js1).or(route_wasm);
        let routes = static_routes.or(route_filesystem);

        // Bind first, then announce — so `ServerListening` never claims a port
        // that isn't actually accepting connections, and a bind failure (port
        // in use) surfaces as an error event instead of a panic.
        let (addr, server) = warp::serve(routes)
            .try_bind_ephemeral(([127, 0, 0, 1], 8080))
            .map_err(|e| {
                io::Error::other(format!("cannot bind dev server to 127.0.0.1:8080: {e}"))
            })?;
        crate::output::emit(crate::output::Event::ServerListening {
            url: format!("http://{addr}"),
        });
        server.await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::render_mle_index;

    #[test]
    fn substitutes_the_entry_as_a_js_string() {
        let html = render_mle_index("game.mle");
        assert!(html.contains("window.__mleGamePath = \"game.mle\""));
        assert!(!html.contains("__MLE_ENTRY__"));
    }

    #[test]
    fn escapes_entries_that_would_break_the_script() {
        let html = render_mle_index("we\"ird\\name.mle");
        assert!(html.contains("we\\\"ird\\\\name.mle"));
    }

    #[test]
    fn escapes_a_script_terminator_in_the_entry() {
        let html = render_mle_index("bad</script>.mle");
        assert!(html.contains("bad<\\/script>.mle"));
    }
}
