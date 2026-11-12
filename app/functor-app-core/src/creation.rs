//! New projects (D24): the templates `functor init --list --json` names, a
//! project's name as a folder name, what may stand where it goes, and the
//! plan the modal shows. The shell checks the disk and creates; this decides.

use serde::{Deserialize, Serialize};

use crate::tasks::Harness;

/// Longest folder name a project gets.
const MAX_NAME: usize = 64;
/// Most names said to be in the way of a new project.
const SHOWN_IN_THE_WAY: usize = 5;

/// One of `functor init --list --json`'s entries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Template {
    /// What `functor init <name>` takes.
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub kind: TemplateKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum TemplateKind {
    Starter,
    Example,
}

/// The templates in `--list --json`'s output, starters first (otherwise in
/// its order). Fields this app does not know (a preview) are ignored.
pub fn parse_templates(json: &str) -> Result<Vec<Template>, String> {
    let mut list: Vec<Template> =
        serde_json::from_str(json).map_err(|e| format!("cannot read the template list: {e}"))?;
    if list.is_empty() {
        return Err("the template list is empty".to_string());
    }
    list.sort_by_key(|t| t.kind);
    Ok(list)
}

/// What the modal asks for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct NewProjectRequest {
    pub name: String,
    /// Where the project's folder goes: absolute, or `~/…`.
    pub parent: String,
    pub template: String,
    /// Run `git init` when the folder is not in a git work tree already.
    pub git: bool,
    /// The first task's message; blank for none.
    pub describe: String,
}

/// What stands where a new project's folder goes, as the shell found it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Absent,
    Empty,
    /// A directory holding these names, sorted.
    Taken(Vec<String>),
    /// A file, or something else that is not a directory.
    NotAFolder,
    Unreadable(String),
}

/// What creating the project will do, for the modal and for the shell.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Plan {
    /// The project's folder.
    pub path: String,
    /// It is inside a git work tree already (no `git init` is offered).
    pub in_git: bool,
    /// `git init` runs after the template's files are written.
    pub git_init: bool,
    /// The steps, in order, as the modal lists them.
    pub steps: Vec<String>,
}

/// The folder a project named `name` gets: trimmed, each run of spaces one
/// `-`; letters, digits, `-`, `_` and `.`, not leading with `.`.
pub fn folder_name(name: &str) -> Result<String, String> {
    let folder = name.split_whitespace().collect::<Vec<_>>().join("-");
    if folder.is_empty() {
        return Err("give the project a name".to_string());
    }
    if let Some(c) = folder
        .chars()
        .find(|c| !(c.is_alphanumeric() || matches!(c, '-' | '_' | '.')))
    {
        return Err(format!(
            "{c:?} cannot be in a folder name: use letters, digits, spaces, - _ and ."
        ));
    }
    if folder.starts_with('.') {
        return Err("a name cannot start with . (its folder would be hidden)".to_string());
    }
    if folder.chars().count() > MAX_NAME {
        return Err(format!("keep the name to {MAX_NAME} characters"));
    }
    Ok(folder)
}

/// Why a project cannot go at `path`, given what stands there: only an
/// absent folder or an empty one will do.
pub fn check_target(path: &str, target: &Target) -> Result<(), String> {
    match target {
        Target::Absent | Target::Empty => Ok(()),
        Target::NotAFolder => Err(format!("{path} exists and is not a folder")),
        Target::Unreadable(why) => Err(format!("{path} cannot be read: {why}")),
        Target::Taken(names) => {
            let shown = names[..names.len().min(SHOWN_IN_THE_WAY)].join(", ");
            let more = match names.len().saturating_sub(SHOWN_IN_THE_WAY) {
                0 => String::new(),
                n => format!(" and {n} more"),
            };
            Err(format!(
                "{path} is not empty: {shown}{more} would be in the way"
            ))
        }
    }
}

/// The plan for `request` with its folder at `path`, where `target` stands
/// and which is (or is not) `in_git`; or why it cannot be made.
pub fn plan(
    request: &NewProjectRequest,
    path: &str,
    target: &Target,
    in_git: bool,
) -> Result<Plan, String> {
    check_target(path, target)?;
    let git_init = request.git && !in_git;
    let mut steps = vec![
        match target {
            Target::Empty => format!("Use the empty folder {path}"),
            _ => format!("Create {path}"),
        },
        format!("Add the {} template's files", request.template),
    ];
    if git_init {
        steps.push("Track changes with git (git init)".to_string());
    }
    steps.push("Open it and start the game".to_string());
    if !request.describe.trim().is_empty() {
        steps.push("Start a task with your description".to_string());
    }
    Ok(Plan {
        path: path.to_string(),
        in_git,
        git_init,
        steps,
    })
}

/// The new-project modal, while it is open.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct NewProject {
    /// The name and parent folder the modal starts with ("New project here"
    /// fills both; a picked folder replaces `parent`).
    pub name: String,
    pub parent: String,
    pub step: Step,
}

/// Where creating a project is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Step {
    Editing,
    /// The shell is creating `path` from the template.
    Creating {
        path: String,
        /// The first task, started once it is open.
        #[serde(skip)]
        #[cfg_attr(feature = "ts", ts(skip))]
        task: Option<(String, Harness)>,
    },
    /// Created; the app is opening it as open request `id`.
    Opening {
        path: String,
        #[serde(skip)]
        #[cfg_attr(feature = "ts", ts(skip))]
        id: u64,
        #[serde(skip)]
        #[cfg_attr(feature = "ts", ts(skip))]
        task: Option<(String, Harness)>,
    },
    /// Stopped at `reason`, with what was and was not done.
    Failed {
        reason: String,
        done: Vec<String>,
        not_done: Vec<String>,
    },
}

impl Step {
    /// Creating or opening: the modal stays until it ends.
    pub fn busy(&self) -> bool {
        matches!(self, Step::Creating { .. } | Step::Opening { .. })
    }
}

/// Why creating a project stopped, with what was and was not done.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateFailure {
    pub reason: String,
    pub done: Vec<String>,
    pub not_done: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_parse_with_starters_first() {
        let json = r#"[
            {"name":"tetris","description":"d","tags":["2d"],"kind":"example"},
            {"name":"3d","description":"d","tags":["3d"],"kind":"starter","preview":"p.png"},
            {"name":"blank","description":"d","tags":[],"kind":"starter"}
        ]"#;
        let names: Vec<String> = parse_templates(json)
            .unwrap()
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(names, ["3d", "blank", "tetris"]);
        for bad in ["", "[]", "{}", r#"[{"name":"x"}]"#] {
            assert!(parse_templates(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn names_become_folder_names() {
        assert_eq!(folder_name("  My   Game "), Ok("My-Game".into()));
        assert_eq!(folder_name("space_race.2"), Ok("space_race.2".into()));
        assert_eq!(folder_name("Jeu été"), Ok("Jeu-été".into()));
        let long = "a".repeat(MAX_NAME + 1);
        for (bad, why) in [
            ("", "give the project a name"),
            ("   ", "give the project a name"),
            ("a/b", "'/' cannot be in a folder name"),
            ("..", "a name cannot start with ."),
            (".hidden", "a name cannot start with ."),
            ("a\\b", "'\\\\' cannot be in a folder name"),
            ("what?", "'?' cannot be in a folder name"),
            (&long, "keep the name to 64 characters"),
        ] {
            let err = folder_name(bad).unwrap_err();
            assert!(err.starts_with(why), "{bad}: {err}");
        }
    }

    #[test]
    fn only_an_absent_or_empty_folder_takes_a_project() {
        assert_eq!(check_target("/p/a", &Target::Absent), Ok(()));
        assert_eq!(check_target("/p/a", &Target::Empty), Ok(()));
        assert_eq!(
            check_target("/p/a", &Target::NotAFolder),
            Err("/p/a exists and is not a folder".into())
        );
        let taken = |n: usize| Target::Taken((0..n).map(|i| format!("f{i}")).collect());
        assert_eq!(
            check_target("/p/a", &taken(2)),
            Err("/p/a is not empty: f0, f1 would be in the way".into())
        );
        assert_eq!(
            check_target("/p/a", &taken(7)),
            Err("/p/a is not empty: f0, f1, f2, f3, f4 and 2 more would be in the way".into())
        );
        assert_eq!(
            check_target("/p/a", &Target::Unreadable("denied".into())),
            Err("/p/a cannot be read: denied".into())
        );
    }

    fn request(git: bool, describe: &str) -> NewProjectRequest {
        NewProjectRequest {
            name: "Space".into(),
            parent: "/p".into(),
            template: "3d".into(),
            git,
            describe: describe.into(),
        }
    }

    #[test]
    fn the_plan_says_what_will_happen() {
        let plan_of = |req, target, in_git| plan(&req, "/p/Space", &target, in_git).unwrap();
        let full = plan_of(request(true, "a racing game"), Target::Absent, false);
        assert!(full.git_init && !full.in_git);
        assert_eq!(
            full.steps,
            [
                "Create /p/Space",
                "Add the 3d template's files",
                "Track changes with git (git init)",
                "Open it and start the game",
                "Start a task with your description",
            ]
        );
        // Inside a work tree, unchecked, or no description: fewer steps.
        let inside = plan_of(request(true, " "), Target::Empty, true);
        assert!(!inside.git_init && inside.in_git);
        assert_eq!(
            inside.steps,
            [
                "Use the empty folder /p/Space",
                "Add the 3d template's files",
                "Open it and start the game",
            ]
        );
        assert!(!plan_of(request(false, ""), Target::Absent, false).git_init);
        assert!(plan(&request(true, ""), "/p/Space", &Target::NotAFolder, false).is_err());
    }
}
