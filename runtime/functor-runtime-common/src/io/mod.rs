pub async fn load_bytes_async(path: &str) -> Result<Vec<u8>, String> {
    #[cfg(target_arch = "wasm32")]
    {
        use js_sys::Uint8Array;
        use wasm_bindgen::JsCast;
        use wasm_bindgen_futures::JsFuture;
        use web_sys::{Request, RequestInit, Response};

        let mut opts = RequestInit::new();
        opts.method("GET");

        let request = Request::new_with_str_and_init(path, &opts)
            .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".into()))?;
        let window = web_sys::window().ok_or("No global `window` exists")?;
        let response_value = JsFuture::from(window.fetch_with_request(&request))
            .await
            .map_err(|e| e.as_string().unwrap_or_else(|| "Unknown error".into()))?;
        let response: Response = response_value
            .dyn_into()
            .map_err(|_| "Failed to cast to Response")?;

        // A missing asset must FAIL (like the native `File::open` path), not
        // return the 404 page's body as if it were the asset — otherwise the
        // glTF/PNG parser downstream chokes on that HTML and panics. Returning
        // Err here routes into the asset system's fallback (empty) asset instead.
        if !response.ok() {
            return Err(format!("{}: HTTP {}", path, response.status()));
        }

        let array_buffer = JsFuture::from(
            response
                .array_buffer()
                .map_err(|_| "Couldn't convert response to array buffer")?,
        )
        .await
        .map_err(|_| "Failed to convert to array buffer")?;
        let array = Uint8Array::new(&array_buffer);
        Ok(array.to_vec())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use tokio::fs::File;
        use tokio::io::AsyncReadExt;

        let mut file = File::open(path).await.map_err(|e| e.to_string())?;
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer)
            .await
            .map_err(|e| e.to_string())?;
        Ok(buffer)
    }
}

pub async fn load_bytes_async2(path: String) -> Result<Vec<u8>, String> {
    #[cfg(target_arch = "wasm32")]
    {
        // fetch() takes relative paths (bundle-served files) and absolute
        // http(s) URLs (CDN assets, CORS permitting) alike — no branch needed
        // for the transfer itself...
        let bytes = load_bytes_async(&path)
            .await
            .map_err(|e| format!("{}: {}", path, e))?;
        // ...but remote responses get the same soft-404 guard as native: a
        // CDN answering HTTP 200 with an HTML error page must not reach the
        // glTF/PNG parsers (they panic on garbage) — it fails the load, which
        // routes to the fallback asset + an AssetError event instead. Local
        // bundle files skip this (the pipelines sniff content, not extension,
        // so a mislabeled-but-valid local file keeps working).
        if is_remote_path(&path) {
            verify_magic(&path, &bytes).map_err(|e| format!("{}: {}", path, e))?;
        }
        Ok(bytes)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let bytes = if is_remote_path(&path) {
            // Remote (URL) assets download off-thread through the shell's
            // installed fetcher and land over a channel — the future stays
            // Pending until the download completes, so a CDN transfer never
            // stalls the render thread.
            remote::fetch(&path)
                .await
                .map_err(|e| format!("{}: {}", path, e))?
        } else {
            // AssetHandle polls asset futures manually with a noop waker, once
            // per frame on the render thread. A chunked tokio::fs read only
            // advances one chunk per poll under that scheme, so large assets
            // took minutes to load natively (a 47MB glb ≈ thousands of
            // frames). Until assets are driven by a real executor (see
            // docs/todo.md "async inbox"), read synchronously: one frame hitch
            // instead of a minutes-long stall.
            std::fs::read(&path).map_err(|e| format!("{}: {}", path, e))?
        };
        // Simulated slow network (dev): FUNCTOR_THROTTLE_ASSETS=<KB/s> holds
        // the loaded bytes until size/rate has elapsed, so loading UX
        // (Sub.assets progress bars, fallback placeholders) is actually
        // visible on a machine where every load is otherwise instant.
        if let Some(kbps) = throttle_kbps() {
            // Clamp to a day: an absurdly small rate must not overflow
            // Duration (a panic on the render thread) — it just hangs the
            // load, which is what the user asked for.
            let seconds = (bytes.len() as f64 / (kbps * 1024.0)).min(24.0 * 3600.0);
            throttle::DelayUntil {
                deadline: std::time::Instant::now() + std::time::Duration::from_secs_f64(seconds),
            }
            .await;
        }
        Ok(bytes)
    }
}

/// See the FUNCTOR_THROTTLE_ASSETS comment in [`load_bytes_async2`].
#[cfg(not(target_arch = "wasm32"))]
fn throttle_kbps() -> Option<f64> {
    std::env::var("FUNCTOR_THROTTLE_ASSETS")
        .ok()?
        .parse::<f64>()
        .ok()
        .filter(|kbps| *kbps > 0.0)
}

#[cfg(not(target_arch = "wasm32"))]
mod throttle {
    use std::{
        future::Future,
        pin::Pin,
        task::{Context, Poll},
        time::Instant,
    };

    /// Pending until the deadline passes. Never registers a waker — like the
    /// asset futures it delays, it relies on the once-per-frame manual polling
    /// in `AssetHandle::poll_load` (a parked executor would hang on it).
    pub struct DelayUntil {
        pub deadline: Instant,
    }

    impl Future for DelayUntil {
        type Output = ();

        fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
            if Instant::now() >= self.deadline {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        }
    }
}

/// True when an asset path names a remote resource (an absolute http/https URL)
/// rather than a file in the project directory. Mirrors the `://` rule the wasm
/// bundle-export lint uses to classify a reference as remote.
pub fn is_remote_path(path: &str) -> bool {
    path.starts_with("http://") || path.starts_with("https://")
}

/// Cheap magic-byte verification for the formats whose URLs we can recognize
/// by extension, applied to every REMOTE asset body on both targets (natively
/// in `remote::fetch`, on wasm after the browser fetch). Unknown extensions
/// pass — this is a poisoning/soft-404 guard, not a general validator.
fn verify_magic(url: &str, bytes: &[u8]) -> Result<(), String> {
    // HTML is never a valid asset of ANY type, so an HTML body — the classic
    // CDN error page — is rejected regardless of extension. This covers
    // extensionless/API-style asset URLs the per-format checks below can't.
    let stripped = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let body = stripped
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .map(|start| &stripped[start..])
        .unwrap_or(&[]);
    let html = [&b"<!doctype"[..], &b"<html"[..]]
        .iter()
        .any(|prefix| body.len() >= prefix.len() && body[..prefix.len()].eq_ignore_ascii_case(prefix));
    if html {
        return Err("response body is an HTML page, not an asset — is the URL an error page?"
            .to_string());
    }
    let path_part = url.split(['?', '#']).next().unwrap_or(url);
    let ext = std::path::Path::new(path_part)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let ok = match ext.as_str() {
        "glb" => bytes.starts_with(b"glTF"),
        // A .gltf is JSON: first non-whitespace byte is '{' (some exporters
        // prefix a UTF-8 BOM — skip it, it's still JSON).
        "gltf" => bytes
            .strip_prefix(&[0xEF, 0xBB, 0xBF])
            .unwrap_or(bytes)
            .iter()
            .find(|b| !b.is_ascii_whitespace())
            .map(|b| *b == b'{')
            .unwrap_or(false),
        "png" => bytes.starts_with(&[0x89, b'P', b'N', b'G']),
        "jpg" | "jpeg" => bytes.starts_with(&[0xFF, 0xD8]),
        _ => true,
    };
    if ok {
        Ok(())
    } else {
        let head: Vec<u8> = bytes.iter().take(12).copied().collect();
        Err(format!(
            "response body is not {} (starts with {:?}) — is the URL an error page?",
            ext,
            String::from_utf8_lossy(&head)
        ))
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use remote::{
    fetch_cached_blocking, remote_cache_hit, set_remote_fetcher, RemoteFetchResult,
    RemoteFetchSender,
};

/// Remote (URL) asset fetching for the native runtime.
///
/// The shell installs a fetcher via [`set_remote_fetcher`] (desktop: reqwest on
/// its tokio runtime); the download itself runs off-thread and lands over a
/// oneshot channel. Verified bytes go to a disk cache (`~/.functor/cache`,
/// overridable with `FUNCTOR_ASSET_CACHE`) so a CDN asset downloads once per
/// machine, not once per run. Cache reads/writes are synchronous on the polling
/// thread — the same accepted one-frame hitch as local file reads (see the
/// load_bytes_async2 comment); the network transfer is what must never stall a
/// frame, and doesn't.
#[cfg(not(target_arch = "wasm32"))]
mod remote {
    use std::{
        path::{Path, PathBuf},
        sync::RwLock,
    };

    use super::verify_magic;

    pub type RemoteFetchResult = Result<Vec<u8>, String>;
    pub type RemoteFetchSender = futures::channel::oneshot::Sender<RemoteFetchResult>;

    type Fetcher = Box<dyn Fn(String, RemoteFetchSender) + Send + Sync>;

    static FETCHER: RwLock<Option<Fetcher>> = RwLock::new(None);

    /// Install the host's remote fetcher: called with a URL and a oneshot
    /// sender, it must (eventually, from any thread) send exactly one result.
    /// Installing replaces any previous fetcher — each runner entry point
    /// installs its own, so a later run in the same process never holds a
    /// handle onto a torn-down runtime.
    pub fn set_remote_fetcher(f: impl Fn(String, RemoteFetchSender) + Send + Sync + 'static) {
        *FETCHER.write().unwrap() = Some(Box::new(f));
    }

    pub async fn fetch(url: &str) -> RemoteFetchResult {
        if let Some(bytes) = cache_read(url) {
            // A cached entry that fails the magic check (poisoned by a pre-fix
            // build, or corrupted) is treated as a miss and refetched.
            if verify_magic(url, &bytes).is_ok() {
                log::debug!("remote asset '{}' served from disk cache", url);
                return Ok(bytes);
            }
            log::debug!("remote asset '{}' cache entry invalid; refetching", url);
        }
        let (tx, rx) = futures::channel::oneshot::channel();
        {
            let fetcher = FETCHER.read().unwrap();
            let Some(fetcher) = fetcher.as_ref() else {
                return Err("remote assets are not supported in this host".to_string());
            };
            fetcher(url.to_string(), tx);
        }
        let bytes = rx
            .await
            .map_err(|_| "remote fetch worker disappeared".to_string())??;
        // Refuse to parse OR cache a body that isn't the format the URL claims
        // (a CDN "soft 404" — HTTP 200 with an HTML error page — would panic
        // the glTF pipeline this run and, if cached, on every later run too).
        verify_magic(url, &bytes)?;
        cache_write(url, &bytes);
        Ok(bytes)
    }

    /// Whether a VERIFIED disk-cache entry exists for `url` — the build-time
    /// existence check's fast path (no network touched).
    pub fn remote_cache_hit(url: &str) -> bool {
        cache_read(url).is_some_and(|bytes| verify_magic(url, &bytes).is_ok())
    }

    /// Synchronous check-cache → download → verify → store, for TOOLING (the
    /// CLI's `functor import` inspecting a sidecar-declared remote model for
    /// its clips). Same cache location, key, and verify rules as [`fetch`],
    /// so the entry this warms is the very one the game's later load hits.
    /// The `download` closure runs only on a cache miss — the CLI passes a
    /// blocking HTTP GET run on its own thread; this module stays
    /// transport-free (mirroring the [`set_remote_fetcher`] hook design).
    ///
    /// `validate` guards the cache for EXTENSIONLESS urls, where
    /// `verify_magic` can only reject HTML: without it, a JSON error body
    /// from `https://api/asset/123` would be cached and served forever, even
    /// after the server recovers. It runs before every store AND on every
    /// hit (a cached body failing it is a miss, refetched) — the caller
    /// passes its format check (e.g. "inspect_model parses this").
    pub fn fetch_cached_blocking(
        url: &str,
        download: impl FnOnce(&str) -> Result<Vec<u8>, String>,
        validate: impl Fn(&[u8]) -> Result<(), String>,
    ) -> Result<Vec<u8>, String> {
        if let Some(bytes) = cache_read(url) {
            // Same poisoned-entry rule as `fetch`: an invalid cached body is
            // a miss, refetched.
            if verify_magic(url, &bytes).is_ok() && validate(&bytes).is_ok() {
                return Ok(bytes);
            }
        }
        let bytes = download(url)?;
        verify_magic(url, &bytes)?;
        validate(&bytes)?;
        cache_write(url, &bytes);
        Ok(bytes)
    }

    fn cache_dir() -> Option<PathBuf> {
        if let Ok(dir) = std::env::var("FUNCTOR_ASSET_CACHE") {
            if !dir.is_empty() {
                return Some(PathBuf::from(dir));
            }
        }
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .ok()?;
        Some(Path::new(&home).join(".functor").join("cache"))
    }

    /// Cache file for a URL: hex sha256 of the URL itself (we key by locator,
    /// not content — the content hash isn't known until after the download).
    fn cache_path(url: &str) -> Option<PathBuf> {
        use sha2::{Digest, Sha256};
        let hash = Sha256::digest(url.as_bytes());
        Some(cache_dir()?.join(format!("{:x}", hash)))
    }

    fn cache_read(url: &str) -> Option<Vec<u8>> {
        std::fs::read(cache_path(url)?).ok()
    }

    /// Best-effort: a cache-write failure must never fail the load. Temp-file +
    /// rename so a concurrent reader never sees a torn file.
    fn cache_write(url: &str, bytes: &[u8]) {
        let Some(path) = cache_path(url) else { return };
        let Some(dir) = path.parent() else { return };
        if std::fs::create_dir_all(dir).is_err() {
            return;
        }
        // Per-process temp name: two functor processes fetching the same new
        // URL must not interleave writes into one shared temp file.
        let tmp = path.with_extension(format!("part-{}", std::process::id()));
        if std::fs::write(&tmp, bytes).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::{
            future::Future,
            pin::Pin,
            task::{Context, Poll},
        };

        fn poll_once<T>(fut: &mut Pin<Box<dyn Future<Output = T>>>) -> Poll<T> {
            let waker = futures::task::noop_waker();
            let mut cx = Context::from_waker(&waker);
            fut.as_mut().poll(&mut cx)
        }

        // glb magic + enough padding that the head-of-body error path (if hit)
        // is readable.
        const GLB: &[u8] = b"glTF2000fake-binary-payload";

        /// One test drives the whole flow (install → pending → resolve → disk
        /// cache hit) because the fetcher is process-global state and the cache
        /// dir comes from an env var — splitting it would race siblings.
        #[test]
        fn fetch_downloads_then_serves_from_disk_cache() {
            let cache = std::env::temp_dir().join(format!(
                "functor-remote-cache-test-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&cache);
            std::env::set_var("FUNCTOR_ASSET_CACHE", &cache);

            // The fetcher stashes the sender instead of answering, so the test
            // controls exactly when the "download" completes.
            let stash: std::sync::Arc<std::sync::Mutex<Option<RemoteFetchSender>>> =
                Default::default();
            let fetch_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            {
                let stash = stash.clone();
                let fetch_count = fetch_count.clone();
                set_remote_fetcher(move |_url, tx| {
                    fetch_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    *stash.lock().unwrap() = Some(tx);
                });
            }

            let url = "https://example.test/model.glb";
            let mut fut: Pin<Box<dyn Future<Output = RemoteFetchResult>>> =
                Box::pin(fetch(url));
            // Download in flight: the future parks without blocking.
            assert!(matches!(poll_once(&mut fut), Poll::Pending));
            assert!(matches!(poll_once(&mut fut), Poll::Pending));

            let tx = stash.lock().unwrap().take().unwrap();
            tx.send(Ok(GLB.to_vec())).unwrap();
            match poll_once(&mut fut) {
                Poll::Ready(Ok(bytes)) => assert_eq!(bytes, GLB),
                Poll::Ready(Err(e)) => panic!("expected Ok, got Err({e})"),
                Poll::Pending => panic!("expected Ready after send"),
            }

            // Second fetch: served from the disk cache, fetcher not called again.
            let mut fut2: Pin<Box<dyn Future<Output = RemoteFetchResult>>> =
                Box::pin(fetch(url));
            match poll_once(&mut fut2) {
                Poll::Ready(Ok(bytes)) => assert_eq!(bytes, GLB),
                _ => panic!("expected cached Ready(Ok(..))"),
            }
            assert_eq!(fetch_count.load(std::sync::atomic::Ordering::SeqCst), 1);

            // A body that isn't what the URL claims (CDN soft-404 HTML) is an
            // error and must NOT be cached.
            let url2 = "https://example.test/other.glb";
            let mut fut3: Pin<Box<dyn Future<Output = RemoteFetchResult>>> =
                Box::pin(fetch(url2));
            assert!(matches!(poll_once(&mut fut3), Poll::Pending));
            let tx = stash.lock().unwrap().take().unwrap();
            tx.send(Ok(b"<html>not found</html>".to_vec())).unwrap();
            match poll_once(&mut fut3) {
                // The extension-independent HTML rejection fires before the
                // per-format glb check for this body.
                Poll::Ready(Err(e)) => assert!(e.contains("HTML page"), "error was: {e}"),
                _ => panic!("expected Ready(Err(..)) for an HTML body"),
            }
            // Nothing new cached: only the good asset's entry exists.
            assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 1);

            // The synchronous tooling path (fetch_cached_blocking) shares the
            // same cache: miss → download closure runs once; hit → it doesn't;
            // an HTML body is an error and is not cached. In the same test
            // because the cache dir env var is process-global.
            let url3 = "https://example.test/tooling.glb";
            let calls = std::cell::Cell::new(0);
            let got = fetch_cached_blocking(
                url3,
                |_| {
                    calls.set(calls.get() + 1);
                    Ok(GLB.to_vec())
                },
                |_| Ok(()),
            );
            assert_eq!(got.unwrap(), GLB);
            assert_eq!(calls.get(), 1);
            let got = fetch_cached_blocking(
                url3,
                |_| {
                    calls.set(calls.get() + 1);
                    Ok(GLB.to_vec())
                },
                |_| Ok(()),
            );
            assert_eq!(got.unwrap(), GLB, "second call served from cache");
            assert_eq!(calls.get(), 1, "download closure must not run on a hit");
            // The caller's validator guards the cache where magic can't (an
            // extensionless url): a failing body is an error and NOT cached,
            // so the entry count stays put and recovery needs no manual
            // cache flush.
            let api_url = "https://example.test/api/asset/123";
            let bad = fetch_cached_blocking(
                api_url,
                |_| Ok(b"{\"error\":\"try later\"}".to_vec()),
                |_| Err("not a model".to_string()),
            );
            assert!(bad.is_err());
            let recovered = fetch_cached_blocking(
                api_url,
                |_| Ok(GLB.to_vec()),
                |bytes| {
                    if bytes.starts_with(b"glTF") {
                        Ok(())
                    } else {
                        Err("not a model".to_string())
                    }
                },
            );
            assert_eq!(recovered.unwrap(), GLB, "recovers once the server does");
            // And the ASYNC runtime path hits the tooling-warmed entry too —
            // the whole point: import warms exactly what the game later loads.
            let mut fut4: Pin<Box<dyn Future<Output = RemoteFetchResult>>> =
                Box::pin(fetch(url3));
            match poll_once(&mut fut4) {
                Poll::Ready(Ok(bytes)) => assert_eq!(bytes, GLB),
                _ => panic!("runtime fetch should hit the tooling-warmed cache entry"),
            }
            let poisoned = fetch_cached_blocking(
                "https://example.test/soft404.glb",
                |_| Ok(b"<html>not found</html>".to_vec()),
                |_| Ok(()),
            );
            assert!(poisoned.is_err());
            assert_eq!(
                std::fs::read_dir(&cache).unwrap().count(),
                3,
                "good entries only: model.glb + tooling.glb + the recovered api asset"
            );

            let _ = std::fs::remove_dir_all(&cache);
        }

        #[test]
        fn magic_verification_by_extension() {
            assert!(verify_magic("https://x/a.glb", GLB).is_ok());
            assert!(verify_magic("https://x/a.glb?v=2", GLB).is_ok());
            assert!(verify_magic("https://x/a.glb", b"<html>").is_err());
            assert!(verify_magic("https://x/a.gltf", b"  {\"asset\":{}}").is_ok());
            // A UTF-8 BOM before the JSON is still a valid .gltf.
            assert!(verify_magic("https://x/a.gltf", b"\xEF\xBB\xBF{\"asset\":{}}").is_ok());
            assert!(verify_magic("https://x/a.gltf", b"<html>").is_err());
            assert!(verify_magic("https://x/a.png", &[0x89, b'P', b'N', b'G', 1]).is_ok());
            assert!(verify_magic("https://x/a.png", b"nope").is_err());
            assert!(verify_magic("https://x/a.jpg", &[0xFF, 0xD8, 0xFF]).is_ok());
            // Unknown extensions pass through unverified...
            assert!(verify_magic("https://x/a.bin", b"anything").is_ok());
            assert!(verify_magic("https://x/noext", b"anything").is_ok());
            // ...EXCEPT an HTML body, which is never a valid asset of any
            // type — extensionless/API-style URLs get soft-404 protection too.
            assert!(verify_magic("https://x/api/asset/123", b"<!DOCTYPE html><html>").is_err());
            assert!(verify_magic("https://x/noext", b"\n  <html><body>err</body>").is_err());
            assert!(verify_magic("https://x/noext", b"\xEF\xBB\xBF<HTML>").is_err());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_paths_are_urls_only() {
        assert!(is_remote_path("https://assets.babylonjs.com/meshes/fish.glb"));
        assert!(is_remote_path("http://localhost:8080/crate.png"));
        assert!(!is_remote_path("crate.png"));
        assert!(!is_remote_path("models/fish.glb"));
        assert!(!is_remote_path("httpx://not-a-scheme.glb"));
    }
}
