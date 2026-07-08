use std::io;

use warp::{http::Response, Filter};

pub struct WasmDevServer;

const INDEX_MLE_HTML: &str = include_str!("../../../runtime/functor-runtime-web/index-mle.html");
const WASM_FILE: &[u8] =
    include_bytes!("../../../runtime/functor-runtime-web/pkg/functor_runtime_web_bg.wasm");
const JS_FILE_1: &[u8] =
    include_bytes!("../../../runtime/functor-runtime-web/pkg/functor_runtime_web.js");

/// The standard inline-script defense: JSON escaping is not HTML escaping, so a
/// `</script>` inside a substituted literal would terminate the page's script
/// block. `<\/` is `/` in a JS string, harmless to the HTML parser.
fn script_safe(json: String) -> String {
    json.replace("</", "<\\/")
}

/// Substitute the project's MLE entry + full file list into the MLE index page:
///
/// - `"__MLE_ENTRY__"` becomes a JSON string literal → `window.__mleGamePath`,
///   the program root.
/// - `"__MLE_PROJECT_FILES__"` becomes a JSON array literal (entry first, then
///   siblings) → `window.__mleProjectFiles`, so multi-file games (`file =
///   module`) load EVERY module, not just the entry (docs/mle.md Track C5).
///
/// Both are valid JS for any path (quotes/backslashes included).
fn render_mle_index(entry: &str, files: &[String]) -> String {
    let entry_literal =
        script_safe(serde_json::to_string(entry).expect("a string always serializes"));
    let files_literal =
        script_safe(serde_json::to_string(files).expect("a string slice always serializes"));
    INDEX_MLE_HTML
        .replace("\"__MLE_ENTRY__\"", &entry_literal)
        .replace("\"__MLE_PROJECT_FILES__\"", &files_literal)
}

/// The project's file list as URLs relative to the served directory (entry
/// first, then sibling `.mle`/`.mlei` files) — the same set the desktop
/// producer links (`mle::project::project_files`), made relative so the web
/// runtime fetches each from the dev server's filesystem route. Falls back to
/// just the entry if the directory can't be scanned.
fn project_file_urls(working_directory: &str, entry: &str) -> Vec<String> {
    let entry_path = std::path::Path::new(working_directory).join(entry);
    let paths = match mle::project::project_files(&entry_path) {
        Ok(paths) => paths,
        Err(_) => return vec![entry.to_string()],
    };
    let root = std::path::Path::new(working_directory);
    paths
        .iter()
        .map(|p| {
            p.strip_prefix(root)
                .unwrap_or(p)
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect()
}

impl WasmDevServer {
    /// Serve an MLE project (docs/mle.md Track C5): same embedded runtime
    /// bundle + filesystem routes, but the index page is the MLE one — there
    /// is no game wasm module; the runtime fetches the entry file, which the
    /// filesystem route serves straight from the project directory.
    pub async fn start_mle(working_directory: &str, entry: &str) -> Result<(), io::Error> {
        let files = project_file_urls(working_directory, entry);
        Self::serve(
            working_directory,
            render_mle_index(entry, &files).into_bytes(),
        )
        .await
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

        // Combine all routes. `no-store` on everything: this is a dev server,
        // and the index page + `.mle` source are re-generated per run — without
        // it, switching samples (each serving its source at the same
        // `/game.mle` URL) can show a stale game from the browser cache.
        let static_routes = route_index.or(route_js1).or(route_wasm);
        let routes = static_routes
            .or(route_filesystem)
            .with(warp::reply::with::header("cache-control", "no-store"));

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
        let html = render_mle_index("game.mle", &["game.mle".to_string()]);
        assert!(html.contains("window.__mleGamePath = \"game.mle\""));
        assert!(!html.contains("__MLE_ENTRY__"));
    }

    #[test]
    fn substitutes_the_project_file_list_as_a_js_array() {
        let html = render_mle_index(
            "game.mle",
            &["game.mle".to_string(), "pieces.mle".to_string()],
        );
        assert!(html.contains("(["));
        assert!(html.contains("\"game.mle\",\"pieces.mle\""));
        assert!(!html.contains("__MLE_PROJECT_FILES__"));
    }

    #[test]
    fn escapes_entries_that_would_break_the_script() {
        let html = render_mle_index("we\"ird\\name.mle", &["we\"ird\\name.mle".to_string()]);
        assert!(html.contains("we\\\"ird\\\\name.mle"));
    }

    #[test]
    fn escapes_a_script_terminator_in_the_entry() {
        let html = render_mle_index("bad</script>.mle", &["bad</script>.mle".to_string()]);
        assert!(html.contains("bad<\\/script>.mle"));
    }
}
