//! Project discovery: every directory under the repo root holding a
//! `functor.json`, with its entries read the way the `functor` CLI reads them.

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
    /// A manifest the CLI refuses to run.
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

/// Reads a `functor.json`. Covers the entry layout only; the CLI's deeper
/// checks (identifier rules, `reverse` ports, input settings) stay in `functor build`.
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
    let text = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let file = match value {
        Value::String(file) if !file.is_empty() => Some(file.clone()),
        Value::Object(_) => text("file"),
        _ => None,
    };
    let file = file.ok_or_else(|| {
        format!("functor.json entry `{name}` must be a .fun path or {{ \"file\": … }}")
    })?;
    Ok(Role {
        name: name.to_string(),
        file,
        module: text("module"),
        prefix: text("prefix"),
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

/// Projects from `(directory, functor.json text)` pairs under `root`, sorted by path.
pub fn projects_from_manifests(root: &Path, manifests: Vec<(PathBuf, String)>) -> Vec<Project> {
    let mut projects: Vec<Project> = manifests
        .into_iter()
        .map(|(dir, text)| Project {
            name: dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            path: relative(root, &dir),
            entries: parse_manifest(&text),
        })
        .collect();
    projects.sort_by(|a, b| a.path.cmp(&b.path));
    projects
}

/// The project nearest `start` (both relative to the repo root): the deepest
/// one containing it, else the first, else none.
pub fn nearest_project<'a>(paths: &[&'a str], start: &str) -> Option<&'a str> {
    let contains = |p: &str| p == "." || Path::new(start).starts_with(p);
    let depth = |p: &str| if p == "." { 0 } else { p.split('/').count() };
    paths
        .iter()
        .copied()
        .filter(|p| contains(p))
        .max_by_key(|p| depth(p))
        .or_else(|| paths.first().copied())
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

/// Every `functor.json` under `root` with its text, skipping hidden and
/// git-ignored paths and `target`/`node_modules`/`dist` even when not ignored.
pub fn read_manifests(root: &Path) -> Vec<(PathBuf, String)> {
    ignore::WalkBuilder::new(root)
        .filter_entry(|e| {
            !matches!(
                e.file_name().to_str(),
                Some("target" | "node_modules" | "dist")
            )
        })
        .build()
        .flatten()
        .filter(|e| e.file_name() == "functor.json" && e.file_type().is_some_and(|t| t.is_file()))
        .filter_map(|e| {
            let dir = e.path().parent()?.to_path_buf();
            // Unreadable reads as empty, which reports as invalid JSON.
            let text = std::fs::read_to_string(e.path()).unwrap_or_default();
            Some((dir, text))
        })
        .collect()
}

/// The state for a start directory: its repo root, every project under it,
/// and the nearest one active. `start` should be canonical.
pub fn discover(start: &Path) -> AppState {
    let root = repo_root(start);
    let projects = projects_from_manifests(&root, read_manifests(&root));
    AppState::new(&root, start, projects)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATHS: [&str; 3] = ["examples/hello", "examples/orbs", "tools/fixture"];

    #[test]
    fn nearest_is_the_start_dir_when_it_is_a_project() {
        assert_eq!(
            nearest_project(&PATHS, "examples/orbs"),
            Some("examples/orbs")
        );
    }

    #[test]
    fn nearest_is_the_closest_ancestor_project() {
        assert_eq!(
            nearest_project(&PATHS, "examples/orbs/assets/models"),
            Some("examples/orbs")
        );
        let nested = ["examples", "examples/orbs"];
        assert_eq!(
            nearest_project(&nested, "examples/orbs/x"),
            Some("examples/orbs")
        );
    }

    #[test]
    fn nearest_from_the_repo_root_is_the_first_project() {
        assert_eq!(nearest_project(&PATHS, "."), Some("examples/hello"));
        // Unless the root is itself a project.
        assert_eq!(nearest_project(&[".", "examples/hello"], "."), Some("."));
    }

    #[test]
    fn nearest_does_not_match_a_sibling_sharing_a_prefix() {
        assert_eq!(
            nearest_project(&["examples/hello", "tools"], "examples/hello-world"),
            Some("examples/hello")
        );
        assert_eq!(
            nearest_project(&["a/hello"], "a/hello-world"),
            Some("a/hello")
        );
        assert_eq!(
            nearest_project(&["a", "b/hello"], "b/hello-world"),
            Some("a")
        );
    }

    #[test]
    fn nearest_with_no_projects_is_none() {
        assert_eq!(nearest_project(&[], "examples/hello"), None);
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

        let state = discover(&root.join("games/a/src"));
        let paths: Vec<&str> = state.projects.iter().map(|p| p.path.as_str()).collect();
        assert_eq!(paths, ["games/a", "games/a/nested", "games/b"]);
        assert_eq!(state.repo_root, root.to_string_lossy());
        assert_eq!(state.active.as_deref(), Some("games/a"));
        assert_eq!(state.projects[2].name, "b");

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
