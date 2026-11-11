//! The `--print` text dump of the state.

use crate::projects::Entries;
use crate::AppState;

/// One line per field, then one per project in path order, the active one
/// marked `*`:
///
/// ```text
/// repo: /src/functor
/// active: examples/hello
/// projects: 2
/// * examples/hello (hello): entry game.fun
///   examples/orbs (orbs): entries client=game.fun module=Client, server=game.fun module=Server
/// ```
pub fn print(state: &AppState) -> String {
    let active = state.active.as_deref();
    let mut lines = vec![
        format!("repo: {}", state.repo_root),
        format!("active: {}", active.unwrap_or("none")),
        format!("projects: {}", state.projects.len()),
    ];
    for p in &state.projects {
        let mark = if Some(p.path.as_str()) == active {
            '*'
        } else {
            ' '
        };
        let entries = match &p.entries {
            Entries::Single { file } => format!("entry {file}"),
            Entries::Named { roles } => {
                let roles: Vec<String> = roles
                    .iter()
                    .map(|r| {
                        let module = r.module.as_ref().map(|m| format!(" module={m}"));
                        let prefix = r.prefix.as_ref().map(|p| format!(" prefix={p}"));
                        format!(
                            "{}={}{}{}",
                            r.name,
                            r.file,
                            module.unwrap_or_default(),
                            prefix.unwrap_or_default()
                        )
                    })
                    .collect();
                format!("entries {}", roles.join(", "))
            }
            Entries::Invalid { error } => format!("invalid: {error}"),
        };
        lines.push(format!("{mark} {} ({}): {entries}", p.path, p.name));
    }
    lines.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::{Project, Role};

    #[test]
    fn prints_projects_with_the_active_one_marked() {
        let state = AppState {
            repo_root: "/src/functor".to_string(),
            projects: vec![
                Project {
                    name: "hello".into(),
                    path: "examples/hello".into(),
                    entries: Entries::Single {
                        file: "game.fun".into(),
                    },
                },
                Project {
                    name: "orbs".into(),
                    path: "examples/orbs".into(),
                    entries: Entries::Named {
                        roles: vec![
                            Role {
                                name: "client".into(),
                                file: "game.fun".into(),
                                module: Some("Client".into()),
                                prefix: None,
                            },
                            Role {
                                name: "server".into(),
                                file: "server.fun".into(),
                                module: None,
                                prefix: None,
                            },
                        ],
                    },
                },
                Project {
                    name: "old".into(),
                    path: "old".into(),
                    entries: Entries::Invalid {
                        error: "bad".into(),
                    },
                },
            ],
            active: Some("examples/hello".into()),
        };
        assert_eq!(
            print(&state),
            "\
repo: /src/functor
active: examples/hello
projects: 3
* examples/hello (hello): entry game.fun
  examples/orbs (orbs): entries client=game.fun module=Client, server=server.fun
  old (old): invalid: bad
"
        );
    }

    #[test]
    fn prints_none_without_projects() {
        let state = AppState {
            repo_root: "/tmp/empty".into(),
            projects: vec![],
            active: None,
        };
        assert_eq!(
            print(&state),
            "repo: /tmp/empty\nactive: none\nprojects: 0\n"
        );
    }
}
