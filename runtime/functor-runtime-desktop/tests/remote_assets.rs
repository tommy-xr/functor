//! End-to-end remote asset loading, natively: a real localhost HTTP server, the
//! real desktop fetcher (reqwest on tokio), the real byte loader + disk cache,
//! and the real glTF model pipeline — everything except a GPU (hydration is
//! lazy, so decoding needs no GL). Hermetic: no external network.
//!
//! The wasm side of the same seam (fetch() with a URL) has no in-repo harness
//! yet — browser e2e is tracked as follow-up in the asset-handling plan.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use functor_runtime_common::asset::{
    build_pipeline, pipelines::ModelPipeline, AssetCache, AssetPollState,
};

/// The smallest valid .glb: header + a JSON chunk declaring an empty glTF 2.0
/// asset. Decodes to a Model with no meshes — the point is that the pipeline
/// PARSES it (reaching AssetPollState::Loaded), which garbage bytes would not.
fn minimal_glb() -> Vec<u8> {
    let mut json = br#"{"asset":{"version":"2.0"}}"#.to_vec();
    while json.len() % 4 != 0 {
        json.push(b' ');
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(12 + 8 + json.len() as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json);
    out
}

/// Serves /model.glb (a valid glb), /poison.glb (HTTP 200 but an HTML body —
/// the CDN "soft 404"), and 404 for everything else.
fn spawn_asset_server() -> u16 {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let port = server.server_addr().to_ip().unwrap().port();
    std::thread::spawn(move || {
        for req in server.incoming_requests() {
            let _ = match req.url() {
                "/model.glb" => req.respond(tiny_http::Response::from_data(minimal_glb())),
                "/poison.glb" => {
                    req.respond(tiny_http::Response::from_string("<html>oops</html>"))
                }
                _ => req.respond(
                    tiny_http::Response::from_string("not found").with_status_code(404),
                ),
            };
        }
    });
    port
}

/// Drive an AssetHandle the way the render loop does — repeated manual polls —
/// until it settles or the deadline passes.
async fn settle<T>(
    handle: &functor_runtime_common::asset::AssetHandle<T>,
) -> AssetPollState<T> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match handle.poll_state() {
            AssetPollState::Loading => {
                assert!(Instant::now() < deadline, "asset did not settle in 10s");
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            settled => return settled,
        }
    }
}

/// One test drives all three cases (loads / 404 / poison) because the fetcher
/// and the FUNCTOR_ASSET_CACHE env var are process-global — parallel test fns
/// would race them.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_glb_loads_end_to_end() {
    let cache_dir = std::env::temp_dir().join(format!(
        "functor-remote-e2e-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&cache_dir);
    std::env::set_var("FUNCTOR_ASSET_CACHE", &cache_dir);

    let port = spawn_asset_server();
    functor_runtime_desktop::install_remote_asset_fetcher(reqwest::Client::new());

    let asset_cache = Arc::new(AssetCache::new());
    let pipeline = build_pipeline(Box::new(ModelPipeline));

    // A real glb over real HTTP decodes to Loaded.
    let url = format!("http://127.0.0.1:{port}/model.glb");
    let handle = asset_cache.load_asset_with_pipeline(pipeline.clone(), &url);
    match settle(&handle).await {
        AssetPollState::Loaded(_) => {}
        _ => panic!("expected the remote glb to load"),
    }
    // ...and its verified bytes landed in the disk cache.
    assert_eq!(std::fs::read_dir(&cache_dir).unwrap().count(), 1);

    // A missing asset fails (fallback), it does not hang or panic.
    let handle = asset_cache.load_asset_with_pipeline(
        pipeline.clone(),
        &format!("http://127.0.0.1:{port}/missing.glb"),
    );
    assert!(matches!(settle(&handle).await, AssetPollState::Failed));

    // A 200-with-HTML body fails BEFORE the glTF parser (which would panic on
    // it) and is not cached.
    let handle = asset_cache.load_asset_with_pipeline(
        pipeline.clone(),
        &format!("http://127.0.0.1:{port}/poison.glb"),
    );
    assert!(matches!(settle(&handle).await, AssetPollState::Failed));
    assert_eq!(std::fs::read_dir(&cache_dir).unwrap().count(), 1);

    let _ = std::fs::remove_dir_all(&cache_dir);
}
