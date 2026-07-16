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
        // http(s) URLs (CDN assets, CORS permitting) alike — no branch needed.
        load_bytes_async(&path)
            .await
            .map_err(|e| format!("{}: {}", path, e))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        // Remote (URL) assets download off-thread through the shell's installed
        // fetcher and land over a channel — the future stays Pending until the
        // download completes, so a CDN transfer never stalls the render thread.
        if is_remote_path(&path) {
            return remote::fetch(&path).await.map_err(|e| format!("{}: {}", path, e));
        }
        // AssetHandle polls asset futures manually with a noop waker, once per
        // frame on the render thread. A chunked tokio::fs read only advances one
        // chunk per poll under that scheme, so large assets took minutes to load
        // natively (a 47MB glb ≈ thousands of frames). Until assets are driven by
        // a real executor (see docs/todo.md "async inbox"), read synchronously:
        // one frame hitch instead of a minutes-long stall.
        std::fs::read(&path).map_err(|e| format!("{}: {}", path, e))
    }
}

/// True when an asset path names a remote resource (an absolute http/https URL)
/// rather than a file in the project directory. Mirrors the `://` rule the wasm
/// bundle-export lint uses to classify a reference as remote.
pub fn is_remote_path(path: &str) -> bool {
    path.starts_with("http://") || path.starts_with("https://")
}

#[cfg(not(target_arch = "wasm32"))]
pub use remote::{set_remote_fetcher, RemoteFetchResult, RemoteFetchSender};

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

    /// Cheap magic-byte verification for the formats whose URLs we can
    /// recognize by extension. Unknown extensions pass — this is a poisoning
    /// guard, not a general validator.
    fn verify_magic(url: &str, bytes: &[u8]) -> Result<(), String> {
        let path_part = url.split(['?', '#']).next().unwrap_or(url);
        let ext = Path::new(path_part)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let ok = match ext.as_str() {
            "glb" => bytes.starts_with(b"glTF"),
            // A .gltf is JSON: first non-whitespace byte is '{'.
            "gltf" => bytes
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
                Poll::Ready(Err(e)) => assert!(e.contains("not glb"), "error was: {e}"),
                _ => panic!("expected Ready(Err(..)) for an HTML body"),
            }
            // Nothing new cached: only the good asset's entry exists.
            assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 1);

            let _ = std::fs::remove_dir_all(&cache);
        }

        #[test]
        fn magic_verification_by_extension() {
            assert!(verify_magic("https://x/a.glb", GLB).is_ok());
            assert!(verify_magic("https://x/a.glb?v=2", GLB).is_ok());
            assert!(verify_magic("https://x/a.glb", b"<html>").is_err());
            assert!(verify_magic("https://x/a.gltf", b"  {\"asset\":{}}").is_ok());
            assert!(verify_magic("https://x/a.gltf", b"<html>").is_err());
            assert!(verify_magic("https://x/a.png", &[0x89, b'P', b'N', b'G', 1]).is_ok());
            assert!(verify_magic("https://x/a.png", b"nope").is_err());
            assert!(verify_magic("https://x/a.jpg", &[0xFF, 0xD8, 0xFF]).is_ok());
            // Unknown extensions pass through unverified.
            assert!(verify_magic("https://x/a.bin", b"anything").is_ok());
            assert!(verify_magic("https://x/noext", b"anything").is_ok());
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
