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
    /// Paths whose bytes came from the debug protocol rather than disk/CDN.
    /// The final manifest of each sync removes uploads deleted on the host.
    uploaded_paths: Mutex<HashSet<String>>,
}

impl AssetCache {
    pub fn new() -> AssetCache {
        AssetCache {
            bytes_cache: Arc::new(Mutex::new(HashMap::new())),
            progress: Arc::new(Mutex::new(ProgressState::default())),
            uploaded_paths: Mutex::new(HashSet::new()),
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

    /// Clone bytes already resident in the shared cache without starting a
    /// filesystem/network load. Native audio uses this for remotely uploaded
    /// sounds; render pipelines use the async handle path below.
    pub fn cached_bytes(&self, path: &str) -> Option<Vec<u8>> {
        self.bytes_cache.lock().unwrap().get(path).cloned()
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

    /// Install bytes uploaded by a remote-development client. Returns whether
    /// the bytes changed and decoded pipeline handles therefore need eviction.
    ///
    /// Uploaded bytes use the exact locator as the cache key, so existing
    /// renderer code remains unaware of where they came from. A later draw
    /// decodes from this warm byte cache instead of touching Android's
    /// filesystem.
    pub fn replace_uploaded(&self, path: &str, bytes: Vec<u8>) -> bool {
        let changed = {
            let mut cache = self.bytes_cache.lock().unwrap();
            if cache.get(path).is_some_and(|current| *current == bytes) {
                false
            } else {
                cache.insert(path.to_string(), bytes);
                true
            }
        };
        self.uploaded_paths.lock().unwrap().insert(path.to_string());
        if changed {
            let mut progress = self.progress.lock().unwrap();
            progress.loaded.remove(path);
            progress.failed.remove(path);
        }
        changed
    }

    /// Complete an uploaded-project sync, forgetting assets that are absent
    /// from its current manifest. Returns removed locators so shells can evict
    /// their decoded model/texture/skybox handles too.
    pub fn retain_uploaded(&self, current: &HashSet<String>) -> Vec<String> {
        let removed = {
            let mut uploaded = self.uploaded_paths.lock().unwrap();
            let mut removed: Vec<String> = uploaded.difference(current).cloned().collect();
            removed.sort();
            uploaded.retain(|path| current.contains(path));
            removed
        };
        for path in &removed {
            self.bytes_cache.lock().unwrap().remove(path);
            let mut progress = self.progress.lock().unwrap();
            progress.started.remove(path);
            progress.loaded.remove(path);
            progress.failed.remove(path);
        }
        removed
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
            let inner_pipeline = pipeline.clone();
            // If bytes are already cached, return a handle immediately
            let future = async move {
                let decoded = Arc::new(inner_pipeline.process(bytes, self_arc.as_ref(), context));
                progress.lock().unwrap().loaded.insert(asset_path_owned);
                Ok(decoded)
            };
            let handle = Arc::new(AssetHandle::new(future, Arc::new(default_asset)));
            // Cache the handle like the cold path below does — without this,
            // every later request down this branch (bytes cached by ANOTHER
            // pipeline's load: e.g. preloaded as a model, then sampled as a
            // texture) minted a fresh handle and re-decoded per call.
            pipeline.cache(asset_path, handle.clone());
            return handle;
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
    match resolve_while_pending_state(cache, pipeline, primary, while_pending) {
        WhilePendingState::Loaded(asset) | WhilePendingState::Loading(Some(asset)) => asset,
        WhilePendingState::Loading(None) | WhilePendingState::Failed => primary.fallback(),
    }
}

/// Explicit state behind [`resolve_while_pending`]. Callers that hydrate
/// long-lived shell resources need to distinguish a loaded placeholder from
/// a loaded primary: the placeholder may be published, but the request must
/// remain active so the primary future keeps advancing.
#[derive(Clone, Debug, PartialEq)]
pub enum WhilePendingState<T> {
    Loading(Option<Arc<T>>),
    Loaded(Arc<T>),
    Failed,
}

pub fn resolve_while_pending_state<T: 'static>(
    cache: &Arc<AssetCache>,
    pipeline: &Arc<super::BuiltAssetPipeline<T>>,
    primary: &AssetHandle<T>,
    while_pending: &[String],
) -> WhilePendingState<T> {
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
        super::AssetPollState::Loaded(asset) => WhilePendingState::Loaded(asset),
        super::AssetPollState::Failed => WhilePendingState::Failed,
        super::AssetPollState::Loading => WhilePendingState::Loading(stand_in),
    }
}

/// Whether a placeholder in `while_pending` was started and still needs
/// polling. Long-lived shell requests must remain registered until this is
/// false even after the primary settles, or `Sub.assets` can be stranded.
pub fn while_pending_chain_is_unsettled(
    cache: &AssetCache,
    while_pending: &[String],
) -> bool {
    while_pending
        .iter()
        .any(|locator| cache.is_unsettled(locator))
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

    #[test]
    fn uploaded_bytes_replace_disk_loading_and_report_real_changes() {
        let cache = AssetCache::new();
        assert!(cache.replace_uploaded("textures/a.png", vec![1, 2, 3]));
        assert!(!cache.replace_uploaded("textures/a.png", vec![1, 2, 3]));
        assert!(cache.replace_uploaded("textures/a.png", vec![4, 5]));
        assert_eq!(
            cache
                .bytes_cache
                .lock()
                .unwrap()
                .get("textures/a.png")
                .cloned(),
            Some(vec![4, 5])
        );
    }

    #[test]
    fn uploaded_manifest_removes_deleted_assets_only() {
        let cache = AssetCache::new();
        cache.replace_uploaded("keep.glb", vec![1]);
        cache.replace_uploaded("delete.png", vec![2]);
        {
            let mut progress = cache.progress.lock().unwrap();
            progress.started.insert("delete.png".to_string());
            progress.loaded.insert("delete.png".to_string());
        }

        let current = HashSet::from(["keep.glb".to_string()]);
        assert_eq!(cache.retain_uploaded(&current), vec!["delete.png"]);
        let bytes = cache.bytes_cache.lock().unwrap();
        assert!(bytes.contains_key("keep.glb"));
        assert!(!bytes.contains_key("delete.png"));
        drop(bytes);
        assert_eq!(cache.progress(), AssetProgress::default());
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

    #[test]
    fn while_pending_state_keeps_placeholder_requests_alive_until_primary_promotes() {
        use std::{cell::Cell, rc::Rc, task::Poll};

        let cache = Arc::new(AssetCache::new());
        let pipeline = super::super::build_pipeline(Box::new(BytesLen));
        let placeholder = temp_file("wp-promote-placeholder.bin", b"12");
        let polls = Rc::new(Cell::new(0));
        let primary = AssetHandle::new(
            futures::future::poll_fn({
                let polls = polls.clone();
                move |_| {
                    if polls.replace(polls.get() + 1) == 0 {
                        Poll::Pending
                    } else {
                        Poll::Ready(Ok(Arc::new(7)))
                    }
                }
            }),
            Arc::new(0),
        );

        let first = resolve_while_pending_state(
            &cache,
            &pipeline,
            &primary,
            std::slice::from_ref(&placeholder),
        );
        assert!(matches!(
            first,
            WhilePendingState::Loading(Some(value)) if *value == 2
        ));

        let second = resolve_while_pending_state(
            &cache,
            &pipeline,
            &primary,
            std::slice::from_ref(&placeholder),
        );
        assert!(matches!(
            second,
            WhilePendingState::Loaded(value) if *value == 7
        ));

        let _ = std::fs::remove_file(placeholder);
    }

    #[test]
    fn settled_primary_reports_a_still_pending_started_placeholder() {
        let cache = Arc::new(AssetCache::new());
        let pipeline = super::super::build_pipeline(Box::new(BytesLen));
        let real = temp_file("wp-settled-primary.bin", b"1234567");
        let primary = cache.load_asset_with_pipeline(pipeline.clone(), &real);
        settle(&primary);

        let pending = "wp/pending-placeholder.bin".to_string();
        cache
            .progress
            .lock()
            .unwrap()
            .started
            .insert(pending.clone());
        pipeline.cache(
            &pending,
            Arc::new(AssetHandle::new(
                std::future::pending::<Result<Arc<usize>, String>>(),
                Arc::new(0),
            )),
        );
        assert!(matches!(
            resolve_while_pending_state(
                &cache,
                &pipeline,
                &primary,
                std::slice::from_ref(&pending)
            ),
            WhilePendingState::Loaded(value) if *value == 7
        ));
        assert!(while_pending_chain_is_unsettled(
            &cache,
            std::slice::from_ref(&pending)
        ));

        let _ = std::fs::remove_file(real);
    }

    /// The cached-bytes branch must CACHE its handle like the cold path: a
    /// path whose bytes another pipeline loaded (preloaded as a model, then
    /// requested as a texture) must hand back one stable handle, not a fresh
    /// re-decoding handle per call.
    #[test]
    fn cached_bytes_branch_caches_the_handle() {
        let cache = Arc::new(AssetCache::new());
        let pipeline_a = super::super::build_pipeline(Box::new(BytesLen));
        let pipeline_b = super::super::build_pipeline(Box::new(BytesLen));
        let path = temp_file("cross-pipeline.bin", b"12345");

        // Pipeline A's cold load populates the bytes cache.
        settle(&cache.load_asset_with_pipeline(pipeline_a.clone(), &path));

        // Pipeline B's first request goes down the cached-bytes branch...
        let first = cache.load_asset_with_pipeline(pipeline_b.clone(), &path);
        // ...and a second request must return the SAME handle, not a fresh
        // re-decoding one.
        let second = cache.load_asset_with_pipeline(pipeline_b.clone(), &path);
        assert!(Arc::ptr_eq(&first, &second), "handle must be cached");
        settle(&first);
        assert_eq!(*first.get(), 5);

        let _ = std::fs::remove_file(&path);
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
