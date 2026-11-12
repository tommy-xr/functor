//! Project discovery: every directory under the repo root holding a
//! `functor.json`, with its entries read the way the `functor` CLI reads them.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::AppState;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Project {
    /// The directory's name (`hello`).
    pub name: String,
    /// Relative to the repo root, `/`-separated; `.` is the root itself.
    pub path: String,
    pub entries: Entries,
}

/// What `functor.json` declares to run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Entries {
    /// `"entry"`, `game.fun` when absent.
    Single { file: String },
    /// `"entries"`: named roles, sorted by name.
    Named { roles: Vec<Role> },
    /// A manifest `functor build` refuses.
    Invalid { error: String },
}

/// One `"entries"` role: `"client.fun"`, or `{ "file", "module" | "prefix" }`
/// for roles sharing a file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Role {
    pub name: String,
    pub file: String,
    #[cfg_attr(feature = "ts", ts(optional))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
    #[cfg_attr(feature = "ts", ts(optional))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
}

/// Reads a `functor.json`. Checks the entry layout only; the CLI's identifier
/// rules, `reverse` ports and input settings are left to `functor build`.
pub fn parse_manifest(text: &str) -> Entries {
    let invalid = |error: &str| Entries::Invalid {
        error: error.to_string(),
    };
    let Ok(json) = serde_json::from_str::<Value>(text) else {
        return invalid("functor.json is not valid JSON");
    };
    if json.get("language").and_then(Value::as_str) != Some("functor-lang") {
        return invalid(
            "not a Functor Lang project: functor.json needs \"language\": \"functor-lang\"",
        );
    }
    match (json.get("entry"), json.get("entries")) {
        (Some(_), Some(_)) => invalid("functor.json declares both `entry` and `entries`"),
        (None, Some(Value::Object(map))) if map.is_empty() => {
            invalid("functor.json `entries` is empty")
        }
        (None, Some(Value::Object(map))) => {
            match map
                .iter()
                .map(|(name, v)| role(name, v))
                .collect::<Result<Vec<_>, _>>()
            {
                Ok(mut roles) => {
                    // Already sorted unless serde_json's `preserve_order` gets enabled.
                    roles.sort_by(|a, b| a.name.cmp(&b.name));
                    Entries::Named { roles }
                }
                Err(error) => invalid(&error),
            }
        }
        (None, Some(_)) => invalid("functor.json `entries` must be a map"),
        // Like the CLI, a non-string `entry` falls back to the default.
        (entry, None) => Entries::Single {
            file: entry
                .and_then(Value::as_str)
                .unwrap_or("game.fun")
                .to_string(),
        },
    }
}

fn role(name: &str, value: &Value) -> Result<Role, String> {
    let err = |what: &str| format!("functor.json entry `{name}`: {what}");
    let map = match value {
        Value::String(file) if !file.is_empty() => {
            return Ok(Role {
                name: name.to_string(),
                file: file.clone(),
                module: None,
                prefix: None,
            })
        }
        Value::Object(map) => map,
        _ => return Err(err("must be a .fun path or { \"file\": … }")),
    };
    if let Some(key) = map
        .keys()
        .find(|k| !matches!(k.as_str(), "file" | "module" | "prefix" | "reverse"))
    {
        return Err(err(&format!("unknown key \"{key}\"")));
    }
    // Absent, null and "" all mean unset, as in the CLI.
    let text = |key: &str| match map.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone()).filter(|s| !s.is_empty())),
        Some(_) => Err(err(&format!("\"{key}\" must be a string"))),
    };
    let file = text("file")?.ok_or_else(|| err("the object form needs a \"file\""))?;
    let (module, prefix) = (text("module")?, text("prefix")?);
    if module.is_some() && prefix.is_some() {
        return Err(err("keep one of \"module\" and \"prefix\""));
    }
    Ok(Role {
        name: name.to_string(),
        file,
        module,
        prefix,
    })
}

/// `path` relative to `root`, `/`-separated; `.` for `root` itself.
pub fn relative(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(Path::new(""));
    let parts: Vec<_> = rel.iter().map(|c| c.to_string_lossy()).collect();
    if parts.is_empty() {
        ".".to_string()
    } else {
        parts.join("/")
    }
}

/// Projects from `(directory, functor.json read)` pairs under `root`, sorted by path.
pub fn projects_from_manifests(
    root: &Path,
    manifests: Vec<(PathBuf, io::Result<String>)>,
) -> Vec<Project> {
    let mut projects: Vec<Project> = manifests
        .into_iter()
        .map(|(dir, text)| Project {
            name: dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            path: relative(root, &dir),
            entries: match text {
                Ok(text) => parse_manifest(&text),
                Err(e) => Entries::Invalid {
                    error: format!("cannot read functor.json: {e}"),
                },
            },
        })
        .collect();
    projects.sort_by(|a, b| a.path.cmp(&b.path));
    projects
}

/// The project to open from `start` (both relative to the repo root): the
/// deepest one containing `start`; else the first directly under `start`;
/// else the first under `examples/`; else the first; else none. `paths` is
/// sorted.
pub fn default_project<'a>(paths: &[&'a str], start: &str) -> Option<&'a str> {
    let contains = |p: &str| p == "." || Path::new(start).starts_with(p);
    let depth = |p: &str| if p == "." { 0 } else { p.split('/').count() };
    let child_of_start =
        |p: &str| p != "." && p.rsplit_once('/').map_or(".", |(dir, _)| dir) == start;
    let first = |pred: &dyn Fn(&str) -> bool| paths.iter().copied().find(|p| pred(p));
    paths
        .iter()
        .copied()
        .filter(|p| contains(p))
        .max_by_key(|p| depth(p))
        .or_else(|| first(&child_of_start))
        .or_else(|| first(&|p| Path::new(p).starts_with("examples")))
        .or_else(|| paths.first().copied())
}

/// The names of a project directory's loadable `.fun` / `.funi` files and its
/// `functor.json`, sorted. Mirrors the loader: top level only (`file = module`
/// is not recursive), and a stem must be a module name (`game`, not `.#game`
/// or `2d`).
pub fn project_files(names: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut files: Vec<String> = names
        .into_iter()
        .filter(|n| {
            let stem = n.strip_suffix(".fun").or_else(|| n.strip_suffix(".funi"));
            n == "functor.json" || stem.is_some_and(functor_lang::project::is_module_stem)
        })
        .collect();
    files.sort();
    files
}

/// [`project_files`] over `dir`'s regular files; empty if it cannot be read.
pub fn read_project_files(dir: &Path) -> Vec<String> {
    let names = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned());
    project_files(names)
}

/// A project's directory from its `path`.
pub fn project_dir(root: &Path, path: &str) -> PathBuf {
    if path == "." {
        root.into()
    } else {
        root.join(path)
    }
}

/// The nearest ancestor of `start` (itself included) with a `.git` (a
/// directory, or a file in a worktree), else `start`.
pub fn repo_root(start: &Path) -> PathBuf {
    start
        .ancestors()
        .find(|dir| dir.join(".git").exists())
        .unwrap_or(start)
        .to_path_buf()
}

/// Every `functor.json` under `root`, skipping hidden and git-ignored paths
/// and `target`/`node_modules`/`dist` even when not ignored.
pub fn read_manifests(root: &Path) -> Vec<(PathBuf, io::Result<String>)> {
    ignore::WalkBuilder::new(root)
        // Only git's ignore rules apply: no `.ignore` files, and no whitelist
        // re-admitting a hidden or build directory.
        .ignore(false)
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            let hidden = e.depth() > 0 && name.starts_with('.');
            !hidden && !matches!(name.as_ref(), "target" | "node_modules" | "dist")
        })
        .build()
        .flatten()
        .filter(|e| e.file_name() == "functor.json" && e.file_type().is_some_and(|t| t.is_file()))
        .filter_map(|e| {
            Some((
                e.path().parent()?.to_path_buf(),
                std::fs::read_to_string(e.path()),
            ))
        })
        .collect()
}

/// The state for a start directory: its repo root, every project under it,
/// and the [`default_project`] active with its files listed. `start` should
/// be canonical.
pub fn discover(start: &Path) -> AppState {
    let root = repo_root(start);
    let projects = projects_from_manifests(&root, read_manifests(&root));
    let mut state = AppState::new(&root, start, projects);
    if let Some(path) = &state.active {
        state.files = read_project_files(&project_dir(&root, path));
    }
    state
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATHS: [&str; 3] = ["examples/hello", "examples/orbs", "tools/fixture"];

    #[test]
    fn default_is_the_start_dir_when_it_is_a_project() {
        assert_eq!(
            default_project(&PATHS, "examples/orbs"),
            Some("examples/orbs")
        );
    }

    #[test]
    fn default_is_the_closest_ancestor_project() {
        assert_eq!(
            default_project(&PATHS, "examples/orbs/assets/models"),
            Some("examples/orbs")
        );
        let nested = ["examples", "examples/orbs"];
        assert_eq!(
            default_project(&nested, "examples/orbs/x"),
            Some("examples/orbs")
        );
        // The repo root as a project contains every start.
        assert_eq!(default_project(&[".", "examples/hello"], "."), Some("."));
    }

    #[test]
    fn outside_a_project_a_child_of_start_comes_first() {
        let paths = [
            "cli/templates",
            "examples/hello",
            "games/a",
            "games/b",
            "tools",
        ];
        assert_eq!(default_project(&paths, "games"), Some("games/a"));
        assert_eq!(default_project(&paths, "."), Some("tools"));
        // A grandchild is not a child.
        assert_eq!(
            default_project(&["games/a/x", "games/b"], "games"),
            Some("games/b")
        );
    }

    #[test]
    fn then_a_project_under_examples() {
        // This repo from its root: not `cli/templates`, the first by path.
        let paths = ["cli/templates", "examples/animation", "examples/hello"];
        assert_eq!(default_project(&paths, "."), Some("examples/animation"));
        assert_eq!(default_project(&paths, "docs"), Some("examples/animation"));
        assert_eq!(
            default_project(&["a/x", "examples/deep/game"], "b"),
            Some("examples/deep/game")
        );
        // `examples-old` is not under `examples/`.
        assert_eq!(
            default_project(&["a/x", "examples-old/y"], "b"),
            Some("a/x")
        );
    }

    #[test]
    fn default_does_not_match_a_sibling_sharing_a_prefix() {
        assert_eq!(
            default_project(&["a/hello", "b"], "a/hello-world"),
            Some("a/hello")
        );
        assert_eq!(
            default_project(&["a", "b/hello"], "b/hello-world"),
            Some("a")
        );
    }

    #[test]
    fn default_with_no_projects_is_none() {
        assert_eq!(default_project(&[], "examples/hello"), None);
    }

    #[test]
    fn project_files_are_the_loadable_modules_and_manifest_sorted() {
        let names = [
            "notes.txt",
            "game.fun",
            "functor.json",
            "assets.fun",
            "net.funi",
            ".fun",
            "a.fun.bak",
            ".#game.fun",
            "2d.fun",
        ];
        assert_eq!(
            project_files(names.map(String::from)),
            ["assets.fun", "functor.json", "game.fun", "net.funi"]
        );
    }

    #[test]
    fn reads_project_files_from_the_top_level_only() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        for file in ["game.fun", "functor.json", "readme.md", "sub/x.fun"] {
            std::fs::create_dir_all(dir.join(file).parent().unwrap()).unwrap();
            std::fs::write(dir.join(file), "").unwrap();
        }
        std::fs::create_dir(dir.join("dir.fun")).unwrap();
        assert_eq!(read_project_files(dir), ["functor.json", "game.fun"]);
        assert!(read_project_files(&dir.join("missing")).is_empty());
    }

    #[test]
    fn manifests_parse_as_the_cli_reads_them() {
        let lang = r#""language": "functor-lang""#;
        let single = |file: &str| Entries::Single {
            file: file.to_string(),
        };
        assert_eq!(parse_manifest(&format!("{{{lang}}}")), single("game.fun"));
        assert_eq!(
            parse_manifest(&format!(r#"{{{lang}, "entry": "main.fun"}}"#)),
            single("main.fun")
        );
        let named = parse_manifest(&format!(
            r#"{{{lang}, "entries": {{"server": {{"file": "game.fun", "module": "Server"}}, "client": "client.fun"}}}}"#
        ));
        assert_eq!(
            named,
            Entries::Named {
                roles: vec![
                    Role {
                        name: "client".into(),
                        file: "client.fun".into(),
                        module: None,
                        prefix: None
                    },
                    Role {
                        name: "server".into(),
                        file: "game.fun".into(),
                        module: Some("Server".into()),
                        prefix: None
                    },
                ]
            }
        );
        for bad in [
            "{".to_string(),
            "{}".to_string(),
            format!(r#"{{{lang}, "entry": "a.fun", "entries": {{}}}}"#),
            format!(r#"{{{lang}, "entries": []}}"#),
            format!(r#"{{{lang}, "entries": {{}}}}"#),
            format!(r#"{{{lang}, "entries": {{"client": 3}}}}"#),
            format!(
                r#"{{{lang}, "entries": {{"s": {{"file": "g.fun", "module": "S", "prefix": "s"}}}}}}"#
            ),
            format!(r#"{{{lang}, "entries": {{"s": {{"file": "g.fun", "module": 3}}}}}}"#),
            format!(r#"{{{lang}, "entries": {{"s": {{"file": "g.fun", "modul": "S"}}}}}}"#),
            format!(r#"{{{lang}, "entries": {{"s": {{"module": "S"}}}}}}"#),
        ] {
            assert!(
                matches!(parse_manifest(&bad), Entries::Invalid { .. }),
                "{bad}"
            );
        }
    }

    #[test]
    fn discovers_projects_in_a_tree() {
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        let game = r#"{"language": "functor-lang"}"#;
        for dir in [
            "games/a",
            "games/a/nested",
            "games/b",
            "target/x",
            "games/a/node_modules/x",
            "dist",
            ".hidden/x",
            "build/x",
        ] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
            std::fs::write(root.join(dir).join("functor.json"), game).unwrap();
        }
        std::fs::create_dir_all(root.join("games/a/src")).unwrap();
        std::fs::create_dir(root.join(".git")).unwrap();
        std::fs::write(root.join(".gitignore"), "build/\n").unwrap();
        // Only git's rules apply: `.ignore` files do not hide projects.
        std::fs::write(root.join(".ignore"), "games/b/\n").unwrap();

        let state = discover(&root.join("games/a/src"));
        let paths: Vec<&str> = state.projects.iter().map(|p| p.path.as_str()).collect();
        assert_eq!(paths, ["games/a", "games/a/nested", "games/b"]);
        assert_eq!(state.repo_root, root.to_string_lossy());
        assert_eq!(state.active.as_deref(), Some("games/a"));
        assert_eq!(state.projects[2].name, "b");
        assert_eq!(state.files, ["functor.json"]);

        // From the root, the first project in path order is active.
        assert_eq!(discover(&root).active.as_deref(), Some("games/a"));
    }

    #[test]
    fn without_git_the_start_dir_is_the_root() {
        let tmp = tempfile::tempdir().unwrap();
        let start = std::fs::canonicalize(tmp.path()).unwrap();
        assert_eq!(repo_root(&start), start);
        let state = discover(&start);
        assert_eq!(state.repo_root, start.to_string_lossy());
        assert!(state.projects.is_empty());
        assert_eq!(state.active, None);

        std::fs::write(
            start.join("functor.json"),
            r#"{"language": "functor-lang"}"#,
        )
        .unwrap();
        let state = discover(&start);
        assert_eq!(state.projects.len(), 1);
        assert_eq!(state.projects[0].path, ".");
        assert_eq!(state.active.as_deref(), Some("."));
    }
}
