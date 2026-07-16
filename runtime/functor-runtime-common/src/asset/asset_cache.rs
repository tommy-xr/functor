use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{Arc, Mutex},
};

use crate::io::load_bytes_async2;

use super::{AssetHandle, AssetPipeline, AssetPipelineContext, BuiltAssetPipeline};

/// A snapshot of asset loading, the data behind `Sub.assets`: how many
/// distinct assets have been referenced, how many are decoded, and which
/// byte-loads failed (missing file, HTTP error). `loaded == total` is "all
/// settled" — a loading screen's gate. Decode failures are NOT here: the
/// pipelines fall back (checkerboard / empty model) and count as loaded.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct AssetProgress {
    pub loaded: usize,
    pub total: usize,
    /// (path, error) per failed byte-load, in stable path order.
    pub failed: Vec<(String, String)>,
}

#[derive(Default)]
struct ProgressState {
    /// Every path a load was ever started for (`total`).
    started: HashSet<String>,
    /// Paths whose bytes loaded AND decoded (`loaded`).
    loaded: HashSet<String>,
    /// Byte-load failures, keyed by path (BTreeMap for stable order).
    failed: BTreeMap<String, String>,
}

pub struct AssetCache {
    bytes_cache: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    progress: Arc<Mutex<ProgressState>>,
}

impl AssetCache {
    pub fn new() -> AssetCache {
        AssetCache {
            bytes_cache: Arc::new(Mutex::new(HashMap::new())),
            progress: Arc::new(Mutex::new(ProgressState::default())),
        }
    }

    /// The current loading snapshot (see [`AssetProgress`]).
    pub fn progress(&self) -> AssetProgress {
        let state = self.progress.lock().unwrap();
        AssetProgress {
            loaded: state.loaded.len(),
            total: state.started.len(),
            failed: state
                .failed
                .iter()
                .map(|(path, err)| (path.clone(), err.clone()))
                .collect(),
        }
    }

    /// Every asset path with loaded bytes — the set the asset hot-reload
    /// watcher stats each frame.
    pub fn loaded_paths(&self) -> Vec<String> {
        self.bytes_cache.lock().unwrap().keys().cloned().collect()
    }

    /// Forget the cached bytes for `path` so the next load re-reads the file
    /// (asset hot-reload). Pipelines cache decoded handles separately — evict
    /// there too (`SceneContext::evict_asset` does both). The path counts as
    /// pending again in `progress()` until the reload settles.
    pub fn evict(&self, path: &str) {
        self.bytes_cache.lock().unwrap().remove(path);
        let mut progress = self.progress.lock().unwrap();
        progress.loaded.remove(path);
        progress.failed.remove(path);
    }

    pub fn load_asset_with_pipeline<T: 'static>(
        self: &Arc<Self>,
        pipeline: Arc<BuiltAssetPipeline<T>>,
        asset_path: &str,
    ) -> Arc<AssetHandle<T>> {
        // First - does the pipeline already have a reference to this?
        if let Some(asset) = pipeline.clone().get_opt(asset_path) {
            return asset.clone();
        }

        let asset_path_owned = asset_path.to_string();
        let bytes_cache = self.bytes_cache.clone();

        let self_arc = Arc::clone(self);
        self.progress
            .lock()
            .unwrap()
            .started
            .insert(asset_path_owned.clone());
        let progress = self.progress.clone();

        // Check cache
        if let Some(cached_bytes) = self.bytes_cache.lock().unwrap().get(&asset_path_owned) {
            // TODO: Can we avoid the clone here
            let bytes = cached_bytes.clone();

            let pipeline = pipeline.clone();
            let context = super::AssetPipelineContext {};
            let default_asset = pipeline.unloaded_asset(context);

            let context = super::AssetPipelineContext {};
            let pipeline = pipeline.clone();
            // If bytes are already cached, return a handle immediately
            let future = async move {
                let decoded = Arc::new(pipeline.process(bytes, self_arc.as_ref(), context));
                progress.lock().unwrap().loaded.insert(asset_path_owned);
                Ok(decoded)
            };
            return Arc::new(AssetHandle::new(future, Arc::new(default_asset)));
        }

        let context = AssetPipelineContext {};
        let pipeline = pipeline.clone();
        let default_asset = pipeline.unloaded_asset(context);

        let outer_pipeline = pipeline.clone();
        // If not cached, load bytes asynchronously
        let bytes_future = async move {
            let bytes = match load_bytes_async2(asset_path_owned.clone()).await {
                Ok(bytes) => bytes,
                Err(e) => {
                    progress
                        .lock()
                        .unwrap()
                        .failed
                        .insert(asset_path_owned, e.clone());
                    return Err(e);
                }
            };
            // Region-aware debug line (see docs/cli-output.md): under the CLI's
            // logger this becomes an `Event::Log` printed above the live panel;
            // silent unless `-v`/`RUST_LOG=debug`. Fires once per asset (on the
            // first load), not on the per-frame hot path.
            log::debug!(
                "loaded asset '{}' ({} bytes)",
                asset_path_owned,
                bytes.len()
            );
            bytes_cache
                .lock()
                .unwrap()
                .insert(asset_path_owned.clone(), bytes.clone());

            let pipeline = pipeline.clone();
            let context = AssetPipelineContext {};
            let decoded = Arc::new(pipeline.process(bytes, self_arc.as_ref(), context));
            progress.lock().unwrap().loaded.insert(asset_path_owned);
            Ok(decoded)
        };

        let handle = Arc::new(AssetHandle::new(bytes_future, Arc::new(default_asset)));
        outer_pipeline.cache(asset_path, handle.clone());
        handle
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset::AssetPollState;

    struct BytesLen;
    impl AssetPipeline<usize> for BytesLen {
        fn process(&self, bytes: Vec<u8>, _: &AssetCache, _: AssetPipelineContext) -> usize {
            bytes.len()
        }
        fn unloaded_asset(&self, _: AssetPipelineContext) -> usize {
            0
        }
    }

    fn temp_file(name: &str, contents: &[u8]) -> String {
        let path = std::env::temp_dir().join(format!(
            "functor-asset-progress-{}-{}",
            std::process::id(),
            name
        ));
        std::fs::write(&path, contents).unwrap();
        path.to_string_lossy().to_string()
    }

    /// Drive a handle to settled the way the render loop does.
    fn settle<T>(handle: &AssetHandle<T>) {
        for _ in 0..100 {
            match handle.poll_state() {
                AssetPollState::Loading => {}
                _ => return,
            }
        }
        panic!("asset did not settle");
    }

    #[test]
    fn progress_counts_loads_failures_and_evictions() {
        let cache = Arc::new(AssetCache::new());
        let pipeline = super::super::build_pipeline(Box::new(BytesLen));
        assert_eq!(cache.progress(), AssetProgress::default());

        // A real file loads: 1/1, no failures.
        let good = temp_file("good.bin", b"12345");
        settle(&cache.load_asset_with_pipeline(pipeline.clone(), &good));
        let progress = cache.progress();
        assert_eq!((progress.loaded, progress.total), (1, 1));
        assert!(progress.failed.is_empty());

        // A missing file fails: 1/2 with the path in `failed`.
        let missing = "does/not/exist.bin";
        settle(&cache.load_asset_with_pipeline(pipeline.clone(), missing));
        let progress = cache.progress();
        assert_eq!((progress.loaded, progress.total), (1, 2));
        assert_eq!(progress.failed.len(), 1);
        assert_eq!(progress.failed[0].0, missing);

        // Re-requesting settled assets never double-counts.
        settle(&cache.load_asset_with_pipeline(pipeline.clone(), &good));
        let progress = cache.progress();
        assert_eq!((progress.loaded, progress.total), (1, 2));

        // Eviction (hot-reload) makes the path pending again...
        cache.evict(&good);
        pipeline.evict(&good);
        let progress = cache.progress();
        assert_eq!((progress.loaded, progress.total), (0, 2));
        // ...until the reload settles.
        settle(&cache.load_asset_with_pipeline(pipeline.clone(), &good));
        let progress = cache.progress();
        assert_eq!((progress.loaded, progress.total), (1, 2));

        let _ = std::fs::remove_file(&good);
    }
}
