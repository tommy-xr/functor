//! Tasks on disk: one JSON file per task (`tasks::to_stored`) under
//! `<home>/projects/<project key>/tasks/<id>.json`, where home is
//! `FUNCTOR_APP_HOME` or `~/.functor/app`. Never inside the user's repo.

use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use functor_app_core::projects::project_dir;
use functor_app_core::tasks::{from_stored, to_stored};
use functor_app_core::{AppState, Task};

/// `FUNCTOR_APP_HOME` (`env`), else `.functor/app` in `user_home`.
pub fn home(env: Option<OsString>, user_home: Option<PathBuf>) -> Option<PathBuf> {
    env.filter(|e| !e.is_empty())
        .map(PathBuf::from)
        .or_else(|| Some(user_home?.join(".functor").join("app")))
}

/// The directory holding the tasks of the project at `dir` (absolute).
fn tasks_dir(home: &Path, dir: &Path) -> PathBuf {
    home.join("projects").join(key(dir)).join("tasks")
}

/// A directory name for `dir`: its last component, readable, then a hash
/// of the whole path, so `a/b` and `a-b` differ: `counter-1c9a…`.
fn key(dir: &Path) -> String {
    let name: String = dir
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .take(40)
        .collect();
    // FNV-1a, 64-bit: stable across builds, unlike `DefaultHasher`.
    let hash = dir
        .to_string_lossy()
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
        });
    format!("{name}-{hash:016x}")
}

/// Every discovered project's saved tasks, and the highest id of any task
/// file (read or not). An unreadable file, or a second task with an id
/// already loaded, is skipped with a message.
pub fn load(home: &Path, state: &AppState) -> (Vec<Task>, u64) {
    let root = Path::new(&state.repo_root);
    let mut tasks = Vec::new();
    let mut ids = HashSet::new();
    let mut highest = 0;
    for project in &state.projects {
        let dir = tasks_dir(home, &project_dir(root, &project.path));
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "json"))
            .collect();
        files.sort();
        for file in files {
            let stem = file.file_stem().and_then(|s| s.to_str());
            highest = highest.max(stem.and_then(|s| s.parse().ok()).unwrap_or(0));
            let read = std::fs::read_to_string(&file).map_err(|e| e.to_string());
            match read.and_then(|text| from_stored(&text)) {
                Ok(task) if !ids.insert(task.id) => {
                    eprintln!(
                        "functor-app: skipping {}: task {} is loaded already",
                        file.display(),
                        task.id
                    );
                }
                Ok(task) => tasks.push(Task {
                    project: project.path.clone(),
                    ..task
                }),
                Err(e) => eprintln!("functor-app: skipping {}: {e}", file.display()),
            }
        }
    }
    (tasks, highest)
}

/// Writes `task` for the project at `dir`, replacing its file whole.
pub fn save(home: &Path, dir: &Path, task: &Task) -> std::io::Result<()> {
    let tasks = tasks_dir(home, dir);
    std::fs::create_dir_all(&tasks)?;
    let file = tasks.join(format!("{}.json", task.id));
    // Written aside then renamed, so a crash never leaves half a file.
    let tmp = tasks.join(format!("{}.json.tmp", task.id));
    std::fs::write(&tmp, to_stored(task))?;
    std::fs::rename(tmp, file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use functor_app_core::tasks::{Harness, Item, TaskStatus};

    #[test]
    fn home_prefers_the_env_var() {
        let user = || Some(PathBuf::from("/u"));
        assert_eq!(
            home(Some("/env/app".into()), user()),
            Some(PathBuf::from("/env/app"))
        );
        assert_eq!(
            home(Some("".into()), user()),
            Some(Path::new("/u/.functor/app").to_path_buf())
        );
        assert_eq!(home(None, None), None);
    }

    #[test]
    fn keys_are_one_path_component() {
        let hello = key(Path::new("/src/my game/examples/hello world"));
        assert!(hello.starts_with("hello-world-"), "{hello}");
        assert_eq!(hello.len(), "hello-world-".len() + 16);
        assert_ne!(key(Path::new("/r/a/b")), key(Path::new("/r/a-b")));
        assert_eq!(key(Path::new("/r/a/b")), key(Path::new("/r/a/b")));
    }

    #[test]
    fn saved_tasks_load_back_and_bad_files_are_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("repo");
        let project = functor_app_core::projects::Project {
            name: "a".into(),
            path: "a".into(),
            entries: functor_app_core::projects::Entries::Single {
                file: "game.fun".into(),
            },
        };
        let state = AppState::new(&root, &root, vec![project]);
        let home = tmp.path().join("home");
        let task = |id| Task {
            id,
            project: "a".into(),
            title: "hi".into(),
            status: TaskStatus::WaitingOnYou,
            harness: Harness::Codex,
            model: None,
            session_id: Some("s".into()),
            run: None,
            transcript: vec![Item::User { text: "hi".into() }],
        };
        let dir = root.join("a");
        save(&home, &dir, &task(1)).unwrap();
        save(&home, &dir, &task(2)).unwrap();
        let tasks_dir = tasks_dir(&home, &dir);
        std::fs::write(tasks_dir.join("3.json"), "{ not json").unwrap();
        std::fs::write(tasks_dir.join("4.json"), to_stored(&task(1))).unwrap();
        assert_eq!(load(&home, &state), (vec![task(1), task(2)], 4));
        assert!(!root.exists(), "wrote inside the repo");
    }
}
