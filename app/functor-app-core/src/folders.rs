//! Open folders: which folder the app opens at startup and the recent
//! projects list. Opening one is `update`'s (`ClientMsg::OpenFolder`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Recent projects kept, most recent first.
pub const MAX_RECENTS: usize = 10;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Recent {
    /// The project directory's name.
    pub name: String,
    /// Absolute and canonical: one entry per directory however it was opened.
    pub path: String,
    /// Its directory could not be opened when last checked.
    #[serde(default)]
    pub missing: bool,
}

/// Puts `path` first in `recents`, once, keeping [`MAX_RECENTS`]; whether
/// that changed anything.
pub fn remember(recents: &mut Vec<Recent>, path: &str) -> bool {
    let name = Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string());
    let entry = Recent {
        name,
        path: path.to_string(),
        missing: false,
    };
    if recents.first() == Some(&entry) {
        return false;
    }
    recents.retain(|r| r.path != path);
    recents.insert(0, entry);
    recents.truncate(MAX_RECENTS);
    true
}

/// Marks `path` missing, if it is a recent; whether it was not already.
pub fn mark_missing(recents: &mut [Recent], path: &str) -> bool {
    match recents.iter_mut().find(|r| r.path == path && !r.missing) {
        Some(r) => {
            r.missing = true;
            true
        }
        None => false,
    }
}

/// The folder to open at startup: `-d` (`dir`); else the current directory
/// when it is in or holds a project (`cwd`, checked by the shell); else the
/// most recent project not missing; else none.
pub fn startup(dir: Option<PathBuf>, cwd: Option<PathBuf>, recents: &[Recent]) -> Option<PathBuf> {
    dir.or(cwd).or_else(|| {
        let recent = recents.iter().find(|r| !r.missing)?;
        Some(PathBuf::from(&recent.path))
    })
}

/// Whether `start` (relative to the root, as [`crate::projects::relative`]
/// gives it) is inside one of the project `paths` or above one.
pub fn holds_project(paths: &[&str], start: &str) -> bool {
    paths.iter().any(|p| {
        let (p, s) = (Path::new(p), Path::new(start));
        *p == *"." || start == "." || s.starts_with(p) || p.starts_with(s)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(recents: &[Recent]) -> Vec<&str> {
        recents.iter().map(|r| r.path.as_str()).collect()
    }

    #[test]
    fn remembering_puts_a_project_first_once() {
        let mut recents = Vec::new();
        assert!(remember(&mut recents, "/a/hello"));
        assert!(remember(&mut recents, "/b/orbs"));
        assert_eq!(paths(&recents), ["/b/orbs", "/a/hello"]);
        assert_eq!(recents[0].name, "orbs");
        // Already first: no change.
        assert!(!remember(&mut recents, "/b/orbs"));
        // Again later: moved, not repeated.
        assert!(remember(&mut recents, "/a/hello"));
        assert_eq!(paths(&recents), ["/a/hello", "/b/orbs"]);
        // A missing entry opened again is found again.
        assert!(mark_missing(&mut recents, "/b/orbs"));
        assert!(!mark_missing(&mut recents, "/b/orbs"));
        assert!(!mark_missing(&mut recents, "/nowhere"));
        assert!(remember(&mut recents, "/b/orbs"));
        assert_eq!(paths(&recents), ["/b/orbs", "/a/hello"]);
        assert!(!recents[0].missing);
    }

    #[test]
    fn recents_are_capped_dropping_the_oldest() {
        let mut recents = Vec::new();
        for i in 0..MAX_RECENTS + 3 {
            remember(&mut recents, &format!("/p/{i}"));
        }
        assert_eq!(recents.len(), MAX_RECENTS);
        assert_eq!(recents[0].path, format!("/p/{}", MAX_RECENTS + 2));
        assert_eq!(recents.last().unwrap().path, "/p/3");
    }

    #[test]
    fn startup_prefers_the_flag_then_the_cwd_then_a_recent() {
        let recent = |path: &str, missing| Recent {
            name: "x".into(),
            path: path.into(),
            missing,
        };
        let recents = [recent("/gone", true), recent("/r", false)];
        let p = |s: &str| Some(PathBuf::from(s));
        assert_eq!(startup(p("/d"), p("/cwd"), &recents), p("/d"));
        assert_eq!(startup(None, p("/cwd"), &recents), p("/cwd"));
        assert_eq!(startup(None, None, &recents), p("/r"));
        assert_eq!(startup(None, None, &recents[..1]), None);
        assert_eq!(startup(None, None, &[]), None);
    }

    #[test]
    fn a_start_holds_a_project_it_is_in_or_above() {
        let projects = ["examples/hello", "games/a"];
        for start in ["examples/hello", "examples/hello/assets", "examples", "."] {
            assert!(holds_project(&projects, start), "{start}");
        }
        for start in ["docs", "examples/hello-world", "games/b"] {
            assert!(!holds_project(&projects, start), "{start}");
        }
        assert!(!holds_project(&[], "."));
        // The root as a project holds every start.
        assert!(holds_project(&["."], "docs"));
    }
}
