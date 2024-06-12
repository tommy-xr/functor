use std::io;

use warp::{http::Response, Filter};

pub struct WasmDevServer;

const INDEX_FILE: &[u8; 658] = include_bytes!("../../../runtime/functor-runtime-web/index.html");

impl WasmDevServer {
    pub async fn start() -> Result<(), io::Error> {
        // Match any request and return hello world!
        // let routes = warp::any().map(|| "Hello, World!");

        let index_html = warp::path("index.html").map(move || {
            Response::builder()
                .header("Content-Type", "text/html")
                .body(INDEX_FILE.to_vec());
        });

        let routes = index_html;

        warp::serve(routes).run(([127, 0, 0, 1], 8080)).await;
        Ok(())
    }
}
