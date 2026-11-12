//! The git behind a task's change: snapshots of the working tree, the
//! change since one, and reverting it. A snapshot is a git tree written with
//! the app's own index into its own object store (`<home>/repos/<key>`),
//! which reads the repo's objects as an alternate. So the user's repo —
//! HEAD, refs, index, stash and object database — is only ever read; ignored
//! files are not in a snapshot. Hooks never run; the repo's clean and smudge
//! filters do, as for any `git add` or checkout. Git runs as a subprocess,
//! paths go after `--` or on stdin, never through a shell.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use functor_app_core::changes::{
    file_diffs, nul_list, parse_numstat, patch_plan, revert_plan, Change, FileChange, FileDiff,
    RevertOp, MAX_FILES,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// The most of a patch read; a longer one is cut and its last file summarised.
const PATCH_BYTES: u64 = 8 * 1024 * 1024;

/// Why a revert cannot start once its snapshot is gone.
pub const BASELINE_GONE: &str = "the snapshot taken when the task started is no longer \
     available (the app's data was removed), so its change cannot be reverted";

/// Environment that points git elsewhere; dropped so the repo at `root` is the one used.
const REPO_ENV: [&str; 6] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
];

pub struct Git {
    /// The work tree's top level.
    root: PathBuf,
    /// The app's directory for this repo; its `objects` is the store.
    dir: PathBuf,
    /// The repo's own objects, read as an alternate.
    objects: PathBuf,
}

impl Git {
    /// The git of `root` when it is a work tree's top level; `None` outside
    /// git (or without git). Runs at startup, before the core loop.
    pub fn open(root: &Path, home: &Path) -> Option<Self> {
        let mut command = std::process::Command::new("git");
        command
            .arg("-C")
            .arg(root)
            .args(["rev-parse", "--path-format=absolute", "--show-toplevel"])
            .args(["--git-path", "objects"])
            .stdin(Stdio::null())
            .stderr(Stdio::null());
        for var in REPO_ENV {
            command.env_remove(var);
        }
        let out = command.output().ok().filter(|o| o.status.success())?;
        let text = String::from_utf8(out.stdout).ok()?;
        let mut lines = text.lines().map(PathBuf::from);
        let (top, objects) = (lines.next()?, lines.next()?);
        if std::fs::canonicalize(&top).ok()? != std::fs::canonicalize(root).ok()? {
            return None;
        }
        let dir = home
            .join("repos")
            .join(crate::store::key(&std::fs::canonicalize(root).ok()?));
        for sub in ["objects/info", "objects/pack"] {
            std::fs::create_dir_all(dir.join(sub)).ok()?;
        }
        Some(Self {
            root: root.to_path_buf(),
            dir,
            objects,
        })
    }

    /// `git -C <root> <args>` against the store, with `index` as its index.
    fn command(&self, args: &[&str], index: Option<&Path>) -> tokio::process::Command {
        let mut command = tokio::process::Command::new("git");
        command.arg("-C").arg(&self.root);
        // No hooks (writing an index runs `post-index-change`), no fsmonitor
        // daemon, and an index wholly the app's (never a split one shared
        // with the repo's `.git`).
        let hooks = self.dir.join("no-hooks");
        command
            .arg("-c")
            .arg(format!("core.hooksPath={}", hooks.display()))
            .args(["-c", "core.fsmonitor=false", "-c", "core.splitIndex=false"])
            .args(args);
        for var in REPO_ENV {
            command.env_remove(var);
        }
        command
            .env("GIT_OBJECT_DIRECTORY", self.dir.join("objects"))
            .env("GIT_ALTERNATE_OBJECT_DIRECTORIES", &self.objects)
            // Paths are paths, never globs or `:(magic)`.
            .env("GIT_LITERAL_PATHSPECS", "1")
            // No opportunistic writes to the user's index.
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(index) = index {
            command.env("GIT_INDEX_FILE", index);
        }
        command
    }

    /// Runs git; its stdout, or why it failed.
    async fn git(&self, args: &[&str], index: Option<&Path>) -> Result<String, String> {
        let out = self
            .command(args, index)
            .output()
            .await
            .map_err(|e| format!("cannot run git: {e}"))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            return Err(format!("git {}: {}", args[0], err.trim()));
        }
        String::from_utf8(out.stdout).map_err(|_| format!("git {}: output is not UTF-8", args[0]))
    }

    /// The working tree now, tracked and untracked-but-not-ignored files,
    /// as a tree in the store.
    pub async fn snapshot(&self) -> Result<String, String> {
        let index = TempIndex::new(&self.dir);
        let kept = self.dir.join("index");
        // Seeded from the last snapshot's index, so git hashes only what
        // changed since; the first starts from HEAD (none before a commit).
        if tokio::fs::copy(&kept, &index.0).await.is_err() {
            let _ = self.git(&["read-tree", "HEAD"], Some(&index.0)).await;
        }
        self.git(&["add", "-A"], Some(&index.0)).await?;
        let tree = self.git(&["write-tree"], Some(&index.0)).await?;
        let _ = tokio::fs::rename(&index.0, &kept).await;
        Ok(tree.trim().to_string())
    }

    /// The files that differ between `base` and the working tree now, and
    /// the snapshot they were measured against.
    async fn files(&self, base: &str) -> Result<(Vec<FileChange>, String), String> {
        let now = self.snapshot().await?;
        let out = self
            .git(
                &["diff", "--numstat", "-z", "-M", "--no-ext-diff", base, &now],
                None,
            )
            .await?;
        Ok((parse_numstat(&out)?, now))
    }

    /// The change since `base`.
    pub async fn change(&self, base: &str) -> Result<Change, String> {
        let (files, now) = self.files(base).await?;
        Ok(Change::from_files(files, now))
    }

    /// The change since `base` and its full diff: at most [`MAX_FILES`]
    /// files (and how many more), lines capped as `patch_plan` says.
    pub async fn diff(&self, base: &str) -> Result<(Change, Vec<FileDiff>, u32), String> {
        let (files, now) = self.files(base).await?;
        let shown = &files[..files.len().min(MAX_FILES)];
        let planned = patch_plan(shown);
        let mut args = vec![
            "diff",
            "-M",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            base,
            &now,
            "--",
        ];
        for (file, _) in shown.iter().zip(&planned).filter(|(_, p)| **p) {
            args.extend(file.from.as_deref());
            args.push(&file.path);
        }
        let (patch, truncated) = match args.last() == Some(&"--") {
            true => (String::new(), false),
            false => self.capped(&args).await?,
        };
        let diffs = file_diffs(shown, &planned, &patch, truncated);
        let more = (files.len() - shown.len()) as u32;
        Ok((Change::from_files(files, now), diffs, more))
    }

    /// Git's stdout up to [`PATCH_BYTES`], and whether it was cut there.
    async fn capped(&self, args: &[&str]) -> Result<(String, bool), String> {
        let mut child = self
            .command(args, None)
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("cannot run git: {e}"))?;
        let mut out = Vec::new();
        let stdout = child.stdout.take().expect("piped");
        stdout
            .take(PATCH_BYTES + 1)
            .read_to_end(&mut out)
            .await
            .map_err(|e| format!("git diff: {e}"))?;
        let truncated = out.len() as u64 > PATCH_BYTES;
        if truncated {
            out.truncate(PATCH_BYTES as usize);
            let _ = child.kill().await;
        } else {
            let status = child.wait().await.map_err(|e| format!("git diff: {e}"))?;
            if !status.success() {
                return Err(format!("git diff: {status}"));
            }
        }
        Ok((String::from_utf8_lossy(&out).into_owned(), truncated))
    }

    /// Puts each of `paths` (the change the user reviewed) that still
    /// differs from `base` back as it was there: created files deleted, the
    /// rest checked out from `base`. Refused, with nothing touched, when the
    /// snapshot or one of its files is gone, or when it would reach through a
    /// symlink or remove anything outside those paths.
    pub async fn revert(&self, base: &str, paths: &[String]) -> Result<(), String> {
        let tree = format!("{base}^{{tree}}");
        if self.git(&["cat-file", "-e", &tree], None).await.is_err() {
            return Err(BASELINE_GONE.to_string());
        }
        let now = self.snapshot().await?;
        let names = self
            .git(
                &["diff", "--name-only", "-z", "--no-renames", base, &now],
                None,
            )
            .await?;
        let reviewed: HashSet<&str> = paths.iter().map(String::as_str).collect();
        let changed: Vec<String> = nul_list(&names)
            .into_iter()
            .filter(|p| reviewed.contains(p.as_str()))
            .collect();
        if changed.is_empty() {
            return Ok(());
        }
        let listing = self
            .git(&["ls-tree", "-r", "-z", "--full-tree", base], None)
            .await?;
        let plan = revert_plan(&listing, &changed)?;
        let writes: Vec<String> = plan
            .iter()
            .filter_map(|op| match op {
                RevertOp::Write { path, .. } => Some(path.clone()),
                RevertOp::Delete { .. } => None,
            })
            .collect();
        let oids = plan.iter().filter_map(|op| match op {
            RevertOp::Write { oid, .. } => Some(oid.as_str()),
            RevertOp::Delete { .. } => None,
        });
        let input: String = oids.map(|oid| format!("{oid}\n")).collect();
        let checked = self
            .stdin(&["cat-file", "--batch-check"], None, &input)
            .await?;
        if checked.lines().any(|l| l.ends_with(" missing")) {
            return Err(BASELINE_GONE.to_string());
        }

        let root = self.root.clone();
        let deletes: Vec<String> = plan
            .iter()
            .filter_map(|op| match op {
                RevertOp::Delete { path } => Some(path.clone()),
                RevertOp::Write { .. } => None,
            })
            .collect();
        let checked = writes.clone();
        tokio::task::spawn_blocking(move || {
            preflight(&root, &deletes, &checked)?;
            delete_all(&root, &deletes)
        })
        .await
        .map_err(|e| e.to_string())??;
        if writes.is_empty() {
            return Ok(());
        }
        // Git writes the files (modes, symlinks, filters as a checkout would),
        // from an index holding `base`, for exactly these paths.
        let index = TempIndex::new(&self.dir);
        self.git(&["read-tree", base], Some(&index.0)).await?;
        let paths: String = writes.iter().map(|p| format!("{p}\0")).collect();
        self.stdin(
            &["checkout-index", "-f", "-z", "--stdin"],
            Some(&index.0),
            &paths,
        )
        .await?;
        Ok(())
    }

    /// Runs git with `input` on its stdin; its stdout.
    async fn stdin(
        &self,
        args: &[&str],
        index: Option<&Path>,
        input: &str,
    ) -> Result<String, String> {
        let mut child = self
            .command(args, index)
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot run git: {e}"))?;
        let mut stdin = child.stdin.take().expect("piped");
        let input = input.as_bytes().to_vec();
        // Written alongside the read, so a full pipe cannot stall either side.
        let write = tokio::spawn(async move {
            let _ = stdin.write_all(&input).await;
        });
        let out = child
            .wait_with_output()
            .await
            .map_err(|e| format!("git {}: {e}", args[0]))?;
        let _ = write.await;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            return Err(format!("git {}: {}", args[0], err.trim()));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

/// Refuses a revert that would reach through a symlink or remove what it
/// does not own: each path's leading directories must be directories (or
/// absent, or a file or link the revert deletes), and a path to write must
/// not be a directory holding anything the revert does not delete. A
/// checkout would otherwise remove such a directory whole, ignored files
/// and all.
fn preflight(root: &Path, deletes: &[String], writes: &[String]) -> Result<(), String> {
    let deleted: HashSet<&str> = deletes.iter().map(String::as_str).collect();
    for path in deletes.iter().chain(writes) {
        let mut prefix = String::new();
        for part in path.split('/').take(path.split('/').count() - 1) {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(part);
            match std::fs::symlink_metadata(root.join(&prefix)) {
                Err(_) => break,
                Ok(m) if m.is_dir() => {}
                Ok(_) if deleted.contains(prefix.as_str()) => break,
                Ok(_) => return Err(format!("{path}: {prefix} is not a directory")),
            }
        }
    }
    for path in writes {
        let dir = root.join(path);
        if !std::fs::symlink_metadata(&dir).is_ok_and(|m| m.is_dir()) {
            continue;
        }
        let mut stack = vec![dir];
        while let Some(d) = stack.pop() {
            let entries = std::fs::read_dir(&d).map_err(|e| format!("{path}: {e}"))?;
            for entry in entries {
                let entry = entry.map_err(|e| format!("{path}: {e}"))?;
                let full = entry.path();
                if entry.file_type().is_ok_and(|t| t.is_dir()) {
                    stack.push(full);
                    continue;
                }
                let rel = full.strip_prefix(root).unwrap_or(&full);
                let rel = rel.to_string_lossy().replace('\\', "/");
                if !deleted.contains(rel.as_str()) {
                    return Err(format!(
                        "{path} is a directory now, holding {rel}: move it away to revert"
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Removes each of `paths` (repo-relative) under `root`, then directories
/// left empty by it. Refuses a path reached through a symlink, which could
/// lead outside the repo, or one that is a directory now.
fn delete_all(root: &Path, paths: &[String]) -> Result<(), String> {
    for path in paths {
        let file = root.join(path);
        let mut dir = root.to_path_buf();
        let mut reachable = true;
        for part in Path::new(path)
            .parent()
            .into_iter()
            .flat_map(Path::components)
        {
            dir.push(part);
            match std::fs::symlink_metadata(&dir) {
                Ok(m) if m.file_type().is_symlink() => {
                    return Err(format!("{path}: {} is a symlink", dir.display()));
                }
                Ok(m) if m.is_dir() => {}
                // No directory there: nothing to delete.
                _ => reachable = false,
            }
            if !reachable {
                break;
            }
        }
        if !reachable {
            continue;
        }
        match std::fs::symlink_metadata(&file) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("{path}: {e}")),
            Ok(m) if m.is_dir() => return Err(format!("{path} is a directory now")),
            Ok(_) => std::fs::remove_file(&file).map_err(|e| format!("{path}: {e}"))?,
        }
        // As git does: a directory the file alone kept goes with it.
        let mut parent = file.parent();
        while let Some(dir) = parent.filter(|d| *d != root) {
            if std::fs::remove_dir(dir).is_err() {
                break;
            }
            parent = dir.parent();
        }
    }
    Ok(())
}

/// Sends task `task`'s change since `baseline` to the core; a failure is
/// only logged, leaving the last summary standing.
pub async fn measure(
    git: &Git,
    task: u64,
    baseline: String,
    events: &tokio::sync::mpsc::UnboundedSender<functor_app_core::Event>,
) {
    match git.change(&baseline).await {
        Ok(change) => {
            let _ = events.send(functor_app_core::Event::Change {
                task,
                baseline,
                change,
            });
        }
        Err(e) => eprintln!("functor-app: cannot measure task {task}'s change: {e}"),
    }
}

/// An index file for one git command, removed when dropped.
struct TempIndex(PathBuf);

impl TempIndex {
    fn new(dir: &Path) -> Self {
        Self(dir.join(format!("index-{}", uuid::Uuid::new_v4().simple())))
    }
}

impl Drop for TempIndex {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        let mut lock = self.0.as_os_str().to_owned();
        lock.push(".lock");
        let _ = std::fs::remove_file(lock);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::process::Command;

    /// Every git in these tests, the app's included, reads no user or
    /// system config.
    pub fn isolate() {
        static HOME: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
        let home = HOME.get_or_init(|| tempfile::tempdir().unwrap());
        std::env::set_var("HOME", home.path());
        std::env::set_var("XDG_CONFIG_HOME", home.path());
        std::env::set_var("GIT_CONFIG_GLOBAL", "/dev/null");
        std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
    }

    /// A throwaway repo with a first commit, and the app home beside it.
    pub struct Repo {
        _tmp: tempfile::TempDir,
        pub root: PathBuf,
        pub home: PathBuf,
    }

    impl Repo {
        pub fn new(files: &[(&str, &str)]) -> Self {
            isolate();
            let tmp = tempfile::tempdir().unwrap();
            let root = std::fs::canonicalize(tmp.path()).unwrap().join("repo");
            let home = root.with_file_name("home");
            std::fs::create_dir_all(&root).unwrap();
            let repo = Self {
                _tmp: tmp,
                root,
                home,
            };
            repo.git(&["init", "-q", "-b", "main"]);
            for (path, text) in files {
                repo.write(path, text);
            }
            repo.git(&["add", "-A"]);
            repo.git(&["commit", "-q", "-m", "first"]);
            repo
        }

        pub fn git(&self, args: &[&str]) -> String {
            let out = Command::new("git")
                .arg("-C")
                .arg(&self.root)
                .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
                .args(args)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env(
                    "XDG_CONFIG_HOME",
                    std::env::var_os("XDG_CONFIG_HOME").unwrap(),
                )
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?}: {out:?}");
            String::from_utf8(out.stdout).unwrap()
        }

        pub fn write(&self, path: &str, text: &str) {
            let file = self.root.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, text).unwrap();
        }

        pub fn read(&self, path: &str) -> Option<String> {
            std::fs::read_to_string(self.root.join(path)).ok()
        }

        fn open(&self) -> Git {
            Git::open(&self.root, &self.home).expect("a git repo")
        }

        /// What a snapshot must leave as it was.
        fn user_state(&self) -> (String, String, String, Vec<u8>, usize) {
            let git_dir = self.root.join(".git");
            let objects = walk(&git_dir.join("objects")).len();
            (
                self.git(&["status", "--porcelain=v2", "--untracked-files=all"]),
                self.git(&["stash", "list"]),
                self.git(&["rev-parse", "HEAD"]),
                std::fs::read(git_dir.join("index")).unwrap(),
                objects,
            )
        }
    }

    fn walk(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for entry in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                match entry.path() {
                    p if p.is_dir() => stack.push(p),
                    p => out.push(p),
                }
            }
        }
        out
    }

    fn counts(change: &Change) -> Vec<(&str, u32, u32)> {
        change
            .files
            .iter()
            .map(|f| (f.path.as_str(), f.added, f.removed))
            .collect()
    }

    /// The fixture every revert test shares: one change of each kind, an
    /// ignored file, and a file dirty before the task started.
    fn repo() -> Repo {
        let repo = Repo::new(&[
            (".gitignore", "*.log\n"),
            ("game.fun", "let a = 1\nlet b = 2\n"),
            ("gone.fun", "let gone = 0\n"),
            ("dirty.txt", "committed\n"),
        ]);
        repo.write("dirty.txt", "dirty before the task\n");
        repo.write("notes.txt", "untracked before the task\n");
        repo.write("staged.txt", "staged before the task\n");
        repo.git(&["add", "staged.txt"]);
        repo
    }

    /// What the "agent" does after the baseline.
    fn edit(repo: &Repo) {
        repo.write("game.fun", "let a = 1\nlet b = 3\nlet c = 4\n");
        repo.write("dir/new file.fun", "let n = 1\n");
        std::fs::remove_file(repo.root.join("gone.fun")).unwrap();
        repo.write("notes.txt", "untracked, then edited\n");
        repo.write("debug.log", "ignored\n");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_snapshot_leaves_the_users_git_state_alone() {
        let repo = repo();
        // `git status` may refresh the index: settle it before comparing.
        let before = repo.user_state();
        let git = repo.open();
        let tree = git.snapshot().await.unwrap();
        let again = git.snapshot().await.unwrap();
        assert_eq!(tree, again, "the same tree twice");
        assert_eq!(repo.user_state(), before);
        // The tree is in the app's store, not the repo's.
        let probe = Command::new("git")
            .arg("-C")
            .arg(&repo.root)
            .args(["cat-file", "-e", &tree])
            .status()
            .unwrap();
        assert!(!probe.success(), "the snapshot reached the repo's objects");
        // Nothing of the app's in the repo, and no index left behind.
        assert!(!walk(&git.dir)
            .iter()
            .any(|p| p.to_string_lossy().contains("index-")));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn the_change_has_every_kind_of_edit_and_no_ignored_file() {
        let repo = repo();
        let git = repo.open();
        let base = git.snapshot().await.unwrap();
        assert!(git.change(&base).await.unwrap().is_empty());
        edit(&repo);
        let change = git.change(&base).await.unwrap();
        assert_eq!(
            counts(&change),
            [
                ("dir/new file.fun", 1, 0),
                ("game.fun", 2, 1),
                ("gone.fun", 0, 1),
                ("notes.txt", 1, 1),
            ]
        );
        let (_, diffs, more) = git.diff(&base).await.unwrap();
        assert_eq!(more, 0);
        let game = diffs.iter().find(|d| d.file.path == "game.fun").unwrap();
        let text: Vec<String> = game
            .lines
            .iter()
            .map(|l| format!("{:?} {}", l.kind, l.text))
            .collect();
        assert_eq!(
            text,
            [
                "Hunk @@ -1,2 +1,3 @@",
                "Context let a = 1",
                "Removed let b = 2",
                "Added let b = 3",
                "Added let c = 4",
            ]
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_rename_shows_as_one_file() {
        let body: String = (0..20).map(|i| format!("line {i}\n")).collect();
        let repo = Repo::new(&[("old.fun", &body)]);
        let git = repo.open();
        let base = git.snapshot().await.unwrap();
        std::fs::remove_file(repo.root.join("old.fun")).unwrap();
        repo.write("new name.fun", &format!("{body}line 20\n"));
        let (change, diffs, _) = git.diff(&base).await.unwrap();
        assert_eq!(change.files.len(), 1);
        assert_eq!(change.files[0].from.as_deref(), Some("old.fun"));
        assert_eq!(change.files[0].path, "new name.fun");
        assert_eq!(diffs[0].lines.last().unwrap().text, "line 20");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn revert_restores_the_change_and_nothing_else() {
        let repo = repo();
        let git = repo.open();
        let base = git.snapshot().await.unwrap();
        edit(&repo);
        let reviewed = git.change(&base).await.unwrap().paths();
        // Edited after the review: not part of what Revert was shown.
        repo.write("later.txt", "written after the review\n");
        git.revert(&base, &reviewed).await.unwrap();
        assert_eq!(
            repo.read("later.txt").as_deref(),
            Some("written after the review\n")
        );
        std::fs::remove_file(repo.root.join("later.txt")).unwrap();
        assert_eq!(
            repo.read("game.fun").as_deref(),
            Some("let a = 1\nlet b = 2\n")
        );
        assert_eq!(repo.read("gone.fun").as_deref(), Some("let gone = 0\n"));
        assert_eq!(repo.read("dir/new file.fun"), None);
        assert!(
            !repo.root.join("dir").exists(),
            "its emptied directory stays"
        );
        assert_eq!(
            repo.read("notes.txt").as_deref(),
            Some("untracked before the task\n")
        );
        // Dirty before the task, and ignored: untouched.
        assert_eq!(
            repo.read("dirty.txt").as_deref(),
            Some("dirty before the task\n")
        );
        assert_eq!(repo.read("debug.log").as_deref(), Some("ignored\n"));
        assert!(git.change(&base).await.unwrap().is_empty());
        // Git sees what it saw before the task.
        assert_eq!(
            repo.git(&["status", "--porcelain"]),
            " M dirty.txt\nA  staged.txt\n?? notes.txt\n"
        );
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn revert_restores_modes_and_never_writes_through_a_symlink() {
        use std::os::unix::fs::PermissionsExt;
        let repo = Repo::new(&[("run.sh", "echo hi\n"), ("data/a.txt", "a\n")]);
        let outside = tempfile::tempdir().unwrap();
        let git = repo.open();
        let base = git.snapshot().await.unwrap();
        let run = repo.root.join("run.sh");
        std::fs::set_permissions(&run, std::fs::Permissions::from_mode(0o755)).unwrap();
        // The directory becomes a link to somewhere else.
        std::fs::remove_dir_all(repo.root.join("data")).unwrap();
        std::os::unix::fs::symlink(outside.path(), repo.root.join("data")).unwrap();
        let reviewed = git.change(&base).await.unwrap().paths();
        git.revert(&base, &reviewed).await.unwrap();
        let mode = std::fs::metadata(&run).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0, "{mode:o}");
        assert!(repo.root.join("data").is_dir());
        assert!(!std::fs::symlink_metadata(repo.root.join("data"))
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(repo.read("data/a.txt").as_deref(), Some("a\n"));
        assert_eq!(walk(outside.path()), Vec::<PathBuf>::new());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn revert_without_its_snapshot_fails_and_touches_nothing() {
        let repo = repo();
        let git = repo.open();
        let base = git.snapshot().await.unwrap();
        edit(&repo);
        let reviewed = git.change(&base).await.unwrap().paths();
        // As if the app's store were removed, or pruned.
        std::fs::remove_dir_all(git.dir.join("objects")).unwrap();
        std::fs::create_dir_all(git.dir.join("objects")).unwrap();
        assert_eq!(
            git.revert(&base, &reviewed).await,
            Err(BASELINE_GONE.to_string())
        );
        assert_eq!(
            repo.read("game.fun").as_deref(),
            Some("let a = 1\nlet b = 3\nlet c = 4\n")
        );
        assert_eq!(
            repo.read("dir/new file.fun").as_deref(),
            Some("let n = 1\n")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn revert_never_removes_a_directory_holding_files_outside_the_change() {
        let repo = Repo::new(&[(".gitignore", "*.log\n"), ("data", "a file\n")]);
        let git = repo.open();
        let base = git.snapshot().await.unwrap();
        // The file becomes a directory, with an ignored file in it.
        std::fs::remove_file(repo.root.join("data")).unwrap();
        repo.write("data/new.txt", "new\n");
        repo.write("data/keep.log", "precious\n");
        let reviewed = git.change(&base).await.unwrap().paths();
        let refused = git.revert(&base, &reviewed).await.unwrap_err();
        assert!(refused.contains("data/keep.log"), "{refused}");
        // Nothing was touched.
        assert_eq!(repo.read("data/new.txt").as_deref(), Some("new\n"));
        assert_eq!(repo.read("data/keep.log").as_deref(), Some("precious\n"));
        // With the directory holding only the change, the file comes back.
        std::fs::remove_file(repo.root.join("data/keep.log")).unwrap();
        git.revert(&base, &reviewed).await.unwrap();
        assert_eq!(repo.read("data").as_deref(), Some("a file\n"));
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn a_snapshot_runs_none_of_the_repos_hooks() {
        use std::os::unix::fs::PermissionsExt;
        let repo = repo();
        let marker = repo.root.with_file_name("hook-ran");
        let hook = repo.root.join(".git/hooks/post-index-change");
        std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
        let script = format!("#!/bin/sh\ntouch '{}'\n", marker.display());
        std::fs::write(&hook, script).unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        let git = repo.open();
        let base = git.snapshot().await.unwrap();
        edit(&repo);
        git.change(&base).await.unwrap();
        assert!(!marker.exists(), "a hook ran");
        // The hook is live: the user's own `git add` runs it.
        repo.git(&["add", "game.fun"]);
        assert!(marker.exists(), "the hook is not installed");
    }

    #[test]
    fn outside_a_repo_there_is_no_git() {
        isolate();
        let tmp = tempfile::tempdir().unwrap();
        assert!(Git::open(tmp.path(), &tmp.path().join("home")).is_none());
        // A directory inside a repo is not its top level.
        let repo = Repo::new(&[("a/x.fun", "")]);
        assert!(Git::open(&repo.root.join("a"), &repo.home).is_none());
        assert!(Git::open(&repo.root, &repo.home).is_some());
    }
}
