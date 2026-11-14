//! The pushed-file cache: project files a host has sent, kept on the device
//! by content hash so a file it already holds is not sent again — across
//! pushes, launches and projects. A push names its files and their hashes
//! first (`POST /push-manifest`); the device installs those it holds and
//! answers with the hashes it lacks; only those are uploaded, and each is
//! checked against its hash before it is kept (on a writer thread: the
//! upload does not wait for it).
//!
//! One file per entry, named by its sha256, written to a temporary name and
//! renamed: an interrupted write leaves no entry. The least recently used
//! entries go when the cache passes its cap.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// What the cache holds by default before evicting.
pub const DEFAULT_CAP_BYTES: u64 = 2 << 30;

/// One file of a push: `POST /push-manifest`'s body is `{"files": [...]}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestFile {
    /// Project-relative, as `/reload-asset` names it.
    pub path: String,
    pub size: u64,
    /// Lowercase hex sha256 of its bytes.
    pub hash: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub files: Vec<ManifestFile>,
}

/// `POST /push-manifest`'s answer.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestAnswer {
    /// The hashes the device lacks: upload the files that have them.
    pub missing: Vec<String>,
}

/// Lowercase hex sha256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

fn is_hash(name: &str) -> bool {
    name.len() == 64 && name.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub struct PushCache {
    dir: PathBuf,
    cap: u64,
    /// Path → hash for the push in flight: what an upload must hash to.
    expected: Mutex<HashMap<String, String>>,
    writer: Mutex<Option<std::sync::mpsc::Sender<Box<dyn FnOnce() + Send>>>>,
}

impl PushCache {
    pub fn new(dir: PathBuf, cap: u64) -> PushCache {
        PushCache { dir, cap, expected: Mutex::new(HashMap::new()), writer: Mutex::new(None) }
    }

    fn entry(&self, hash: &str) -> Option<PathBuf> {
        is_hash(hash).then(|| self.dir.join(hash))
    }

    /// The bytes with `hash`, marking them used; `None` when not held (or
    /// its file is not the size the push says: a truncated entry).
    pub fn get(&self, hash: &str, size: u64) -> Option<Vec<u8>> {
        let path = self.entry(hash)?;
        let bytes = std::fs::read(&path).ok()?;
        if bytes.len() as u64 != size {
            let _ = std::fs::remove_file(&path);
            return None;
        }
        // Best effort: an entry not touched is only evicted sooner.
        if let Ok(file) = std::fs::File::options().write(true).open(&path) {
            let _ = file.set_modified(SystemTime::now());
        }
        Some(bytes)
    }

    /// Keep `bytes` under `hash`. Refused, with nothing written, unless they
    /// hash to it.
    pub fn put(&self, hash: &str, bytes: &[u8]) -> Result<(), String> {
        let path = self.entry(hash).ok_or_else(|| format!("not a sha256: {hash:?}"))?;
        let actual = sha256_hex(bytes);
        if actual != hash {
            return Err(format!("the file hashes to {actual}, not the {hash} its push named"));
        }
        let write = || -> std::io::Result<()> {
            std::fs::create_dir_all(&self.dir)?;
            let partial = self.dir.join(format!("{hash}.partial"));
            std::fs::write(&partial, bytes)?;
            std::fs::rename(&partial, &path)
        };
        write().map_err(|e| format!("could not cache the file: {e}"))?;
        self.evict(hash);
        Ok(())
    }

    /// Entries as `(path, bytes, last used)`; partial writes are dropped.
    fn entries(&self) -> Vec<(PathBuf, u64, SystemTime)> {
        let Ok(dir) = std::fs::read_dir(&self.dir) else { return Vec::new() };
        dir.flatten()
            .filter_map(|entry| {
                let path = entry.path();
                if !entry.file_name().to_str().is_some_and(is_hash) {
                    let _ = std::fs::remove_file(&path);
                    return None;
                }
                let meta = entry.metadata().ok()?;
                Some((path, meta.len(), meta.modified().ok()?))
            })
            .collect()
    }

    /// Bytes held.
    pub fn size(&self) -> u64 {
        self.entries().iter().map(|(_, len, _)| len).sum()
    }

    /// Drop the least recently used entries until the cache fits its cap,
    /// never `keep` (the one just written).
    fn evict(&self, keep: &str) {
        let mut entries = self.entries();
        let mut held: u64 = entries.iter().map(|(_, len, _)| len).sum();
        entries.sort_by_key(|(_, _, used)| *used);
        for (path, len, _) in entries {
            if held <= self.cap {
                break;
            }
            if path.file_name().and_then(|name| name.to_str()) == Some(keep) {
                continue;
            }
            if std::fs::remove_file(&path).is_ok() {
                held -= len;
            }
        }
    }

    /// Forget everything held; the bytes freed.
    pub fn clear(&self) -> u64 {
        let freed = self.size();
        let _ = std::fs::remove_dir_all(&self.dir);
        self.expected.lock().unwrap_or_else(|e| e.into_inner()).clear();
        freed
    }

    /// Begin a push of `manifest`: the files held, as `(path, bytes)` to
    /// install, and the hashes to ask for. Its uploads are then checked
    /// ([`Self::accept`]).
    pub fn begin(&self, manifest: &Manifest) -> (Vec<(String, Vec<u8>)>, ManifestAnswer) {
        let mut held = Vec::new();
        let mut missing = Vec::new();
        let mut asked = HashSet::new();
        let mut expected = HashMap::new();
        for file in &manifest.files {
            expected.insert(file.path.clone(), file.hash.clone());
            match self.get(&file.hash, file.size) {
                Some(bytes) => held.push((file.path.clone(), bytes)),
                None => {
                    if asked.insert(file.hash.clone()) {
                        missing.push(file.hash.clone());
                    }
                }
            }
        }
        *self.expected.lock().unwrap_or_else(|e| e.into_inner()) = expected;
        (held, ManifestAnswer { missing })
    }

    /// An upload of `path` arrived: keep it when the push in flight named
    /// it. `Err` when its bytes are not the ones named.
    pub fn accept(&self, path: &str, bytes: &[u8]) -> Result<(), String> {
        let hash = self.expected.lock().unwrap_or_else(|e| e.into_inner()).get(path).cloned();
        match hash {
            Some(hash) => self.put(&hash, bytes).map_err(|e| format!("{path}: {e}")),
            // Unnamed (a sender without manifests): not cached.
            None => Ok(()),
        }
    }

    /// [`Self::accept`], off the caller's thread: hashing and writing a
    /// large file takes seconds on a device, which the upload need not wait
    /// for. A file that is not the one named is logged and not kept.
    pub fn accept_later(self: &Arc<Self>, path: String, bytes: Vec<u8>) {
        let hash = self.expected.lock().unwrap_or_else(|e| e.into_inner()).get(&path).cloned();
        let Some(hash) = hash else { return };
        let cache = self.clone();
        self.later(Box::new(move || {
            if let Err(error) = cache.put(&hash, &bytes) {
                log::warn!("push cache: {path}: {error}");
            }
        }));
    }

    /// Wait for every [`Self::accept_later`] so far.
    pub fn settle(&self) {
        let (done, settled) = std::sync::mpsc::channel();
        self.later(Box::new(move || {
            let _ = done.send(());
        }));
        let _ = settled.recv();
    }

    /// Run `work` on the cache's writer thread (started on first use), in order.
    fn later(&self, work: Box<dyn FnOnce() + Send>) {
        let mut writer = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        if writer.is_none() {
            let (queue, work) = std::sync::mpsc::channel::<Box<dyn FnOnce() + Send>>();
            let spawned = std::thread::Builder::new().name("functor-push-cache".to_string()).spawn(move || {
                crate::jobs::lower_priority();
                for work in work {
                    work();
                }
            });
            *writer = spawned.is_ok().then_some(queue);
        }
        // Without a writer the file is not cached: it is sent again next time.
        if let Some(queue) = writer.as_ref() {
            let _ = queue.send(work);
        }
    }

    /// The push ended (`/sync-assets`).
    pub fn end(&self) {
        self.expected.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
}

static INSTALLED: RwLock<Option<Arc<PushCache>>> = RwLock::new(None);

/// Keep pushed files under `dir` (a device shell with app storage).
pub fn install(dir: &Path, cap: u64) {
    *INSTALLED.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(PushCache::new(dir.to_path_buf(), cap)));
}

/// This process's cache, when its shell installed one.
pub fn installed() -> Option<Arc<PushCache>> {
    INSTALLED.read().unwrap_or_else(|e| e.into_inner()).clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, bytes: &[u8]) -> ManifestFile {
        ManifestFile { path: path.into(), size: bytes.len() as u64, hash: sha256_hex(bytes) }
    }

    #[test]
    fn a_push_asks_only_for_what_the_cache_lacks() {
        let dir = tempfile::tempdir().unwrap();
        let cache = PushCache::new(dir.path().join("push"), 1 << 20);
        let (fish, game, edited) = (b"fish".as_slice(), b"let init = 0".as_slice(), b"let init = 1".as_slice());
        let manifest = Manifest { files: vec![file("fish.glb", fish), file("copy/fish.glb", fish), file("game.fun", game)] };
        // Nothing held: both hashes asked for, the shared one once.
        let (held, answer) = cache.begin(&manifest);
        assert!(held.is_empty());
        assert_eq!(answer.missing, [sha256_hex(fish), sha256_hex(game)]);
        cache.accept("fish.glb", fish).unwrap();
        cache.accept("game.fun", game).unwrap();
        // An upload the push did not name is not kept.
        cache.accept("stray.bin", b"stray").unwrap();
        assert_eq!(cache.size(), (fish.len() + game.len()) as u64);
        cache.end();

        // The same project again: everything installs from the cache.
        let (held, answer) = cache.begin(&manifest);
        assert!(answer.missing.is_empty());
        let paths: Vec<&str> = held.iter().map(|(path, _)| path.as_str()).collect();
        assert_eq!(paths, ["fish.glb", "copy/fish.glb", "game.fun"]);
        assert_eq!(held[1].1, fish);

        // A one-line edit, and another project with the same model: one file to send.
        let other = Manifest { files: vec![file("models/shark.glb", fish), file("game.fun", edited)] };
        let (held, answer) = cache.begin(&other);
        assert_eq!(held.len(), 1);
        assert_eq!(answer.missing, [sha256_hex(edited)]);
    }

    #[test]
    fn a_file_is_kept_only_when_it_hashes_to_what_the_push_named() {
        let dir = tempfile::tempdir().unwrap();
        let cache = PushCache::new(dir.path().to_path_buf(), 1 << 20);
        let whole = b"the whole file".as_slice();
        let manifest = Manifest { files: vec![file("a.bin", whole)] };
        cache.begin(&manifest);
        // Cut short, or other bytes: refused, and nothing is left behind.
        let error = cache.accept("a.bin", &whole[..5]).unwrap_err();
        assert!(error.contains("a.bin") && error.contains("hashes to"), "{error}");
        assert_eq!(cache.size(), 0);
        assert_eq!(cache.begin(&manifest).1.missing.len(), 1, "still asked for: the push resumes");
        // A write that never finished is not an entry.
        std::fs::write(dir.path().join(format!("{}.partial", sha256_hex(whole))), b"the wh").unwrap();
        assert_eq!(cache.begin(&manifest).1.missing.len(), 1);
        cache.accept("a.bin", whole).unwrap();
        assert!(cache.begin(&manifest).1.missing.is_empty());
        // An entry truncated behind the cache's back is dropped, not served.
        std::fs::write(dir.path().join(sha256_hex(whole)), b"the wh").unwrap();
        let (held, answer) = cache.begin(&manifest);
        assert!(held.is_empty() && answer.missing.len() == 1);
        assert!(cache.put("../escape", b"x").is_err());
    }

    #[test]
    fn an_upload_is_kept_off_the_callers_thread() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Arc::new(PushCache::new(dir.path().to_path_buf(), 1 << 20));
        let manifest = Manifest { files: vec![file("a.bin", b"aaaa"), file("b.bin", b"bbbb")] };
        cache.begin(&manifest);
        cache.accept_later("a.bin".into(), b"aaaa".to_vec());
        // Not the bytes named: logged, not kept.
        cache.accept_later("b.bin".into(), b"other".to_vec());
        cache.accept_later("unnamed.bin".into(), b"cccc".to_vec());
        cache.settle();
        assert_eq!(cache.begin(&manifest).1.missing, [sha256_hex(b"bbbb")]);
        assert_eq!(cache.size(), 4);
    }

    #[test]
    fn the_least_recently_used_go_first_and_clear_empties_it() {
        let dir = tempfile::tempdir().unwrap();
        let cache = PushCache::new(dir.path().to_path_buf(), 25);
        let blobs: Vec<Vec<u8>> = (0..3u8).map(|n| vec![n; 10]).collect();
        let hashes: Vec<String> = blobs.iter().map(|blob| sha256_hex(blob)).collect();
        let age = |hash: &str, secs: u64| {
            let file = std::fs::File::options().write(true).open(dir.path().join(hash)).unwrap();
            file.set_modified(SystemTime::now() - std::time::Duration::from_secs(secs)).unwrap();
        };
        cache.put(&hashes[0], &blobs[0]).unwrap();
        cache.put(&hashes[1], &blobs[1]).unwrap();
        age(&hashes[0], 100);
        age(&hashes[1], 200);
        // Using the older one makes it the newer.
        assert!(cache.get(&hashes[1], 10).is_some());
        cache.put(&hashes[2], &blobs[2]).unwrap();
        assert!(cache.get(&hashes[0], 10).is_none(), "evicted: least recently used");
        assert!(cache.get(&hashes[1], 10).is_some() && cache.get(&hashes[2], 10).is_some());
        assert_eq!(cache.size(), 20);
        // One file over the whole cap is still kept (it was just sent).
        let big = vec![9u8; 40];
        cache.put(&sha256_hex(&big), &big).unwrap();
        assert_eq!(cache.size(), 40);
        assert_eq!(cache.clear(), 40);
        assert_eq!(cache.size(), 0);
    }
}
