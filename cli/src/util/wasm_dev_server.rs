use std::io;

use warp::{http::Response, Filter};

pub struct WasmDevServer;

const INDEX_HTML: &[u8] = include_bytes!("../../../runtime/functor-runtime-web/index.html");
const JS_FILE_1: &[u8] =
    include_bytes!("../../../runtime/functor-runtime-web/pkg/functor_runtime_web_bg.js");
const WASM_FILE: &[u8] =
    include_bytes!("../../../runtime/functor-runtime-web/pkg/functor_runtime_web_bg.wasm");
const JS_FILE_2: &[u8] =
    include_bytes!("../../../runtime/functor-runtime-web/pkg/functor_runtime_web.js");

impl WasmDevServer {
    pub async fn start(working_directory: &str) -> Result<(), io::Error> {
        let wd = working_directory.to_owned();
        // Match any request and return hello world!
        // let routes = warp::any().map(|| "Hello, World!");

        println!("Starting dev server");

        fn create_route(
            path: &'static str,
            content: &'static [u8],
            content_type: &'static str,
        ) -> warp::filters::BoxedFilter<(impl warp::Reply,)> {
            warp::path(path)
                .map(move || {
                    Response::builder()
                        .header("Content-Type", content_type)
                        .body(content.to_vec())
                })
                .boxed()
        }

        // Define routes for each file
        let route_index = create_route("index.html", INDEX_HTML, "text/html");
        let route_js1 = warp::path!("pkg" / "functor_runtime_web_bg.js")
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
        let route_js2 = warp::path!("pkg" / "functor_runtime_web.js")
            .map(|| {
                Response::builder()
                    .header("Content-Type", "application/javascript")
                    .body(JS_FILE_2.to_vec())
            })
            .boxed();

        // Route to serve files from the specified working directory
        let route_filesystem = warp::fs::dir(wd);

        // Combine all routes
        let static_routes = route_index.or(route_js1).or(route_wasm).or(route_js2);
        let routes = static_routes.or(route_filesystem);

        warp::serve(routes).run(([127, 0, 0, 1], 8080)).await;
        Ok(())
    }
}
