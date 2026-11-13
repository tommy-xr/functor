//! Path evidence (D38): the game's predicted path from the moment a task
//! started, under the task's baseline code and under its current code, with
//! the same recorded inputs. The shell takes the two captures; when to take
//! them, and when to show nothing, is decided here.

use serde::{Deserialize, Serialize};

use crate::changes::Change;
use crate::projects::{project_files, project_prefix};
use crate::tasks::Task;

/// The game as the task's baseline was taken: where both paths start.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Moment {
    /// The frame the game showed, and the recorded input steps (from its
    /// start) both paths replay.
    Taken { frame: u64, steps: u64 },
    /// Why there is none.
    Missing { reason: String },
}

/// A task's path evidence, for the change measured as `tree`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Evidence {
    /// The change's snapshot the new path runs (`Change::tree`).
    pub tree: String,
    pub state: EvidenceState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EvidenceState {
    /// The shell is taking it.
    Taking,
    /// The moment's frame with the path drawn, before and after the change:
    /// files the app serves at `/evidence/<name>`.
    Shown { before: String, after: String },
    /// Both paths are the same: the change moves nothing.
    Same,
    /// Not shown, and why.
    Withheld { reason: String },
}

impl Evidence {
    /// The files it names.
    pub fn files(&self) -> Vec<String> {
        match &self.state {
            EvidenceState::Shown { before, after } => vec![before.clone(), after.clone()],
            _ => vec![],
        }
    }
}

/// What a task's evidence should become now.
#[derive(Debug, PartialEq, Eq)]
pub enum Next {
    /// It is current.
    Keep,
    /// It becomes this; with `take`, the shell is asked for the captures.
    Set {
        evidence: Option<Evidence>,
        take: bool,
    },
}

/// What `task`'s evidence should become: none without a code change or a
/// moment, the moment's reason when it has none, else taken once per
/// measured change. Only between runs: a run's edits are not final.
pub fn next(task: &Task) -> Next {
    if task.run.is_some() || task.reverting {
        return Next::Keep;
    }
    let change = task
        .change
        .as_ref()
        .filter(|c| changes_code(c, &task.project));
    let (Some(change), Some(moment)) = (change, &task.moment) else {
        return match task.evidence {
            Some(_) => Next::Set {
                evidence: None,
                take: false,
            },
            None => Next::Keep,
        };
    };
    if task
        .evidence
        .as_ref()
        .is_some_and(|e| e.tree == change.tree)
    {
        return Next::Keep;
    }
    let (state, take) = match moment {
        Moment::Taken { .. } => (EvidenceState::Taking, true),
        Moment::Missing { reason } => (
            EvidenceState::Withheld {
                reason: reason.clone(),
            },
            false,
        ),
    };
    Next::Set {
        evidence: Some(Evidence {
            tree: change.tree.clone(),
            state,
        }),
        take,
    }
}

/// Whether `change` touches the game's code: a file the loader reads
/// ([`project_files`]: top-level `.fun`, `.funi`, `functor.json`) in project
/// `project` (repo-relative, `.` the root). Files past the summary's list
/// may be code.
pub fn changes_code(change: &Change, project: &str) -> bool {
    let prefix = project_prefix(project);
    let code = |path: &String| {
        path.strip_prefix(&prefix)
            .is_some_and(|rest| !project_files([rest.to_string()]).is_empty())
    };
    change.more > 0
        || change
            .files
            .iter()
            .any(|f| code(&f.path) || f.from.as_ref().is_some_and(code))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changes::FileChange;

    fn change(paths: &[&str], tree: &str) -> Change {
        let files = paths
            .iter()
            .map(|p| FileChange {
                path: p.to_string(),
                from: None,
                added: 1,
                removed: 1,
                binary: false,
                large: false,
            })
            .collect();
        Change::from_files(files, tree.to_string())
    }

    fn task(change: Option<Change>, moment: Option<Moment>) -> Task {
        let mut task: Task = serde_json::from_value(serde_json::json!({
            "id": 1, "project": "game", "title": "t", "status": {"kind": "ready_for_review"},
            "harness": "claude", "model": null, "session_id": null, "run": null,
        }))
        .unwrap();
        task.change = change;
        task.moment = moment;
        task
    }

    const TAKEN: Moment = Moment::Taken {
        frame: 12,
        steps: 40,
    };

    #[test]
    fn code_is_the_projects_fun_files_and_manifest() {
        assert!(changes_code(&change(&["game/game.fun"], "t"), "game"));
        assert!(changes_code(&change(&["game/functor.json"], "t"), "game"));
        assert!(changes_code(&change(&["game.fun"], "t"), "."));
        assert!(changes_code(&change(&["game/game.funi"], "t"), "game"));
        assert!(!changes_code(&change(&["game/sub/x.fun"], "t"), "game"));
        assert!(!changes_code(&change(&["game/hero.png"], "t"), "game"));
        assert!(!changes_code(&change(&["other/game.fun"], "t"), "game"));
        assert!(!changes_code(&change(&["README.md"], "t"), "."));
        let mut renamed = change(&["game/notes.txt"], "t");
        renamed.files[0].from = Some("game/old.fun".into());
        assert!(changes_code(&renamed, "game"));
    }

    #[test]
    fn a_code_change_with_a_moment_is_taken_once_per_tree() {
        let t = task(Some(change(&["game/game.fun"], "tree-2")), Some(TAKEN));
        let taking = Evidence {
            tree: "tree-2".into(),
            state: EvidenceState::Taking,
        };
        assert_eq!(
            next(&t),
            Next::Set {
                evidence: Some(taking.clone()),
                take: true
            }
        );
        let mut t = t;
        t.evidence = Some(taking);
        assert_eq!(next(&t), Next::Keep);
        // A new measurement is taken again.
        t.change = Some(change(&["game/game.fun"], "tree-3"));
        assert!(matches!(next(&t), Next::Set { take: true, .. }));
        // Not while a run edits.
        t.run = Some(crate::tasks::Run {
            id: 1,
            usage: None,
            views: 0,
        });
        assert_eq!(next(&t), Next::Keep);
    }

    #[test]
    fn nothing_runs_without_a_code_change_or_a_moment() {
        assert_eq!(
            next(&task(Some(change(&["game/hero.png"], "t")), Some(TAKEN))),
            Next::Keep
        );
        assert_eq!(next(&task(None, Some(TAKEN))), Next::Keep);
        assert_eq!(
            next(&task(Some(change(&["game/game.fun"], "t")), None)),
            Next::Keep
        );
        // Evidence for a change that is no longer code goes.
        let mut t = task(Some(change(&["game/hero.png"], "t")), Some(TAKEN));
        t.evidence = Some(Evidence {
            tree: "old".into(),
            state: EvidenceState::Same,
        });
        assert_eq!(
            next(&t),
            Next::Set {
                evidence: None,
                take: false
            }
        );
    }

    #[test]
    fn a_missing_moment_records_why() {
        let missing = Moment::Missing {
            reason: "the game was not running when the task started".into(),
        };
        let t = task(Some(change(&["game/game.fun"], "t")), Some(missing));
        assert_eq!(
            next(&t),
            Next::Set {
                evidence: Some(Evidence {
                    tree: "t".into(),
                    state: EvidenceState::Withheld {
                        reason: "the game was not running when the task started".into()
                    }
                }),
                take: false
            }
        );
    }
}
