//! Data-file reload: poll the versions ([`crate::storage::ProjectReader::version`])
//! of the project files a program read, and report the ones that changed once
//! changes have been quiet for [`SETTLE`] — so a save, an archive repack or a
//! burst of edits reloads once, and a file still being written waits.
//!
//! Polling, not OS notifications, as for `.fun` files: it sees deletes,
//! rename-saves and zip entries (an archive's index is re-read when it changes,
//! and only entries whose crc or size moved count).

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

/// How long versions must hold still before a change is reported.
pub const SETTLE: Duration = Duration::from_millis(150);
/// How often versions are polled.
pub const POLL: Duration = Duration::from_millis(100);

#[derive(Default)]
pub struct DataWatch {
    /// Path → (version last reported, version last seen); `None`: missing.
    files: BTreeMap<String, (Option<String>, Option<String>)>,
    /// When a seen version last moved, while unreported.
    moved: Option<Instant>,
    polled: Option<Instant>,
    /// Reported again at the next settle (their reload failed).
    retry: BTreeSet<String>,
    /// The last report was only a retry.
    retried: bool,
}

impl DataWatch {
    pub fn new() -> DataWatch {
        DataWatch::default()
    }

    /// Watch exactly `paths`; new ones start at their current version (the
    /// program just read them).
    pub fn watch(
        &mut self,
        paths: impl IntoIterator<Item = String>,
        version: impl Fn(&str) -> Option<String>,
    ) {
        let mut files = BTreeMap::new();
        for path in paths {
            let entry = self.files.remove(&path).unwrap_or_else(|| {
                let now = version(&path);
                (now.clone(), now)
            });
            files.insert(path, entry);
        }
        self.files = files;
        self.retry.retain(|path| self.files.contains_key(path));
    }

    /// Watch `path` from `version` on (what was last sent of it).
    pub fn insert(&mut self, path: String, version: Option<String>) {
        self.files.insert(path, (version.clone(), version));
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Poll (at most every [`POLL`]); once changes have settled, the paths
    /// whose version differs from the last report, sorted. An entry that
    /// changed and changed back (an archive rewritten with it intact) isn't
    /// reported.
    pub fn poll(&mut self, now: Instant, version: impl Fn(&str) -> Option<String>) -> Vec<String> {
        if self.polled.is_some_and(|at| now.duration_since(at) < POLL) {
            return Vec::new();
        }
        self.polled = Some(now);
        for (path, (_, seen)) in self.files.iter_mut() {
            let current = version(path);
            if *seen != current {
                *seen = current;
                self.moved = Some(now);
            }
        }
        match self.moved {
            Some(at) if now.duration_since(at) >= SETTLE => self.moved = None,
            _ => return Vec::new(),
        }
        let mut changed = Vec::new();
        for (path, (reported, seen)) in self.files.iter_mut() {
            if reported != seen {
                *reported = seen.clone();
                changed.push(path.clone());
            }
        }
        self.retried = changed.is_empty() && !self.retry.is_empty();
        let mut out: BTreeSet<String> = std::mem::take(&mut self.retry);
        out.extend(changed);
        out.into_iter().collect()
    }

    /// The reload `paths` triggered failed (a file read mid-write, say):
    /// report them again after the next settle — once, not after a retry.
    pub fn failed(&mut self, paths: &[String], now: Instant) {
        if self.retried {
            return;
        }
        self.retry.extend(paths.iter().cloned());
        self.moved = Some(now);
    }
}

/// `a.bin`, or `a.bin (+2 more)`: the paths a data reload names.
pub fn describe(paths: &[String]) -> String {
    match paths {
        [] => String::new(),
        [one] => one.clone(),
        [first, rest @ ..] => format!("{first} (+{} more)", rest.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    /// Versions by path, editable between polls.
    struct Disk(RefCell<HashMap<String, String>>);

    impl Disk {
        fn new(files: &[(&str, &str)]) -> Disk {
            Disk(RefCell::new(
                files
                    .iter()
                    .map(|(p, v)| (p.to_string(), v.to_string()))
                    .collect(),
            ))
        }
        fn set(&self, path: &str, version: Option<&str>) {
            let mut files = self.0.borrow_mut();
            match version {
                Some(v) => files.insert(path.to_string(), v.to_string()),
                None => files.remove(path),
            };
        }
        fn version(&self) -> impl Fn(&str) -> Option<String> + '_ {
            |path| self.0.borrow().get(path).cloned()
        }
    }

    fn ms(start: Instant, ms: u64) -> Instant {
        start + Duration::from_millis(ms)
    }

    /// A change is reported once it has held still for SETTLE; a burst of
    /// changes is one report; unwatched files never are.
    #[test]
    fn changes_settle_then_report_once() {
        let disk = Disk::new(&[("a", "1"), ("b", "1"), ("other", "1")]);
        let mut watch = DataWatch::new();
        watch.watch(["a".to_string(), "b".to_string()], disk.version());
        let t = Instant::now();
        assert!(watch.poll(t, disk.version()).is_empty());

        disk.set("other", Some("2"));
        assert!(watch.poll(ms(t, 100), disk.version()).is_empty());
        assert!(watch.poll(ms(t, 400), disk.version()).is_empty());

        disk.set("a", Some("2"));
        assert!(watch.poll(ms(t, 500), disk.version()).is_empty(), "moving");
        disk.set("b", Some("2"));
        assert!(
            watch.poll(ms(t, 600), disk.version()).is_empty(),
            "still moving"
        );
        assert!(
            watch.poll(ms(t, 650), disk.version()).is_empty(),
            "throttled"
        );
        assert!(
            watch.poll(ms(t, 700), disk.version()).is_empty(),
            "settling"
        );
        assert_eq!(watch.poll(ms(t, 800), disk.version()), ["a", "b"]);
        assert!(watch.poll(ms(t, 1000), disk.version()).is_empty());
    }

    /// A delete reports (the read now fails), and so does the recreate; an
    /// entry that changes and changes back before settling doesn't.
    #[test]
    fn deletes_recreates_and_round_trips() {
        let disk = Disk::new(&[("a", "1")]);
        let mut watch = DataWatch::new();
        watch.watch(["a".to_string()], disk.version());
        let t = Instant::now();
        disk.set("a", None);
        watch.poll(t, disk.version());
        assert_eq!(watch.poll(ms(t, 200), disk.version()), ["a"]);
        disk.set("a", Some("2"));
        watch.poll(ms(t, 300), disk.version());
        assert_eq!(watch.poll(ms(t, 500), disk.version()), ["a"]);

        // A torn archive (unreadable) then the same entry again.
        disk.set("a", None);
        watch.poll(ms(t, 600), disk.version());
        disk.set("a", Some("2"));
        watch.poll(ms(t, 700), disk.version());
        assert!(watch.poll(ms(t, 900), disk.version()).is_empty());
    }

    /// Re-deriving the set keeps known versions (a pending change still
    /// reports), starts new paths at their current version, drops the rest.
    #[test]
    fn rewatching_keeps_known_paths() {
        let disk = Disk::new(&[("a", "1"), ("b", "1")]);
        let mut watch = DataWatch::new();
        watch.watch(["a".to_string()], disk.version());
        let t = Instant::now();
        disk.set("a", Some("2"));
        watch.poll(t, disk.version());
        watch.watch(["a".to_string(), "b".to_string()], disk.version());
        assert_eq!(watch.len(), 2);
        assert_eq!(watch.poll(ms(t, 200), disk.version()), ["a"]);
        watch.watch(["b".to_string()], disk.version());
        disk.set("a", Some("3"));
        watch.poll(ms(t, 300), disk.version());
        assert!(watch.poll(ms(t, 500), disk.version()).is_empty());
    }

    /// A failed reload retries once after the settle, not again.
    #[test]
    fn a_failed_reload_retries_once() {
        let disk = Disk::new(&[("a", "1")]);
        let mut watch = DataWatch::new();
        watch.watch(["a".to_string()], disk.version());
        let t = Instant::now();
        disk.set("a", Some("2"));
        watch.poll(t, disk.version());
        let changed = watch.poll(ms(t, 200), disk.version());
        assert_eq!(changed, ["a"]);
        watch.failed(&changed, ms(t, 200));
        assert!(
            watch.poll(ms(t, 300), disk.version()).is_empty(),
            "settling"
        );
        let retried = watch.poll(ms(t, 400), disk.version());
        assert_eq!(retried, ["a"]);
        watch.failed(&retried, ms(t, 400));
        assert!(watch.poll(ms(t, 800), disk.version()).is_empty(), "once");

        // A fresh change can retry again.
        disk.set("a", Some("3"));
        watch.poll(ms(t, 900), disk.version());
        let changed = watch.poll(ms(t, 1100), disk.version());
        watch.failed(&changed, ms(t, 1100));
        assert_eq!(watch.poll(ms(t, 1300), disk.version()), ["a"]);
    }

    #[test]
    fn describe_names_the_first_path() {
        assert_eq!(describe(&["a".into()]), "a");
        assert_eq!(
            describe(&["a".into(), "b".into(), "c".into()]),
            "a (+2 more)"
        );
    }
}
