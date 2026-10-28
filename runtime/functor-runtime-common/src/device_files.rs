//! The headset's project files under `functor run vr`: what the host pushed,
//! plus files fetched from the host on demand (`external` prefixes, or
//! anything not pushed). A read the device can't satisfy is queued, never
//! waited on: the reader fails like a missing file, the host polls
//! `GET /project-files`, answers each request (`POST /project-files`), then
//! reloads the game once. Fetched files persist in app storage under the
//! host's version (size + mtime, or a zip entry's crc + size), so the next
//! session confirms them instead of resending.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::asset::AssetCache;
use crate::debug_protocol::{
    ProjectFileReply, ProjectFileRequest, ProjectFileRequests, ProjectFileStatus, ProjectSession,
};
use crate::storage::{ProjectReadError, ProjectReader};

/// Requests queued at once; more wait for a later load.
const MAX_PENDING: usize = 256;
/// A host that polled this recently is taken to be fetching.
const HOST_FRESH: Duration = Duration::from_secs(5);
/// What the persisted store holds by default before evicting the least
/// recently used files.
pub const DEFAULT_STORE_BYTES: u64 = 1 << 30;

pub struct DeviceFiles {
    pushed: Arc<AssetCache>,
    /// Where fetched files persist (`None`: memory only).
    store: Option<PathBuf>,
    cap: u64,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    session: Option<ProjectSession>,
    polled: Option<Instant>,
    /// Path → the version the device holds, if any, to confirm.
    pending: BTreeMap<String, Option<String>>,
    /// Fetched or confirmed this session, and read since.
    ready: HashMap<String, Arc<[u8]>>,
    /// Confirmed this session; read from the store on first use.
    confirmed: HashSet<String>,
    /// The host doesn't have it either: its reason.
    missing: HashMap<String, String>,
    index: Index,
}

/// `index.json` in the store.
#[derive(Default, Serialize, Deserialize)]
struct Index {
    next: u64,
    clock: u64,
    files: BTreeMap<String, Stored>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Stored {
    version: String,
    len: u64,
    /// The blob's name in the store.
    file: String,
    /// `Index::clock` at last use, for eviction.
    used: u64,
}

impl DeviceFiles {
    /// `store` is created if missing; an unreadable index starts empty.
    pub fn new(pushed: Arc<AssetCache>, store: Option<PathBuf>, cap: u64) -> DeviceFiles {
        let index = store
            .as_ref()
            .and_then(|dir| std::fs::read(dir.join("index.json")).ok())
            .and_then(|json| serde_json::from_slice(&json).ok())
            .unwrap_or_default();
        DeviceFiles {
            pushed,
            store,
            cap,
            state: Mutex::new(State {
                index,
                ..State::default()
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A load from a host session (`run vr`). A new session forgets what the
    /// last one fetched, confirmed or ruled missing; the store stays.
    pub fn begin(&self, session: ProjectSession) {
        let mut state = self.lock();
        if state.session.as_ref().map(|s| s.id) != Some(session.id) {
            state.pending.clear();
            state.ready.clear();
            state.confirmed.clear();
            state.missing.clear();
        }
        state.session = Some(session);
    }

    /// `GET /project-files`; also marks the host as connected.
    pub fn requests(&self) -> ProjectFileRequests {
        let mut state = self.lock();
        state.polled = Some(Instant::now());
        ProjectFileRequests {
            session: state.session.as_ref().map_or(0, |s| s.id),
            requests: state
                .pending
                .iter()
                .map(|(path, have)| ProjectFileRequest {
                    path: path.clone(),
                    have: have.clone(),
                })
                .collect(),
        }
    }

    /// `POST /project-files`. Returns a status line and, when new bytes are
    /// ready, the project-root locator whose decoded asset to evict.
    pub fn accept(&self, reply: ProjectFileReply) -> Result<(String, Option<String>), String> {
        let mut state = self.lock();
        let Some(session) = state.session.clone().filter(|s| s.id == reply.session) else {
            return Err(format!("stale reply for {} (another session)", reply.path));
        };
        let Some(have) = state.pending.remove(&reply.path) else {
            return Err(format!("{} wasn't requested", reply.path));
        };
        let path = reply.path;
        let locator = format!("{}{path}", session.entry_dir);
        match reply.status {
            ProjectFileStatus::Bytes { version, bytes } => {
                let len = bytes.len();
                let bytes: Arc<[u8]> = bytes.into();
                self.persist(&mut state, &path, &version, &bytes);
                state.ready.insert(path.clone(), bytes);
                log::info!("proxied {path} ({len} bytes) from the host");
                Ok((format!("stored {path}"), Some(locator)))
            }
            ProjectFileStatus::Current { version } => {
                if have.as_ref() != Some(&version)
                    || state.index.files.get(&path).map(|s| &s.version) != Some(&version)
                {
                    // The stored copy changed or went away: ask for the bytes.
                    state.pending.insert(path.clone(), None);
                    return Ok((format!("refetching {path}"), None));
                }
                state.index.clock += 1;
                let clock = state.index.clock;
                if let Some(stored) = state.index.files.get_mut(&path) {
                    stored.used = clock;
                }
                self.save_index(&state);
                state.confirmed.insert(path.clone());
                log::info!("{path} on the headset is current");
                Ok((format!("confirmed {path}"), Some(locator)))
            }
            ProjectFileStatus::Missing { message } => {
                log::info!("the host has no {path}: {message}");
                state.missing.insert(path.clone(), message);
                Ok((format!("missing {path}"), None))
            }
        }
    }

    /// Keep `bytes` in the store, evicting least recently used files to fit.
    /// A file bigger than the whole store stays in memory only.
    fn persist(&self, state: &mut State, path: &str, version: &str, bytes: &[u8]) {
        let Some(dir) = &self.store else { return };
        let len = bytes.len() as u64;
        if let Some(old) = state.index.files.remove(path) {
            let _ = std::fs::remove_file(dir.join(old.file));
        }
        if len > self.cap {
            self.save_index(state);
            return;
        }
        let mut total: u64 = state.index.files.values().map(|s| s.len).sum();
        while total + len > self.cap {
            let Some((oldest, _)) = state
                .index
                .files
                .iter()
                .min_by_key(|(_, stored)| stored.used)
                .map(|(path, stored)| (path.clone(), stored.used))
            else {
                break;
            };
            let stored = state.index.files.remove(&oldest).expect("just found");
            total -= stored.len;
            let _ = std::fs::remove_file(dir.join(stored.file));
            state.confirmed.remove(&oldest);
        }
        state.index.next += 1;
        state.index.clock += 1;
        let file = format!("{}.bin", state.index.next);
        let tmp = dir.join(format!("{file}.tmp"));
        let written = std::fs::create_dir_all(dir)
            .and_then(|_| std::fs::write(&tmp, bytes))
            .and_then(|_| std::fs::rename(&tmp, dir.join(&file)));
        if let Err(e) = written {
            log::warn!("couldn't keep {path} on the headset ({e}); it's in memory only");
            return;
        }
        let used = state.index.clock;
        state.index.files.insert(
            path.to_string(),
            Stored {
                version: version.to_string(),
                len,
                file,
                used,
            },
        );
        self.save_index(state);
    }

    fn save_index(&self, state: &State) {
        let Some(dir) = &self.store else { return };
        let tmp = dir.join("index.json.tmp");
        let json = serde_json::to_vec(&state.index).expect("the index serializes");
        let saved = std::fs::create_dir_all(dir)
            .and_then(|_| std::fs::write(&tmp, json))
            .and_then(|_| std::fs::rename(&tmp, dir.join("index.json")));
        if let Err(e) = saved {
            log::warn!("couldn't save the headset's file index ({e})");
        }
    }

    /// Queue `path` for the host (unless already queued, ruled missing, or
    /// over the cap) and say why it isn't here yet.
    fn miss(&self, state: &mut State, path: &str) -> String {
        if state.session.is_none() {
            return "not pushed to this device; paths are case-sensitive, and hidden or ignored \
files aren't pushed"
                .to_string();
        }
        if !state.pending.contains_key(path) && state.pending.len() < MAX_PENDING {
            let have = state.index.files.get(path).map(|s| s.version.clone());
            state.pending.insert(path.to_string(), have);
        }
        if state.polled.is_some_and(|at| at.elapsed() < HOST_FRESH) {
            "not on the headset yet — `functor run vr` is fetching it from your computer and \
reloads the game when it arrives"
                .to_string()
        } else {
            "not on the headset, and no `functor run vr` is connected to fetch it — restart it"
                .to_string()
        }
    }
}

impl ProjectReader for DeviceFiles {
    fn read(&self, path: &str) -> Result<Arc<[u8]>, ProjectReadError> {
        let mut state = self.lock();
        let entry_dir = state.session.as_ref().map_or("", |s| &s.entry_dir);
        let pushed = format!("{entry_dir}{path}");
        if self.pushed.exists(&pushed) {
            drop(state);
            return self.pushed.read(&pushed);
        }
        if let Some(bytes) = state.ready.get(path) {
            return Ok(bytes.clone());
        }
        if let Some(message) = state.missing.get(path) {
            return Err(ProjectReadError::Unreadable(message.clone()));
        }
        // A confirmed stored copy, read outside the lock.
        let stored = state
            .confirmed
            .contains(path)
            .then(|| state.index.files.get(path).map(|s| s.file.clone()))
            .flatten();
        if let (Some(file), Some(dir)) = (stored, &self.store) {
            drop(state);
            let read = std::fs::read(dir.join(file));
            let mut state = self.lock();
            match read {
                Ok(bytes) => {
                    let bytes: Arc<[u8]> = bytes.into();
                    state.ready.insert(path.to_string(), bytes.clone());
                    return Ok(bytes);
                }
                Err(_) => {
                    // Gone from the store: fetch it again.
                    state.confirmed.remove(path);
                    state.index.files.remove(path);
                    return Err(ProjectReadError::Unreadable(self.miss(&mut state, path)));
                }
            }
        }
        Err(ProjectReadError::Unreadable(self.miss(&mut state, path)))
    }

    fn exists(&self, path: &str) -> bool {
        let mut state = self.lock();
        let entry_dir = state.session.as_ref().map_or("", |s| &s.entry_dir);
        if self.pushed.exists(&format!("{entry_dir}{path}"))
            || state.ready.contains_key(path)
            || state.confirmed.contains(path)
        {
            return true;
        }
        if !state.missing.contains_key(path) {
            self.miss(&mut state, path);
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(id: u64, entry_dir: &str) -> ProjectSession {
        ProjectSession {
            id,
            entry_dir: entry_dir.to_string(),
        }
    }

    fn reply(session: u64, path: &str, status: ProjectFileStatus) -> ProjectFileReply {
        ProjectFileReply {
            session,
            path: path.to_string(),
            status,
        }
    }

    fn bytes(version: &str, data: &[u8]) -> ProjectFileStatus {
        ProjectFileStatus::Bytes {
            version: version.to_string(),
            bytes: data.to_vec(),
        }
    }

    fn read(files: &DeviceFiles, path: &str) -> Result<Vec<u8>, String> {
        match files.read(path) {
            Ok(bytes) => Ok(bytes.to_vec()),
            Err(ProjectReadError::Unreadable(e)) => Err(e),
            Err(ProjectReadError::Outside) => Err("outside".to_string()),
        }
    }

    fn paths(files: &DeviceFiles) -> Vec<(String, Option<String>)> {
        files
            .requests()
            .requests
            .into_iter()
            .map(|r| (r.path, r.have))
            .collect()
    }

    /// Without a host session it's the pushed files only, as before.
    #[test]
    fn without_a_session_only_pushed_files_read() {
        let pushed = Arc::new(AssetCache::new());
        pushed.replace_uploaded("data/a.bin", vec![1]);
        let files = DeviceFiles::new(pushed, None, DEFAULT_STORE_BYTES);
        assert_eq!(read(&files, "data/a.bin"), Ok(vec![1]));
        assert!(read(&files, "ss2/x.bin")
            .unwrap_err()
            .starts_with("not pushed"));
        assert!(!files.exists("ss2/x.bin"));
        assert_eq!(files.requests().requests, vec![]);
    }

    /// Paths start at the entry's folder; a miss is queued (never waited
    /// on) and the host's bytes then read, persisted, with the locator to
    /// evict.
    #[test]
    fn misses_are_queued_and_fetched_bytes_read() {
        let store = tempfile::tempdir().unwrap();
        let pushed = Arc::new(AssetCache::new());
        pushed.replace_uploaded("src/data.bin", vec![1]);
        let files = DeviceFiles::new(pushed, Some(store.path().into()), DEFAULT_STORE_BYTES);
        files.begin(session(7, "src/"));
        assert_eq!(read(&files, "data.bin"), Ok(vec![1]));

        let err = read(&files, "ss2/a.bin").unwrap_err();
        assert!(err.contains("no `functor run vr` is connected"), "{err}");
        assert!(!files.exists("ss2/b.bin"));
        assert_eq!(
            paths(&files),
            [
                ("ss2/a.bin".to_string(), None),
                ("ss2/b.bin".to_string(), None)
            ]
        );
        // Polled: the host is fetching.
        let err = read(&files, "ss2/a.bin").unwrap_err();
        assert!(err.contains("is fetching it"), "{err}");

        let accepted = files.accept(reply(7, "ss2/a.bin", bytes("v1", b"abc")));
        assert_eq!(
            accepted,
            Ok((
                "stored ss2/a.bin".to_string(),
                Some("src/ss2/a.bin".to_string())
            ))
        );
        assert_eq!(read(&files, "ss2/a.bin"), Ok(b"abc".to_vec()));
        assert!(files.exists("ss2/a.bin"));
        assert_eq!(paths(&files), [("ss2/b.bin".to_string(), None)]);
        assert!(store.path().join("index.json").is_file());
    }

    /// Replies for another session, or for paths nobody asked for, are
    /// refused; "missing" sticks for the session without re-asking.
    #[test]
    fn stale_unrequested_and_missing_replies() {
        let files = DeviceFiles::new(Arc::new(AssetCache::new()), None, DEFAULT_STORE_BYTES);
        files.begin(session(1, ""));
        assert!(read(&files, "ss2/a.bin").is_err());
        let stale = files.accept(reply(2, "ss2/a.bin", bytes("v", b"x")));
        assert!(stale.unwrap_err().contains("another session"));
        let unasked = files.accept(reply(1, "ss2/other.bin", bytes("v", b"x")));
        assert!(unasked.unwrap_err().contains("wasn't requested"));

        let missing = ProjectFileStatus::Missing {
            message: "ss2/ isn't mapped".to_string(),
        };
        assert!(files.accept(reply(1, "ss2/a.bin", missing)).is_ok());
        assert_eq!(
            read(&files, "ss2/a.bin"),
            Err("ss2/ isn't mapped".to_string())
        );
        assert!(!files.exists("ss2/a.bin"));
        assert_eq!(paths(&files), []);

        // A new session asks again.
        files.begin(session(2, ""));
        assert!(read(&files, "ss2/a.bin").is_err());
        assert_eq!(paths(&files), [("ss2/a.bin".to_string(), None)]);
    }

    /// After an app restart the store survives: the next session offers the
    /// stored version and the host confirms it instead of resending; a
    /// changed version is fetched again.
    #[test]
    fn stored_files_are_confirmed_across_restarts() {
        let store = tempfile::tempdir().unwrap();
        let open = || {
            DeviceFiles::new(
                Arc::new(AssetCache::new()),
                Some(store.path().into()),
                DEFAULT_STORE_BYTES,
            )
        };
        let files = open();
        files.begin(session(1, ""));
        let _ = read(&files, "ss2/a.bin");
        let _ = read(&files, "ss2/b.bin");
        files
            .accept(reply(1, "ss2/a.bin", bytes("v1", b"abc")))
            .unwrap();
        files
            .accept(reply(1, "ss2/b.bin", bytes("v1", b"old")))
            .unwrap();
        drop(files);

        let files = open();
        files.begin(session(2, ""));
        assert!(
            !files.exists("ss2/a.bin"),
            "unconfirmed until the host says so"
        );
        let _ = read(&files, "ss2/b.bin");
        assert_eq!(
            paths(&files),
            [
                ("ss2/a.bin".to_string(), Some("v1".to_string())),
                ("ss2/b.bin".to_string(), Some("v1".to_string()))
            ]
        );
        let current = ProjectFileStatus::Current {
            version: "v1".to_string(),
        };
        assert_eq!(
            files.accept(reply(2, "ss2/a.bin", current)).map(|(s, _)| s),
            Ok("confirmed ss2/a.bin".to_string())
        );
        assert_eq!(read(&files, "ss2/a.bin"), Ok(b"abc".to_vec()));
        // b changed on the host: new bytes replace the stored copy.
        files
            .accept(reply(2, "ss2/b.bin", bytes("v2", b"new")))
            .unwrap();
        assert_eq!(read(&files, "ss2/b.bin"), Ok(b"new".to_vec()));
        // A "current" for a version the device doesn't hold refetches.
        files.begin(session(3, ""));
        let _ = read(&files, "ss2/a.bin");
        let stale = ProjectFileStatus::Current {
            version: "v0".to_string(),
        };
        files.accept(reply(3, "ss2/a.bin", stale)).unwrap();
        assert_eq!(paths(&files), [("ss2/a.bin".to_string(), None)]);
    }

    /// Over the cap, the least recently used files go first.
    #[test]
    fn the_store_evicts_least_recently_used_files() {
        let store = tempfile::tempdir().unwrap();
        let files = DeviceFiles::new(Arc::new(AssetCache::new()), Some(store.path().into()), 5);
        files.begin(session(1, ""));
        for path in ["a.bin", "b.bin", "c.bin"] {
            let _ = read(&files, path);
        }
        files.accept(reply(1, "a.bin", bytes("v", b"aa"))).unwrap();
        files.accept(reply(1, "b.bin", bytes("v", b"bb"))).unwrap();
        files.accept(reply(1, "c.bin", bytes("v", b"cc"))).unwrap();
        let state = files.lock();
        let stored: Vec<&String> = state.index.files.keys().collect();
        assert_eq!(stored, ["b.bin", "c.bin"]);
        let blobs = std::fs::read_dir(store.path()).unwrap().count();
        assert_eq!(blobs, 3, "two blobs and the index");
        // Still readable this session from memory.
        drop(state);
        assert_eq!(read(&files, "a.bin"), Ok(b"aa".to_vec()));
    }
}
