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
    /// Bytes held, once counted.
    held: Mutex<Option<u64>>,
}

impl PushCache {
    pub fn new(dir: PathBuf, cap: u64) -> PushCache {
        PushCache { dir, cap, expected: Mutex::new(HashMap::new()), writer: Mutex::new(None), held: Mutex::new(None) }
    }

    fn entry(&self, hash: &str) -> Option<PathBuf> {
        is_hash(hash).then(|| self.dir.join(hash))
    }

    /// The bytes with `hash`, marking them used; `None` when not held (or
    /// its file is not the size the push says: a truncated entry). Trusted
    /// by size: entries are written whole and checked then, not re-hashed.
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
        // Listing the cache costs a stat per entry: only once it may be over.
        let mut held = self.held.lock().unwrap_or_else(|e| e.into_inner());
        let now = match *held {
            Some(before) => before + bytes.len() as u64,
            None => self.size(),
        };
        *held = Some(if now > self.cap { self.evict(hash) } else { now });
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
    /// never `keep` (the one just written). The bytes left.
    fn evict(&self, keep: &str) -> u64 {
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
        held
    }

    /// Forget everything held, once the writes queued before it are done;
    /// the bytes freed.
    pub fn clear(self: &Arc<Self>) -> u64 {
        let (done, freed) = std::sync::mpsc::channel();
        let cache = self.clone();
        self.later(Box::new(move || {
            let bytes = cache.size();
            let _ = std::fs::remove_dir_all(&cache.dir);
            *cache.held.lock().unwrap_or_else(|e| e.into_inner()) = Some(0);
            let _ = done.send(bytes);
        }));
        self.expected.lock().unwrap_or_else(|e| e.into_inner()).clear();
        freed.recv().unwrap_or(0)
    }

    /// Whether an entry of `size` bytes is held under `hash`.
    fn holds(&self, hash: &str, size: u64) -> bool {
        let len = self.entry(hash).and_then(|path| std::fs::metadata(path).ok()).map(|meta| meta.len());
        len == Some(size)
    }

    /// Begin a push of `manifest` (one at a time per device): the files
    /// held, to install with [`Self::get`], and the hashes to ask for. Its
    /// uploads are then checked ([`Self::accept_later`]).
    pub fn begin(&self, manifest: &Manifest) -> (Vec<ManifestFile>, ManifestAnswer) {
        let mut held = Vec::new();
        let mut missing = Vec::new();
        let mut asked = HashSet::new();
        let mut expected = HashMap::new();
        for file in &manifest.files {
            expected.insert(file.path.clone(), file.hash.clone());
            if self.holds(&file.hash, file.size) {
                held.push(file.clone());
            } else if asked.insert(file.hash.clone()) {
                missing.push(file.hash.clone());
            }
        }
        *self.expected.lock().unwrap_or_else(|e| e.into_inner()) = expected;
        (held, ManifestAnswer { missing })
    }

    /// An upload of `path` arrived: keep it when the push in flight named
    /// it and it hashes to what was named (else it is logged). Off the
    /// caller's thread: hashing and writing a large file takes seconds on a
    /// device, which the upload need not wait for.
    pub fn accept_later(self: &Arc<Self>, path: &str, bytes: &[u8]) {
        let hash = self.expected.lock().unwrap_or_else(|e| e.into_inner()).get(path).cloned();
        // Unnamed (a sender without manifests): not cached.
        let Some(hash) = hash else { return };
        let (cache, path, bytes) = (self.clone(), path.to_string(), bytes.to_vec());
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

    fn cache(dir: &Path, cap: u64) -> Arc<PushCache> {
        Arc::new(PushCache::new(dir.to_path_buf(), cap))
    }

    /// What a push of `manifest` finds: the paths held, and the hashes asked for.
    fn push(cache: &PushCache, manifest: &Manifest) -> (Vec<String>, Vec<String>) {
        let (held, answer) = cache.begin(manifest);
        (held.into_iter().map(|file| file.path).collect(), answer.missing)
    }

    #[test]
    fn a_push_asks_only_for_what_the_cache_lacks() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache(&dir.path().join("push"), 1 << 20);
        let (fish, game, edited) = (b"fish".as_slice(), b"let init = 0".as_slice(), b"let init = 1".as_slice());
        let manifest = Manifest { files: vec![file("fish.glb", fish), file("copy/fish.glb", fish), file("game.fun", game)] };
        // Nothing held: both hashes asked for, the shared one once.
        assert_eq!(push(&cache, &manifest), (vec![], vec![sha256_hex(fish), sha256_hex(game)]));
        cache.accept_later("fish.glb", fish);
        cache.accept_later("game.fun", game);
        // An upload the push did not name is not kept.
        cache.accept_later("stray.bin", b"stray");
        cache.settle();
        assert_eq!(cache.size(), (fish.len() + game.len()) as u64);
        cache.end();

        // The same project again: everything installs from the cache.
        let (held, missing) = push(&cache, &manifest);
        assert!(missing.is_empty());
        assert_eq!(held, ["fish.glb", "copy/fish.glb", "game.fun"]);
        assert_eq!(cache.get(&sha256_hex(fish), 4).as_deref(), Some(fish));

        // A one-line edit, and another project with the same model: one file to send.
        let other = Manifest { files: vec![file("models/shark.glb", fish), file("game.fun", edited)] };
        assert_eq!(push(&cache, &other), (vec!["models/shark.glb".to_string()], vec![sha256_hex(edited)]));
    }

    #[test]
    fn a_file_is_kept_only_when_it_hashes_to_what_the_push_named() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache(dir.path(), 1 << 20);
        let whole = b"the whole file".as_slice();
        let manifest = Manifest { files: vec![file("a.bin", whole)] };
        let asked = vec![sha256_hex(whole)];
        cache.begin(&manifest);
        // Cut short, or other bytes: not kept, and nothing is left behind.
        cache.accept_later("a.bin", &whole[..5]);
        cache.settle();
        assert_eq!(cache.size(), 0);
        assert_eq!(push(&cache, &manifest).1, asked, "still asked for: the push resumes");
        assert!(cache.put(&asked[0], b"other bytes").unwrap_err().contains("hashes to"));
        // A write that never finished is not an entry.
        std::fs::write(dir.path().join(format!("{}.partial", asked[0])), b"the wh").unwrap();
        assert_eq!(push(&cache, &manifest).1, asked);
        cache.accept_later("a.bin", whole);
        cache.settle();
        assert!(push(&cache, &manifest).1.is_empty());
        // An entry truncated behind the cache's back is asked for again, not served.
        std::fs::write(dir.path().join(&asked[0]), b"the wh").unwrap();
        assert_eq!(push(&cache, &manifest), (vec![], asked.clone()));
        assert!(cache.get(&asked[0], whole.len() as u64).is_none());
        assert!(cache.put("../escape", b"x").is_err());
    }

    #[test]
    fn the_least_recently_used_go_first_and_clear_empties_it() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache(dir.path(), 25);
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
        // A clear waits for what was being written, then nothing is held.
        cache.begin(&Manifest { files: vec![file("late.bin", b"late")] });
        cache.accept_later("late.bin", b"late");
        assert_eq!(cache.clear(), 4, "the write landed (evicting the big one) before the clear");
        assert_eq!(cache.size(), 0);
        cache.put(&hashes[0], &blobs[0]).unwrap();
        assert_eq!(cache.size(), 10);
    }
}
