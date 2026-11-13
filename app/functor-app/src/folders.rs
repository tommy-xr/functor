//! Folders the page names: checking one before it is opened, reading it as
//! a workspace, and directory suggestions for the path field. Only
//! directory names are ever listed, never files or their contents.

use std::path::{Path, PathBuf};

use functor_app_core::folders::holds_project;
use functor_app_core::projects::{discover, relative};
use functor_app_core::{update, AppState, Event};

use crate::changes::Git;

/// Most directories one suggestion lists.
const MAX_SUGGESTIONS: usize = 20;
/// Most entries of one directory read for suggestions.
const MAX_SCANNED: usize = 5000;

/// `path` with a leading `~` as the user's `home`.
pub(crate) fn expand(path: &str, home: Option<&Path>) -> Option<PathBuf> {
    match (path.strip_prefix('~'), home) {
        (Some(""), Some(home)) => Some(home.to_path_buf()),
        (Some(rest), Some(home)) if rest.starts_with(std::path::is_separator) => {
            Some(home.join(&rest[1..]))
        }
        (Some(_), _) => None,
        (None, _) => Some(PathBuf::from(path)),
    }
}

/// The folder `path` names, as the app may open it: `~` expanded, absolute,
/// resolved through symlinks and `..`, an existing directory it can list.
/// Taken as given (a name may end in a space); the page trims what is typed.
pub fn resolve(path: &str, home: Option<&Path>) -> Result<PathBuf, String> {
    if path.is_empty() {
        return Err("no folder given".to_string());
    }
    let expanded = expand(path, home).ok_or("only your own home can be named with ~")?;
    if !expanded.is_absolute() {
        return Err("give the folder's full path".to_string());
    }
    let dir = std::fs::canonicalize(&expanded).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => "it does not exist".to_string(),
        _ => e.to_string(),
    })?;
    if !dir.is_dir() {
        return Err("it is not a folder".to_string());
    }
    // Their walk for projects would read the whole disk or home.
    if dir.parent().is_none() {
        return Err("that is the whole disk: open a folder inside it".to_string());
    }
    if home.and_then(|h| std::fs::canonicalize(h).ok()).as_deref() == Some(&dir) {
        return Err("that is your whole home folder: open a folder inside it".to_string());
    }
    std::fs::read_dir(&dir).map_err(|e| format!("it cannot be read: {e}"))?;
    Ok(dir)
}

/// Up to [`MAX_SUGGESTIONS`] directories completing `path`, sorted: those
/// in its parent whose names start with its last part (ignoring case), or
/// all of a directory named with a trailing separator. Hidden ones are left
/// out. Spelled as typed, so a chosen one extends what is in the field.
pub fn suggest(path: &str, home: Option<&Path>) -> Vec<String> {
    // What is typed up to its last separator names the directory listed.
    let Some(at) = path.rfind(std::path::is_separator) else {
        return Vec::new();
    };
    let (typed, stem) = path.split_at(at + 1);
    let Some(parent) = expand(typed, home).filter(|p| p.is_absolute()) else {
        return Vec::new();
    };
    let stem = stem.to_lowercase();
    let Ok(entries) = std::fs::read_dir(&parent) else {
        return Vec::new();
    };
    let mut dirs: Vec<String> = entries
        .take(MAX_SCANNED)
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let wanted = !name.starts_with('.') && name.to_lowercase().starts_with(&stem);
            // `Path::is_dir` follows a symlink: a link to a folder is one.
            (wanted && e.path().is_dir()).then(|| format!("{typed}{name}"))
        })
        .collect();
    dirs.sort();
    dirs.truncate(MAX_SUGGESTIONS);
    dirs
}

/// The folder chosen in the window's native picker, a sheet starting at
/// `from`; `None` if cancelled. Built off the core loop's thread: rfd hands
/// the panel to the main thread (tao's event loop) and waits for it there.
#[cfg(target_os = "macos")]
pub async fn pick(from: Option<PathBuf>) -> Option<String> {
    let dialog = tokio::task::spawn_blocking(move || {
        let mut dialog = rfd::AsyncFileDialog::new().set_title("Open Folder");
        if let Some(from) = from {
            dialog = dialog.set_directory(from);
        }
        dialog.pick_folder()
    });
    // A panic (no window to attach the sheet to) is no choice.
    let picked = dialog.await.ok()?.await?;
    Some(picked.path().to_string_lossy().into_owned())
}

/// Only the macOS window has a picker (`AppState::native_picker`).
#[cfg(not(target_os = "macos"))]
pub async fn pick(_from: Option<PathBuf>) -> Option<String> {
    None
}

/// Whether the app started in `cwd` should open it: it is in or above a
/// project. The filesystem root (where the Finder starts apps) and the
/// home folder never are: their walks would read the whole disk or home.
pub fn cwd_holds_project(cwd: &Path) -> bool {
    if resolve(&cwd.to_string_lossy(), std::env::home_dir().as_deref()).is_err() {
        return false;
    }
    let state = discover(cwd);
    let paths: Vec<&str> = state.projects.iter().map(|p| p.path.as_str()).collect();
    holds_project(&paths, &relative(Path::new(&state.repo_root), cwd))
}

/// A workspace as read from disk, with its git when changes are tracked
/// and the highest task id in its store.
pub struct Loaded {
    pub state: AppState,
    pub git: Option<Git>,
    pub highest: u64,
}

/// Reads the folder `path` names as a workspace: its root, projects, the
/// active one and its files, and what the store at `home` keeps for them.
/// With `track`, its git, when it is a work tree (`--print` runs none).
/// Blocking.
pub fn load(path: &str, home: &Path, track: bool) -> Result<Loaded, String> {
    let dir = resolve(path, std::env::home_dir().as_deref())?;
    let mut state = discover(&dir);
    let (tasks, highest) = crate::store::load(home, &state);
    state = update(state, Event::TasksLoaded { tasks }).0;
    let layouts = crate::store::load_layouts(home, &state);
    state = update(state, Event::LayoutsLoaded { layouts }).0;
    state.harness_choice = crate::store::load_harnesses(home, &state);
    state.review_settings = crate::store::load_reviews(home, &state);
    let root = PathBuf::from(&state.repo_root);
    let git = track.then(|| Git::open(&root, home)).flatten();
    state.tracks_changes = git.is_some();
    state.git = track.then(|| crate::server::git_info(&root)).flatten();
    Ok(Loaded {
        state,
        git,
        highest,
    })
}

// Spelled with `/`, as on Unix; the window (and its picker) is macOS-only.
#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn tree(paths: &[&str]) -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        for path in paths {
            let path = root.join(path);
            match path.to_string_lossy().ends_with('/') {
                true => std::fs::create_dir_all(&path).unwrap(),
                false => {
                    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                    std::fs::write(&path, "secret").unwrap();
                }
            }
        }
        (tmp, root)
    }

    #[test]
    fn a_folder_resolves_absolute_and_canonical() {
        let (_tmp, root) = tree(&["a/b/", "file.txt"]);
        let s = |p: &Path| p.to_string_lossy().into_owned();
        assert_eq!(resolve(&s(&root.join("a/b")), None), Ok(root.join("a/b")));
        // Trailing separators and `..` name the same folder.
        for spelled in [
            format!("{}/", s(&root.join("a/b"))),
            s(&root.join("a/b/../b")),
        ] {
            assert_eq!(resolve(&spelled, None), Ok(root.join("a/b")), "{spelled}");
        }
        assert_eq!(resolve("~/a", Some(&root)), Ok(root.join("a")));
        let whole = |what: &str| {
            Err(format!(
                "that is your whole {what}: open a folder inside it"
            ))
        };
        assert_eq!(resolve("~", Some(&root)), whole("home folder"));
        assert_eq!(resolve(&s(&root), Some(&root)), whole("home folder"));
        assert_eq!(
            resolve("/", None),
            Err("that is the whole disk: open a folder inside it".into())
        );
        for (bad, why) in [
            ("", "no folder given"),
            ("a/b", "give the folder's full path"),
            ("~other/x", "only your own home can be named with ~"),
        ] {
            assert_eq!(resolve(bad, Some(&root)), Err(why.to_string()), "{bad}");
        }
        assert_eq!(
            resolve(&s(&root.join("nope")), None),
            Err("it does not exist".to_string())
        );
        assert_eq!(
            resolve(&s(&root.join("file.txt")), None),
            Err("it is not a folder".to_string())
        );
    }

    #[test]
    fn a_symlinked_folder_resolves_to_its_target() {
        let (_tmp, root) = tree(&["real/project/"]);
        std::os::unix::fs::symlink(root.join("real"), root.join("link")).unwrap();
        let via = root.join("link/project");
        assert_eq!(
            resolve(&via.to_string_lossy(), None),
            Ok(root.join("real/project"))
        );
    }

    #[test]
    fn an_unreadable_folder_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let (_tmp, root) = tree(&["locked/"]);
        let locked = root.join("locked");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        // Root reads it anyway; then there is nothing to refuse.
        let readable = std::fs::read_dir(&locked).is_ok();
        let result = resolve(&locked.to_string_lossy(), None);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
        if !readable {
            assert!(result.unwrap_err().starts_with("it cannot be read"));
        }
    }

    #[test]
    fn suggestions_are_directory_names_only_never_files_or_hidden_ones() {
        let (_tmp, root) = tree(&[
            "Games/",
            "games-old/",
            "gadgets.txt",
            ".git/",
            ".hidden-games/",
            "docs/",
            "games-old/inner/",
        ]);
        let r = root.to_string_lossy();
        assert_eq!(
            suggest(&format!("{r}/ga"), None),
            [format!("{r}/Games"), format!("{r}/games-old")]
        );
        // All of a directory, still without files or hidden ones.
        assert_eq!(
            suggest(&format!("{r}/"), None),
            [
                format!("{r}/Games"),
                format!("{r}/docs"),
                format!("{r}/games-old")
            ]
        );
        assert_eq!(
            suggest(&format!("{r}/games-old/"), None),
            [format!("{r}/games-old/inner")]
        );
        // Spelled as typed, `~` included.
        assert_eq!(suggest("~/do", Some(&root)), ["~/docs"]);
        for none in [
            format!("{r}/.h"),
            format!("{r}/gadgets.txt/"),
            format!("{r}/nope/"),
            "relative/pa".to_string(),
            "~".to_string(),
            String::new(),
        ] {
            assert!(suggest(&none, None).is_empty(), "{none}");
        }
    }

    #[test]
    fn suggestions_are_capped() {
        let names: Vec<String> = (0..MAX_SUGGESTIONS + 5)
            .map(|i| format!("d{i:02}/"))
            .collect();
        let (_tmp, root) = tree(&names.iter().map(String::as_str).collect::<Vec<_>>());
        let listed = suggest(&format!("{}/d", root.to_string_lossy()), None);
        assert_eq!(listed.len(), MAX_SUGGESTIONS);
    }

    #[test]
    fn the_root_never_holds_a_project_for_startup() {
        assert!(!cwd_holds_project(Path::new("/")));
        let (_tmp, root) = tree(&["games/a/functor.json", "docs/"]);
        std::fs::create_dir(root.join(".git")).unwrap();
        assert!(cwd_holds_project(&root));
        assert!(cwd_holds_project(&root.join("games/a")));
        assert!(!cwd_holds_project(&root.join("docs")));
    }
}
