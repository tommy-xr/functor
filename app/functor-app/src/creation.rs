//! New projects: the templates the located `functor` lists, checking where
//! a project would go, and creating it there. Processes take argv only (the
//! template is one the core found in the list); nothing is written outside
//! the folder the user named, and only folders this creates are removed,
//! and only while empty.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use functor_app_core::creation::{
    folder_name, parse_templates, plan, CreateFailure, NewProjectRequest, Plan, Target, Template,
};

/// `functor init --list --json`: the templates, or why there are none.
pub async fn list(functor: &Path) -> Result<Vec<Template>, String> {
    let out = tokio::process::Command::new(functor)
        .args(["init", "--list", "--json"])
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| format!("cannot run {}: {e}", functor.display()))?;
    if !out.status.success() {
        return Err(format!(
            "{} cannot list templates: it may be older than this app",
            functor.display()
        ));
    }
    parse_templates(&String::from_utf8_lossy(&out.stdout))
}

/// What stands at `dir`. A symlink is not followed: it is in the way.
pub fn target(dir: &Path) -> Target {
    let Ok(meta) = std::fs::symlink_metadata(dir) else {
        return Target::Absent;
    };
    if !meta.is_dir() {
        return Target::NotAFolder;
    }
    match std::fs::read_dir(dir) {
        Err(e) => Target::Unreadable(e.to_string()),
        Ok(entries) => {
            let mut names: Vec<String> = entries
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            match names.is_empty() {
                true => Target::Empty,
                false => Target::Taken(names),
            }
        }
    }
}

/// `dir`'s nearest existing ancestor (itself included), canonical, and the
/// names below it that do not exist yet, top first.
fn existing_ancestor(dir: &Path) -> Option<(PathBuf, Vec<PathBuf>)> {
    let mut missing = Vec::new();
    for at in dir.ancestors() {
        match std::fs::canonicalize(at) {
            Ok(real) => {
                missing.reverse();
                return Some((real, missing));
            }
            Err(_) => missing.push(PathBuf::from(at.file_name()?)),
        }
    }
    None
}

/// Whether `dir` (or its nearest existing ancestor) is in a git work tree;
/// false without git.
fn in_git(git: &OsStr, dir: &Path) -> bool {
    let mut command = std::process::Command::new(git);
    command
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--is-inside-work-tree"])
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    for var in crate::changes::REPO_ENV {
        command.env_remove(var);
    }
    command
        .output()
        .is_ok_and(|o| o.status.success() && o.stdout.starts_with(b"true"))
}

/// The project's `folder` under `request`'s parent (its `~` expanded and
/// its existing part resolved), that existing part, and the names under it
/// to create, top first.
fn locate(
    request: &NewProjectRequest,
    folder: &str,
) -> Result<(PathBuf, PathBuf, Vec<PathBuf>), String> {
    let parent = crate::folders::expand(&request.parent, std::env::home_dir().as_deref())
        .filter(|p| p.is_absolute())
        .ok_or("give the folder's full path")?;
    let dir = parent.join(folder);
    let (base, missing) = existing_ancestor(&dir).ok_or("no part of that folder exists")?;
    let path = missing.iter().fold(base.clone(), |p, m| p.join(m));
    Ok((path, base, missing))
}

/// What `request` would do, checked against the disk now.
pub fn check(request: &NewProjectRequest) -> Result<Plan, String> {
    check_with(OsStr::new("git"), request)
}

fn check_with(git: &OsStr, request: &NewProjectRequest) -> Result<Plan, String> {
    let (path, base, _) = locate(request, &folder_name(&request.name)?)?;
    plan(
        request,
        &path.to_string_lossy(),
        &target(&path),
        in_git(git, &base),
    )
}

/// What creating did: the project's folder, and a note when git could not
/// track it.
pub struct Created {
    pub path: String,
    pub note: Option<String>,
}

/// Creates `request`'s project in `folder` (its name, as the core checked
/// it) under its parent: the folder (and missing parents), the
/// template's files (`functor -d <folder> init <template>`), then `git
/// init` if planned. On failure, folders this made are removed if empty.
pub async fn create(
    functor: &Path,
    request: &NewProjectRequest,
    folder: &str,
) -> Result<Created, CreateFailure> {
    create_with(functor, OsStr::new("git"), request, folder).await
}

async fn create_with(
    functor: &Path,
    git: &OsStr,
    request: &NewProjectRequest,
    folder: &str,
) -> Result<Created, CreateFailure> {
    let fail = |reason: String, done: Vec<String>, not_done: &[String]| CreateFailure {
        reason,
        done,
        not_done: not_done.to_vec(),
    };
    let plan = check_with(git, request).map_err(|e| fail(e, vec![], &[]))?;
    let (path, base, missing) =
        locate(request, folder).map_err(|e| fail(e, vec![], &plan.steps))?;
    let mut done = Vec::new();
    // Made one at a time, so each one made is known (and only those undone).
    let mut made = Vec::new();
    let mut at = base;
    for name in &missing {
        at = at.join(name);
        if let Err(e) = std::fs::create_dir(&at) {
            undo(&made, &mut done);
            let reason = format!("cannot create {}: {e}", at.display());
            return Err(fail(reason, done, &plan.steps));
        }
        made.push(at.clone());
    }
    if !made.is_empty() {
        done.push(plan.steps[0].clone());
    }
    let out = tokio::process::Command::new(functor)
        .arg("-d")
        .arg(&path)
        .args(["init", &request.template])
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await;
    let failed = match out {
        Ok(out) if out.status.success() => None,
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout) + String::from_utf8_lossy(&out.stderr);
            let lines: std::collections::VecDeque<String> =
                text.lines().map(str::to_string).collect();
            let cause = crate::session::cause(&lines)
                .unwrap_or("no output")
                .to_string();
            Some(format!("functor init {} failed: {cause}", request.template))
        }
        Err(e) => Some(format!("cannot run {}: {e}", functor.display())),
    };
    if let Some(reason) = failed {
        undo(&made, &mut done);
        return Err(fail(reason, done, &plan.steps[1..]));
    }
    done.push(plan.steps[1].clone());
    let mut note = None;
    if plan.git_init {
        let mut command = tokio::process::Command::new(git);
        command
            .arg("-C")
            .arg(&path)
            .args(["init", "-q"])
            .stdin(Stdio::null())
            .kill_on_drop(true);
        for var in crate::changes::REPO_ENV {
            command.env_remove(var);
        }
        match command.output().await {
            Ok(out) if out.status.success() => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                note = Some(
                    "git is not installed, so this project's changes are not tracked: \
                     review features are off"
                        .to_string(),
                );
            }
            Ok(out) => {
                let reason = format!(
                    "git init failed: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                );
                return Err(fail(reason, done, &plan.steps[2..]));
            }
            Err(e) => return Err(fail(format!("cannot run git: {e}"), done, &plan.steps[2..])),
        }
    }
    Ok(Created {
        path: path.to_string_lossy().into_owned(),
        note,
    })
}

/// Removes the folders made, deepest first, while empty; says so in `done`.
fn undo(made: &[PathBuf], done: &mut Vec<String>) {
    for dir in made.iter().rev() {
        if std::fs::remove_dir(dir).is_ok() && dir == made.last().expect("made") {
            done.push(format!("Removed the empty folder {} again", dir.display()));
        }
    }
}

// Spelled with `/` and run with `sh` scripts as `functor`, as on Unix.
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn request(name: &str, parent: &Path) -> NewProjectRequest {
        NewProjectRequest {
            name: name.into(),
            parent: parent.to_string_lossy().into_owned(),
            template: "3d".into(),
            git: true,
            describe: String::new(),
        }
    }

    fn tmp() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        (tmp, root)
    }

    /// A `functor` that runs `script` (`$2` is the `-d` folder).
    fn fake_functor(dir: &Path, script: &str) -> PathBuf {
        let path = dir.join("fake-functor");
        std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    const NO_GIT: &str = "functor-app-test-no-such-git";

    #[test]
    fn what_stands_at_a_target() {
        let (_tmp, root) = tmp();
        std::fs::create_dir_all(root.join("empty")).unwrap();
        std::fs::create_dir_all(root.join("taken/sub")).unwrap();
        std::fs::write(root.join("taken/b.txt"), "").unwrap();
        std::fs::write(root.join("file"), "").unwrap();
        std::os::unix::fs::symlink(root.join("empty"), root.join("link")).unwrap();
        assert_eq!(target(&root.join("none")), Target::Absent);
        assert_eq!(target(&root.join("empty")), Target::Empty);
        assert_eq!(
            target(&root.join("taken")),
            Target::Taken(vec!["b.txt".into(), "sub".into()])
        );
        assert_eq!(target(&root.join("file")), Target::NotAFolder);
        // A link to an empty folder is still in the way.
        assert_eq!(target(&root.join("link")), Target::NotAFolder);
    }

    #[test]
    fn a_check_resolves_the_parent_and_offers_git_outside_a_work_tree() {
        let (_tmp, root) = tmp();
        let plan = check(&request("My Game", &root.join("new/parent"))).unwrap();
        assert_eq!(plan.path, root.join("new/parent/My-Game").to_string_lossy());
        assert!(plan.git_init && !plan.in_git);
        assert_eq!(plan.steps[0], format!("Create {}", plan.path));
        // Inside a work tree (here, a `.git` file git accepts as one), none.
        let repo = root.join("repo");
        std::fs::create_dir(&repo).unwrap();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .output()
                .unwrap()
        };
        assert!(git(&["init", "-q"]).status.success());
        let plan = check(&request("x", &repo.join("games"))).unwrap();
        assert!(plan.in_git && !plan.git_init);
        for (bad, why) in [
            (request("a/b", &root), "'/' cannot be in a folder name"),
            (
                request("x", Path::new("relative")),
                "give the folder's full path",
            ),
        ] {
            assert!(check(&bad).unwrap_err().starts_with(why), "{why}");
        }
        std::fs::create_dir_all(root.join("x/y")).unwrap();
        assert_eq!(
            check(&request("x", &root)).unwrap_err(),
            format!(
                "{} is not empty: y would be in the way",
                root.join("x").display()
            )
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_failed_init_says_what_was_done_and_removes_only_what_it_made() {
        let (_tmp, root) = tmp();
        let functor = fake_functor(&root, "echo 'error: no such template' >&2; exit 1");
        let parent = root.join("games");
        let failed = create_with(&functor, OsStr::new(NO_GIT), &request("A", &parent), "A")
            .await
            .err()
            .unwrap();
        assert_eq!(
            failed.reason,
            "functor init 3d failed: error: no such template"
        );
        let a = parent.join("A");
        assert_eq!(
            failed.done,
            [
                format!("Create {}", a.display()),
                format!("Removed the empty folder {} again", a.display()),
            ]
        );
        assert_eq!(failed.not_done[0], "Add the 3d template's files");
        // The parent it also made is gone; the folder it was given stays.
        assert!(!parent.exists() && root.exists());

        // An empty folder that was already there is kept.
        std::fs::create_dir(root.join("B")).unwrap();
        let failed = create_with(&functor, OsStr::new(NO_GIT), &request("B", &root), "B")
            .await
            .err()
            .unwrap();
        assert!(failed.done.is_empty(), "{:?}", failed.done);
        assert!(root.join("B").is_dir());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn without_git_the_project_is_made_with_a_note() {
        let (_tmp, root) = tmp();
        let functor = fake_functor(&root, "echo '{}' > \"$2/functor.json\"");
        let made = create_with(&functor, OsStr::new(NO_GIT), &request("C", &root), "C")
            .await
            .ok()
            .unwrap();
        assert_eq!(made.path, root.join("C").to_string_lossy());
        assert!(root.join("C/functor.json").is_file());
        assert!(made.note.unwrap().contains("review features are off"));
        // A folder that is not empty is refused before anything is written.
        let failed = create_with(&functor, OsStr::new(NO_GIT), &request("C", &root), "C")
            .await
            .err()
            .unwrap();
        assert!(failed.reason.ends_with("functor.json would be in the way"));
        assert!(failed.done.is_empty());
    }
}
