use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{Arc, Mutex},
};

use crate::io::load_bytes_async2;

use super::{AssetHandle, AssetPipeline, AssetPipelineContext, BuiltAssetPipeline};

/// A snapshot of asset loading, the data behind `Sub.assets`: how many
/// distinct assets have been referenced, how many are decoded, and which
/// byte-loads failed (missing file, HTTP error). The "all settled" gate — a
/// loading screen's dismiss condition — is `total > 0 && loaded +
/// failed.len() == total`: failures never join `loaded`, and frame one can
/// deliver `0/0` before anything is referenced. Decode failures are NOT
/// here: the pipelines fall back (checkerboard / empty model) and count as
/// loaded.
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

    /// Whether `path` was started but has not yet settled (loaded or failed)
    /// — [`resolve_while_pending`]'s liveness check. A started load must
    /// keep being polled to completion (futures advance only when polled),
    /// or `Sub.assets`' settled gate (`loaded + failed == total`) never
    /// fires.
    pub fn is_unsettled(&self, path: &str) -> bool {
        let progress = self.progress.lock().unwrap();
        progress.started.contains(path)
            && !progress.loaded.contains(path)
            && !progress.failed.contains_key(path)
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

/// The asset to RENDER for a primary that carries an `Asset.whilePending`
/// chain: the primary when LOADED; while it is still loading, the first
/// LOADED chain entry; otherwise the pipeline fallback. A FAILED primary
/// falls back exactly like a chainless one — failure is not pending, and a
/// placeholder must not mask it (`Sub.assets` still reports it).
///
/// Liveness contract: asset futures only advance when POLLED, and this is
/// the only poll site for chain entries. So new entries are requested only
/// while the primary is pending (a warm-cache primary never touches its
/// placeholders), but every entry the cache has STARTED keeps being driven
/// here until it settles — even once the primary (or an earlier entry) has
/// landed, and even after a hot-reload eviction. An abandoned in-flight
/// placeholder would otherwise sit in `Sub.assets`' `total` forever and
/// hold the settled gate (`loaded + failed == total`) open.
pub fn resolve_while_pending<T: 'static>(
    cache: &Arc<AssetCache>,
    pipeline: &Arc<super::BuiltAssetPipeline<T>>,
    primary: &AssetHandle<T>,
    while_pending: &[String],
) -> Arc<T> {
    if while_pending.is_empty() {
        // The overwhelmingly common case: byte-identical to the old
        // single-poll `get()` path.
        return primary.get();
    }
    let primary_state = primary.poll_state();
    let pending = matches!(primary_state, super::AssetPollState::Loading);
    let mut stand_in: Option<Arc<T>> = None;
    for locator in while_pending {
        if pending || cache.is_unsettled(locator) {
            let handle = cache.load_asset_with_pipeline(pipeline.clone(), locator);
            if let super::AssetPollState::Loaded(asset) = handle.poll_state() {
                if pending && stand_in.is_none() {
                    stand_in = Some(asset);
                }
            }
        }
    }
    match primary_state {
        super::AssetPollState::Loaded(asset) => asset,
        super::AssetPollState::Failed => primary.fallback(),
        super::AssetPollState::Loading => stand_in.unwrap_or_else(|| primary.fallback()),
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

    /// `Asset.whilePending` resolution: the first LOADED chain entry stands
    /// in while the primary loads (failed entries skipped); a loaded primary
    /// wins outright; a FAILED primary keeps the fallback (a placeholder
    /// must not mask failure). Local reads settle on their first poll, so a
    /// genuinely-pending primary is a hand-built never-resolving handle.
    #[test]
    fn while_pending_resolves_placeholders_only_while_loading() {
        let cache = Arc::new(AssetCache::new());
        let pipeline = super::super::build_pipeline(Box::new(BytesLen));

        let pending_forever: AssetHandle<usize> = AssetHandle::new(
            std::future::pending::<Result<Arc<usize>, String>>(),
            Arc::new(0),
        );
        let placeholder = temp_file("wp-placeholder.bin", b"12");
        let real = temp_file("wp-real.bin", b"1234567");

        // Pending primary + a chain whose FIRST entry fails (missing file):
        // the failed entry is skipped and the loaded one stands in. The
        // resolve call itself requests the chain loads.
        let chain = vec!["wp/does-not-exist.bin".to_string(), placeholder.clone()];
        let shown = resolve_while_pending(&cache, &pipeline, &pending_forever, &chain);
        assert_eq!(*shown, 2, "loaded placeholder (2 bytes) stands in");

        // An empty chain on a pending primary keeps today's fallback.
        let shown = resolve_while_pending(&cache, &pipeline, &pending_forever, &[]);
        assert_eq!(*shown, 0, "no chain -> pipeline fallback");

        // A loaded primary wins even with a loaded placeholder available.
        let primary = cache.load_asset_with_pipeline(pipeline.clone(), &real);
        settle(&primary);
        let shown = resolve_while_pending(&cache, &pipeline, &primary, &chain);
        assert_eq!(*shown, 7, "loaded primary wins");

        // A FAILED primary falls back to the pipeline default (0), NOT the
        // placeholder — failure is not pending, and must stay visible.
        let failed = cache.load_asset_with_pipeline(pipeline.clone(), "wp/missing-primary.bin");
        settle(&failed);
        let shown = resolve_while_pending(&cache, &pipeline, &failed, &chain);
        assert_eq!(*shown, 0, "failed primary keeps the fallback");

        for f in [placeholder, real] {
            let _ = std::fs::remove_file(&f);
        }
    }

    /// Liveness: a STARTED chain entry keeps being driven to settled even
    /// once the primary has landed — an abandoned placeholder would sit in
    /// `Sub.assets`' total forever and hold the settled gate open. The
    /// synchronously-testable strand is hot-reload eviction: evicting an
    /// inactive placeholder leaves it started-but-not-loaded, and only the
    /// resolve call re-drives it.
    #[test]
    fn while_pending_drives_started_entries_to_settled() {
        let cache = Arc::new(AssetCache::new());
        let pipeline = super::super::build_pipeline(Box::new(BytesLen));

        let placeholder = temp_file("wp-live-placeholder.bin", b"12");
        let real = temp_file("wp-live-real.bin", b"1234567");
        let chain = vec![placeholder.clone()];

        // Pending primary requests + settles the placeholder (sync read).
        let pending_forever: AssetHandle<usize> = AssetHandle::new(
            std::future::pending::<Result<Arc<usize>, String>>(),
            Arc::new(0),
        );
        let _ = resolve_while_pending(&cache, &pipeline, &pending_forever, &chain);
        let progress = cache.progress();
        assert_eq!((progress.loaded, progress.total), (1, 1));

        // The primary lands; hot reload then evicts the (now inactive)
        // placeholder: started-but-unsettled — the strand.
        let primary = cache.load_asset_with_pipeline(pipeline.clone(), &real);
        settle(&primary);
        cache.evict(&placeholder);
        pipeline.evict(&placeholder);
        assert!(cache.is_unsettled(&placeholder));
        let progress = cache.progress();
        assert_eq!((progress.loaded, progress.total), (1, 2), "placeholder pending again");

        // The next resolve — primary LOADED, so no new placeholder is
        // NEEDED for display — still re-drives the started entry, and the
        // settled gate can fire again.
        let shown = resolve_while_pending(&cache, &pipeline, &primary, &chain);
        assert_eq!(*shown, 7, "primary still shown");
        let progress = cache.progress();
        assert_eq!(
            (progress.loaded, progress.total),
            (2, 2),
            "started placeholder driven back to settled"
        );
        assert!(!cache.is_unsettled(&placeholder));

        // An entry that was NEVER started is not requested once the primary
        // is loaded (placeholders stay lazy).
        let never = temp_file("wp-live-never.bin", b"123");
        let shown =
            resolve_while_pending(&cache, &pipeline, &primary, &[never.clone()]);
        assert_eq!(*shown, 7);
        let progress = cache.progress();
        assert_eq!(progress.total, 2, "never-needed placeholder never counted");

        for f in [placeholder, real, never] {
            let _ = std::fs::remove_file(&f);
        }
    }
}
