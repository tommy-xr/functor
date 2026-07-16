//! Asset hot-reload: stat every loaded asset file each frame — the same
//! mtime-polling scheme the Functor Lang project watcher uses for `.fun` files — and
//! report the ones that changed on disk so the run loop can evict them from
//! the caches. The next draw then re-reads and re-decodes the file: save a
//! `.glb`/`.png` in your editor and the running scene updates in ~1 frame.

use std::{collections::HashMap, time::SystemTime};

pub struct AssetWatcher {
    stamps: HashMap<String, SystemTime>,
    /// Changed-but-not-yet-stable mtimes. A save is not atomic — reloading on
    /// the FIRST new mtime reads a half-written file (a torn PNG/glb, which
    /// must not reach the parsers) — so a change is only reported once the
    /// mtime holds still across two consecutive polls (i.e. the writer went
    /// quiet for at least a frame; a long export keeps bumping it and waits).
    pending: HashMap<String, SystemTime>,
}

impl AssetWatcher {
    pub fn new() -> AssetWatcher {
        AssetWatcher {
            stamps: HashMap::new(),
            pending: HashMap::new(),
        }
    }

    /// The subset of `loaded` whose file changed on disk since last seen and
    /// has settled (see `pending`). A path seen for the first time only
    /// records its stamp (loading it was the freshest read possible — nothing
    /// to reload). A path that can't be stat'd is left alone: deleted-mid-save
    /// files come back a moment later, and non-file paths (URLs) never match.
    pub fn changed(&mut self, loaded: impl IntoIterator<Item = String>) -> Vec<String> {
        let mut out = Vec::new();
        for path in loaded {
            let Ok(mtime) = std::fs::metadata(&path).and_then(|md| md.modified()) else {
                continue;
            };
            match self.stamps.get(&path) {
                Some(prev) if *prev != mtime => match self.pending.get(&path) {
                    // Same mtime two polls in a row: the writer is done.
                    Some(pending) if *pending == mtime => {
                        self.pending.remove(&path);
                        self.stamps.insert(path.clone(), mtime);
                        out.push(path);
                    }
                    // First sighting of this mtime (or still being written):
                    // hold until it stabilizes.
                    _ => {
                        self.pending.insert(path, mtime);
                    }
                },
                Some(_) => {
                    self.pending.remove(&path);
                }
                None => {
                    self.stamps.insert(path, mtime);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str, contents: &[u8]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "functor-asset-watch-{}-{}",
            std::process::id(),
            name
        ));
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn first_sighting_records_then_change_reports_once() {
        let file = temp_file("a.png", b"v1");
        let path = file.to_string_lossy().to_string();
        let mut watcher = AssetWatcher::new();

        // First sighting: stamp only, no reload.
        assert!(watcher.changed([path.clone()]).is_empty());
        // Unchanged: quiet.
        assert!(watcher.changed([path.clone()]).is_empty());

        // Bump the mtime explicitly (rewriting can land in the same clock tick).
        let bump = |secs: u64| {
            let f = std::fs::File::options().write(true).open(&file).unwrap();
            f.set_modified(SystemTime::now() + std::time::Duration::from_secs(secs))
                .unwrap();
        };
        bump(2);

        // Poll 1 sees the new mtime: held as pending (the writer may still be
        // mid-save). Poll 2 sees it stable: reported. Poll 3: quiet.
        assert!(watcher.changed([path.clone()]).is_empty());
        assert_eq!(watcher.changed([path.clone()]), vec![path.clone()]);
        assert!(watcher.changed([path.clone()]).is_empty());

        // A write that keeps changing (a slow export) stays held until it
        // settles.
        bump(4);
        assert!(watcher.changed([path.clone()]).is_empty()); // pending @4
        bump(6);
        assert!(watcher.changed([path.clone()]).is_empty()); // still moving: pending @6
        assert_eq!(watcher.changed([path.clone()]), vec![path.clone()]); // settled
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn unstattable_paths_are_ignored() {
        let mut watcher = AssetWatcher::new();
        let gone = "definitely/not/a/file.glb".to_string();
        let url = "https://example.test/remote.glb".to_string();
        assert!(watcher.changed([gone, url]).is_empty());
    }
}
