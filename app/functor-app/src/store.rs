//! Tasks on disk: one JSON file per task (`tasks::to_stored`) under
//! `<home>/projects/<project key>/tasks/<id>.json`, and the project's picked
//! agent in `<home>/projects/<project key>/harness.json` and its canvas
//! layout in `layout.json` beside it, where home is
//! `FUNCTOR_APP_HOME` or `~/.functor/app`. Never inside the user's repo.
//! Task ids are reserved in `<home>/task-ids/`, so app processes sharing a
//! home never give two tasks one id. All of it is private to the user.

use std::collections::{BTreeMap, HashSet};
use std::ffi::OsString;
use std::fs::{DirBuilder, File, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use functor_app_core::projects::project_dir;
use functor_app_core::tasks::{from_stored, to_stored};
use functor_app_core::{AppState, Harness, Layout, Task};

/// `FUNCTOR_APP_HOME` (`env`), else `.functor/app` in `user_home`.
pub fn home(env: Option<OsString>, user_home: Option<PathBuf>) -> Option<PathBuf> {
    env.filter(|e| !e.is_empty())
        .map(PathBuf::from)
        .or_else(|| Some(user_home?.join(".functor").join("app")))
}

/// What the app keeps for the project at `dir` (absolute).
fn project_home(home: &Path, dir: &Path) -> PathBuf {
    home.join("projects").join(key(dir))
}

fn tasks_dir(home: &Path, dir: &Path) -> PathBuf {
    project_home(home, dir).join("tasks")
}

fn harness_file(home: &Path, dir: &Path) -> PathBuf {
    project_home(home, dir).join("harness.json")
}

fn layout_file(home: &Path, dir: &Path) -> PathBuf {
    project_home(home, dir).join("layout.json")
}

/// A directory name for `dir`: its last component, readable, then a hash
/// of the whole path, so `a/b` and `a-b` differ: `counter-1c9a…`.
pub(crate) fn key(dir: &Path) -> String {
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

/// Reserves a task id no other app process on `home` holds: the first from
/// `from` whose marker file this call creates (`create_new`, so the
/// filesystem settles a race). Stores from before markers have none, so
/// `from` must be above every loaded id.
pub fn reserve_id(home: &Path, from: u64) -> io::Result<u64> {
    let dir = home.join("task-ids");
    create_private_dir(&dir)?;
    let mut id = from.max(1);
    loop {
        match create_private(&dir.join(id.to_string())) {
            Ok(_) => return Ok(id),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => id += 1,
            Err(e) => return Err(e),
        }
    }
}

/// Makes an existing `home` 0700 on Unix, so a store from before files were
/// private is closed to other users too; creates it if missing.
pub fn make_private(home: &Path) -> io::Result<()> {
    create_private_dir(home)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(home, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Creates `dir` and its missing parents as 0700 on Unix. Windows has no
/// mode bits: they inherit the home's ACL, the user's own by default.
fn create_private_dir(dir: &Path) -> io::Result<()> {
    let mut builder = DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(dir)
}

/// Creates `file`, which must not exist, as 0600 on Unix (see
/// [`create_private_dir`] for Windows).
fn create_private(file: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(file)
}

/// Each discovered project's picked agent; a missing or unreadable file is
/// no pick.
pub fn load_harnesses(home: &Path, state: &AppState) -> BTreeMap<String, Harness> {
    load_each(home, state, harness_file)
}

/// Writes `harness` as the pick for the project at `dir`.
pub fn save_harness(home: &Path, dir: &Path, harness: Harness) -> std::io::Result<()> {
    save_json(&harness_file(home, dir), &harness)
}

/// Each discovered project's saved layout, as written; the core normalizes
/// it. A missing or unreadable file is no layout.
pub fn load_layouts(home: &Path, state: &AppState) -> BTreeMap<String, Layout> {
    load_each(home, state, layout_file)
}

/// Writes `layout` for the project at `dir`.
pub fn save_layout(home: &Path, dir: &Path, layout: &Layout) -> io::Result<()> {
    save_json(&layout_file(home, dir), layout)
}

/// Each discovered project's value from its `file`, by project path;
/// missing or unreadable ones are left out.
fn load_each<T: serde::de::DeserializeOwned>(
    home: &Path,
    state: &AppState,
    file: fn(&Path, &Path) -> PathBuf,
) -> BTreeMap<String, T> {
    let root = Path::new(&state.repo_root);
    state
        .projects
        .iter()
        .filter_map(|p| {
            let text = std::fs::read_to_string(file(home, &project_dir(root, &p.path)));
            Some((p.path.clone(), serde_json::from_str(&text.ok()?).ok()?))
        })
        .collect()
}

fn save_json<T: serde::Serialize>(file: &Path, value: &T) -> io::Result<()> {
    create_private_dir(file.parent().expect("under home"))?;
    write_whole(file, &serde_json::to_string(value)?)
}

/// Written aside then renamed, so a crash never leaves half a file. The
/// temp name is this write's own (`<file>.<pid>.<n>.tmp`): writers never
/// share one.
pub(crate) fn write_whole(file: &Path, text: &str) -> io::Result<()> {
    static WRITES: AtomicU64 = AtomicU64::new(0);
    let n = WRITES.fetch_add(1, Ordering::Relaxed);
    let mut tmp = file.as_os_str().to_owned();
    tmp.push(format!(".{}.{n}.tmp", std::process::id()));
    let tmp = PathBuf::from(tmp);
    let written = create_private(&tmp)
        .and_then(|mut f| f.write_all(text.as_bytes()))
        .and_then(|()| std::fs::rename(&tmp, file));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// Writes `task` for the project at `dir`, replacing its file whole.
pub fn save(home: &Path, dir: &Path, task: &Task) -> io::Result<()> {
    let tasks = tasks_dir(home, dir);
    create_private_dir(&tasks)?;
    write_whole(&tasks.join(format!("{}.json", task.id)), &to_stored(task))
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
            baseline: None,
            change: None,
            reverting: false,
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

    /// The one project `a` under `<tmp>/repo`, and the home beside it.
    fn one_project(tmp: &Path) -> (AppState, PathBuf, PathBuf) {
        let root = tmp.join("repo");
        let project = functor_app_core::projects::Project {
            name: "a".into(),
            path: "a".into(),
            entries: functor_app_core::projects::Entries::Single {
                file: "game.fun".into(),
            },
        };
        let state = AppState::new(&root, &root, vec![project]);
        (state, root.join("a"), tmp.join("home"))
    }

    fn task(id: u64) -> Task {
        Task {
            id,
            project: "a".into(),
            title: "hi".into(),
            status: TaskStatus::WaitingOnYou,
            harness: Harness::Claude,
            model: None,
            session_id: None,
            run: None,
            baseline: None,
            change: None,
            reverting: false,
            transcript: vec![Item::User { text: "hi".into() }],
        }
    }

    /// Two app processes on one store (here threads, each with its own
    /// hint, as each loaded the same store): every task gets its own id and
    /// file.
    #[test]
    fn stores_allocating_at_once_never_share_an_id_or_lose_a_file() {
        let tmp = tempfile::tempdir().unwrap();
        let (state, dir, home) = one_project(tmp.path());
        let ids: Vec<u64> = std::thread::scope(|s| {
            let workers: Vec<_> = (0..8)
                .map(|_| {
                    s.spawn(|| {
                        let mut next = 1;
                        let mut mine = Vec::new();
                        for _ in 0..10 {
                            let id = reserve_id(&home, next).unwrap();
                            save(&home, &dir, &task(id)).unwrap();
                            next = id + 1;
                            mine.push(id);
                        }
                        mine
                    })
                })
                .collect();
            workers
                .into_iter()
                .flat_map(|w| w.join().unwrap())
                .collect()
        });
        let unique: HashSet<u64> = ids.iter().copied().collect();
        assert_eq!(unique.len(), 80, "{ids:?}");
        let (tasks, highest) = load(&home, &state);
        assert_eq!(tasks.len(), 80);
        assert_eq!(highest, 80);
    }

    /// Concurrent writes of one file each use their own temp file, so none
    /// fails (one shared name lost the race: rename found it gone).
    #[cfg(unix)]
    #[test]
    fn concurrent_saves_of_one_task_all_land() {
        let tmp = tempfile::tempdir().unwrap();
        let (state, dir, home) = one_project(tmp.path());
        std::thread::scope(|s| {
            for _ in 0..8 {
                s.spawn(|| {
                    for _ in 0..50 {
                        save(&home, &dir, &task(1)).unwrap();
                    }
                });
            }
        });
        assert_eq!(load(&home, &state), (vec![task(1)], 1));
        let left: Vec<_> = std::fs::read_dir(tasks_dir(&home, &dir)).unwrap().collect();
        assert_eq!(left.len(), 1, "temp files left behind: {left:?}");
    }

    #[cfg(unix)]
    #[test]
    fn the_store_is_private_to_the_user() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let (_, dir, home) = one_project(tmp.path());
        save(&home, &dir, &task(1)).unwrap();
        save_harness(&home, &dir, Harness::Codex).unwrap();
        reserve_id(&home, 1).unwrap();
        let mode = |p: PathBuf| std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        let tasks = tasks_dir(&home, &dir);
        assert_eq!(mode(tasks.join("1.json")), 0o600);
        assert_eq!(mode(harness_file(&home, &dir)), 0o600);
        assert_eq!(mode(home.join("task-ids").join("1")), 0o600);
        for d in [
            &home,
            &home.join("projects"),
            &project_home(&home, &dir),
            &tasks,
        ] {
            assert_eq!(mode(d.clone()), 0o700, "{}", d.display());
        }
        assert_eq!(mode(home.join("task-ids")), 0o700);
    }

    /// A home from before the store was private (0755, a 0644 task file)
    /// is closed to other users at startup.
    #[cfg(unix)]
    #[test]
    fn an_old_open_home_is_made_private() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let (_, dir, home) = one_project(tmp.path());
        let tasks = tasks_dir(&home, &dir);
        std::fs::create_dir_all(&tasks).unwrap();
        std::fs::write(tasks.join("3.json"), V1_TASK_3).unwrap();
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o755)).unwrap();
        make_private(&home).unwrap();
        let mode = std::fs::metadata(&home).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }

    /// A store written before ids were reserved (format 1, numeric ids, no
    /// markers) loads as it is, and new ids start above it.
    #[test]
    fn a_store_from_before_reservations_loads_and_new_ids_clear_it() {
        let tmp = tempfile::tempdir().unwrap();
        let (state, dir, home) = one_project(tmp.path());
        let tasks = tasks_dir(&home, &dir);
        std::fs::create_dir_all(&tasks).unwrap();
        std::fs::write(tasks.join("3.json"), V1_TASK_3).unwrap();
        let (loaded, highest) = load(&home, &state);
        assert_eq!(loaded.iter().map(|t| t.id).collect::<Vec<_>>(), [3]);
        assert_eq!(loaded[0].model.as_deref(), Some("m"));
        assert_eq!(reserve_id(&home, highest + 1).unwrap(), 4);
    }

    /// Task 3 as the app saved it before ids were reserved.
    const V1_TASK_3: &str = r#"{
  "version": 1,
  "task": {
    "id": 3,
    "project": "a",
    "title": "hi",
    "status": {
      "kind": "waiting_on_you"
    },
    "harness": "claude",
    "model": "m",
    "session_id": "s",
    "run": null,
    "baseline": null,
    "change": null,
    "reverting": false
  },
  "transcript": [
    {
      "kind": "user",
      "text": "hi"
    }
  ]
}"#;

    #[test]
    fn layouts_load_back_per_project() {
        use functor_app_core::{LayoutOp, View};
        let tmp = tempfile::tempdir().unwrap();
        let (state, dir, home) = one_project(tmp.path());
        let mut layout = Layout::initial();
        let side = LayoutOp::Open {
            view: View::File {
                path: "game.fun".into(),
            },
            target: functor_app_core::layout::Target::Side,
        };
        assert!(layout.apply(side, |_| Ok(())).unwrap());
        save_layout(&home, &dir, &layout).unwrap();
        assert_eq!(
            load_layouts(&home, &state),
            BTreeMap::from([("a".into(), layout)])
        );
        // A file that is not a layout is none.
        std::fs::write(layout_file(&home, &dir), "{\"root\": 3}").unwrap();
        assert_eq!(load_layouts(&home, &state), BTreeMap::new());
        assert!(!tmp.path().join("repo").exists(), "wrote inside the repo");
    }

    #[test]
    fn harness_picks_load_back_per_project() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("repo");
        let project = |path: &str| functor_app_core::projects::Project {
            name: path.into(),
            path: path.into(),
            entries: functor_app_core::projects::Entries::Single {
                file: "game.fun".into(),
            },
        };
        let state = AppState::new(&root, &root, vec![project("a"), project("b"), project("c")]);
        let home = tmp.path().join("home");
        save_harness(&home, &root.join("a"), Harness::Claude).unwrap();
        save_harness(&home, &root.join("b"), Harness::Claude).unwrap();
        save_harness(&home, &root.join("b"), Harness::Codex).unwrap();
        // An agent this build does not know is no pick.
        save_harness(&home, &root.join("c"), Harness::Claude).unwrap();
        std::fs::write(harness_file(&home, &root.join("c")), "\"gpt\"").unwrap();
        assert_eq!(
            load_harnesses(&home, &state),
            BTreeMap::from([("a".into(), Harness::Claude), ("b".into(), Harness::Codex)])
        );
        assert!(!root.exists(), "wrote inside the repo");
    }
}
