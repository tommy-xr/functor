//! The task queue (D30): one run per project at a time. A task made while
//! its project is busy waits its turn, in order, and starts by itself when
//! the project is free, given every message the user sent it meanwhile.
//!
//! A change waiting for Keep or Revert holds the queue too. A change is
//! measured from its task's snapshot of the whole working tree, so a second
//! task's edits on top would show in the first one's change, and reverting
//! the first would undo them as well: the two cannot be told apart. So the
//! queue waits for the decision, and says so; only the user starts a task
//! beside such a change.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::tasks::{has_change, Item, Spoken, Task};
use crate::voice;

/// Why a project's next task cannot start now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Wait {
    /// Task `task` is running.
    Running { task: u64 },
    /// Task `task`'s change is being put back (a revert writes anywhere in
    /// the repo, so it holds every project).
    Reverting { task: u64 },
    /// Task `task`'s change waits for the user's Keep or Revert.
    Decision { task: u64 },
    /// Task `task`'s change is not measured yet (opening its changes measures it).
    Measuring { task: u64 },
}

impl Wait {
    /// Why a task cannot start, as a refusal says it.
    pub fn reason(&self) -> String {
        match self {
            Wait::Running { task } => {
                format!("task {task} is running: one run per project at a time")
            }
            Wait::Reverting { task } => format!("task {task}'s change is being reverted"),
            Wait::Decision { task } => {
                format!("task {task}'s change waits for you: keep or revert it first")
            }
            Wait::Measuring { task } => {
                format!("task {task}'s change is not measured yet: open its changes")
            }
        }
    }
}

/// A project's queue, derived after every event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Queue {
    /// Its queued tasks, the next to start first.
    pub tasks: Vec<u64>,
    /// Why a task cannot start in the project now; `None`: it would start at once.
    pub wait: Option<Wait>,
    /// A task the user starts now would queue ([`holds`]).
    pub holds: bool,
}

/// Why a task cannot start in `project` now (`tracks`: changes are
/// tracked). A change waiting anywhere in the repo holds it: snapshots
/// cover the whole working tree, whatever the project.
pub fn wait(tasks: &[Task], tracks: bool, project: &str) -> Option<Wait> {
    if let Some(t) = (tasks.iter()).find(|t| t.project == project && t.run.is_some()) {
        return Some(Wait::Running { task: t.id });
    }
    if let Some(t) = tasks.iter().find(|t| t.reverting) {
        return Some(Wait::Reverting { task: t.id });
    }
    if !tracks {
        return None;
    }
    tasks.iter().find_map(|t| match (has_change(t), &t.baseline, &t.change) {
        (true, _, _) => Some(Wait::Decision { task: t.id }),
        (false, Some(_), None) => Some(Wait::Measuring { task: t.id }),
        _ => None,
    })
}

/// Why a task the user starts now in `project` waits: a run or a revert,
/// or tasks already in line (it goes behind them). A change waiting for a
/// decision holds only the queue's own starts: beside it, the user may
/// start one themselves.
pub fn holds(tasks: &[Task], tracks: bool, project: &str) -> Option<Wait> {
    match wait(tasks, tracks, project) {
        Some(w @ (Wait::Running { .. } | Wait::Reverting { .. })) => Some(w),
        Some(w) if !order(tasks, project).is_empty() => Some(w),
        _ => None,
    }
}

/// `project`'s queued tasks, the next to start first.
pub fn order(tasks: &[Task], project: &str) -> Vec<u64> {
    let mut queued: Vec<(u64, u64)> = tasks
        .iter()
        .filter(|t| t.project == project)
        .filter_map(|t| t.queued.map(|key| (key, t.id)))
        .collect();
    queued.sort();
    queued.into_iter().map(|(_, id)| id).collect()
}

/// The queue of each of `projects` (and every task's project) that has
/// queued tasks or a wait.
pub fn queues<'a>(
    tasks: &'a [Task],
    tracks: bool,
    projects: impl IntoIterator<Item = &'a str>,
) -> BTreeMap<String, Queue> {
    let mut out = BTreeMap::new();
    let all = projects.into_iter().chain(tasks.iter().map(|t| t.project.as_str()));
    for project in all {
        if out.contains_key(project) {
            continue;
        }
        let queue = Queue {
            tasks: order(tasks, project),
            wait: wait(tasks, tracks, project),
            holds: holds(tasks, tracks, project).is_some(),
        };
        if !queue.tasks.is_empty() || queue.wait.is_some() {
            out.insert(project.to_string(), queue);
        }
    }
    out
}

/// The tasks that can start now: each free project's first queued one.
pub fn ready(tasks: &[Task], tracks: bool) -> Vec<u64> {
    queues(tasks, tracks, [])
        .values()
        .filter(|q| q.wait.is_none())
        .filter_map(|q| q.tasks.first().copied())
        .collect()
}

/// The place in line a newly queued task takes: after every other.
pub fn last_place(tasks: &[Task]) -> u64 {
    tasks.iter().filter_map(|t| t.queued).max().unwrap_or(0) + 1
}

/// Moves queued task `id` to index `to` of its project's queue (the end
/// when past it); the tasks whose places changed, or why it cannot move.
pub fn reorder(tasks: &mut [Task], id: u64, to: usize) -> Result<Vec<u64>, String> {
    let Some(task) = tasks.iter().find(|t| t.id == id) else {
        return Err(format!("there is no task {id}"));
    };
    if task.queued.is_none() {
        return Err(format!("task {id} is not queued"));
    }
    let mut line = order(tasks, &task.project);
    line.retain(|&t| t != id);
    line.insert(to.min(line.len()), id);
    // Places are renumbered from the lowest the project held, so other
    // projects' places keep their meaning.
    let first = tasks
        .iter()
        .filter(|t| line.contains(&t.id))
        .filter_map(|t| t.queued)
        .min()
        .unwrap_or(1);
    let mut changed = Vec::new();
    for (i, queued) in line.into_iter().enumerate() {
        let task = tasks.iter_mut().find(|t| t.id == queued).expect("in line");
        let place = Some(first + i as u64);
        if task.queued != place {
            task.queued = place;
            changed.push(queued);
        }
    }
    Ok(changed)
}

/// Said to the agent before messages it gets after the first in one run.
pub const LATER: &str = "The user added this later, in order. Where a later message differs \
from an earlier one, the later one wins; if one drops the request, change nothing.";

/// What the agent is sent as `task`'s run starts: the messages it has not
/// seen, in order. With no session to resume that is every one (D30: the
/// request and all said since, so a change of mind is honoured); else
/// those after its last run (a run cut before its agent started leaves no
/// `Stopped`, so its messages are sent again).
pub fn prompt(task: &Task) -> String {
    fn user(i: &Item) -> Option<(&String, &Option<Spoken>)> {
        match i {
            Item::User { text, spoken } => Some((text, spoken)),
            _ => None,
        }
    }
    let messages: Vec<(&String, &Option<Spoken>)> = match task.session_id {
        None => task.transcript.iter().filter_map(user).collect(),
        Some(_) => {
            let mut trailing: Vec<_> = task.transcript.iter().rev().map_while(user).collect();
            trailing.reverse();
            trailing
        }
    };
    let said = |(text, spoken): &(&String, &Option<Spoken>)| match spoken {
        Some(s) => voice::prompt(text, s),
        None => text.to_string(),
    };
    match &messages[..] {
        [] => String::new(),
        [one] => said(one),
        [first, rest @ ..] => {
            let rest: Vec<String> = rest.iter().map(said).collect();
            format!("{}\n\n{LATER}\n\n{}", said(first), rest.join("\n\n"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changes::{Change, FileChange};
    use crate::tasks::{Harness, Run, TaskStatus};

    fn task(id: u64, project: &str) -> Task {
        Task {
            id,
            project: project.to_string(),
            title: format!("t{id}"),
            status: TaskStatus::Done,
            harness: Harness::Claude,
            scope: Default::default(),
            model: None,
            session_id: None,
            run: None,
            queued: None,
            baseline: None,
            change: None,
            reverting: false,
            moment: None,
            evidence: None,
            report: None,
            review: None,
            created_at: None,
            updated_at: None,
            transcript: vec![],
        }
    }

    fn queued(id: u64, project: &str, place: u64) -> Task {
        Task {
            queued: Some(place),
            ..task(id, project)
        }
    }

    fn running(id: u64, project: &str) -> Task {
        Task {
            run: Some(Run::default()),
            ..task(id, project)
        }
    }

    fn with_change(mut t: Task, files: usize) -> Task {
        t.baseline = Some("t0".to_string());
        let file = |i| FileChange {
            path: format!("f{i}"),
            from: None,
            added: 1,
            removed: 0,
            binary: false,
            large: false,
        };
        t.change = Some(Change::from_files((0..files).map(file).collect(), "t1".into()));
        t
    }

    fn user(text: &str) -> Item {
        Item::User {
            text: text.to_string(),
            spoken: None,
        }
    }

    #[test]
    fn a_project_waits_on_its_run_any_revert_and_its_undecided_changes() {
        let tasks = vec![running(1, "a"), queued(2, "a", 1), queued(3, "b", 2)];
        assert_eq!(wait(&tasks, true, "a"), Some(Wait::Running { task: 1 }));
        assert_eq!(wait(&tasks, true, "b"), None);
        assert_eq!(ready(&tasks, true), [3]);

        // A revert holds every project.
        let tasks = vec![
            Task {
                reverting: true,
                ..with_change(task(1, "a"), 1)
            },
            queued(3, "b", 1),
        ];
        assert_eq!(wait(&tasks, true, "b"), Some(Wait::Reverting { task: 1 }));
        assert!(ready(&tasks, true).is_empty());
        // A project with no tasks yet, named, says a new one would wait.
        assert!(queues(&tasks, true, ["c"])["c"].holds);

        // A change waiting for Keep or Revert holds its project, measured or not;
        // an empty one, or untracked changes, do not.
        let waiting = vec![with_change(task(1, "a"), 1), queued(2, "a", 1)];
        assert_eq!(wait(&waiting, true, "a"), Some(Wait::Decision { task: 1 }));
        assert_eq!(wait(&waiting, false, "a"), None);
        // Another project's too: the snapshots cover the whole tree.
        assert_eq!(wait(&waiting, true, "b"), Some(Wait::Decision { task: 1 }));
        let measuring = Task {
            change: None,
            ..with_change(task(1, "a"), 1)
        };
        assert_eq!(
            wait(&[measuring], true, "a"),
            Some(Wait::Measuring { task: 1 })
        );
        assert_eq!(wait(&[with_change(task(1, "a"), 0)], true, "a"), None);
    }

    #[test]
    fn queues_are_in_place_order_per_project() {
        let tasks = vec![
            queued(5, "a", 3),
            running(1, "a"),
            queued(4, "a", 1),
            queued(6, "b", 2),
            task(7, "c"),
        ];
        let q = queues(&tasks, true, ["d"]);
        assert_eq!(q["a"].tasks, [4, 5]);
        assert_eq!(q["a"].wait, Some(Wait::Running { task: 1 }));
        assert_eq!(q["b"].tasks, [6]);
        // A project with nothing queued and nothing to wait on has none.
        assert!(!q.contains_key("c"));
        assert_eq!(last_place(&tasks), 4);
        assert_eq!(last_place(&[]), 1);
    }

    #[test]
    fn a_queued_task_moves_within_its_project_only() {
        let mut tasks = vec![
            queued(1, "a", 2),
            queued(2, "a", 4),
            queued(3, "b", 3),
            queued(4, "a", 5),
            task(5, "a"),
        ];
        // To the front: run next.
        assert_eq!(reorder(&mut tasks, 4, 0), Ok(vec![4, 1]));
        assert_eq!(order(&tasks, "a"), [4, 1, 2]);
        assert_eq!(tasks[2].queued, Some(3), "b's place is its own");
        // Past the end: last.
        reorder(&mut tasks, 4, 9).unwrap();
        assert_eq!(order(&tasks, "a"), [1, 2, 4]);
        assert_eq!(reorder(&mut tasks, 1, 0), Ok(vec![]));
        assert_eq!(
            reorder(&mut tasks, 5, 0),
            Err("task 5 is not queued".to_string())
        );
        assert!(reorder(&mut tasks, 9, 0).is_err());
    }

    #[test]
    fn a_first_run_gets_every_message_and_a_resumed_one_only_the_new() {
        let mut t = queued(1, "a", 1);
        t.transcript = vec![user("make the ball red")];
        assert_eq!(prompt(&t), "make the ball red");
        t.transcript.push(user("actually blue"));
        t.transcript.push(Item::User {
            text: "and bigger".to_string(),
            spoken: Some(Spoken {
                headset: "Quest".to_string(),
                context: None,
                route: None,
            }),
        });
        let sent = prompt(&t);
        assert!(
            sent.starts_with(&format!("make the ball red\n\n{LATER}\n\nactually blue\n\n")),
            "{sent}"
        );
        assert!(sent.ends_with("misheard.)\n\nand bigger"), "{sent}");

        // A stopped run with no session: the next run is a new session, so
        // it is told everything again.
        t.transcript = vec![user("one"), Item::Stopped, user("two")];
        assert_eq!(prompt(&t), format!("one\n\n{LATER}\n\ntwo"));
        // With a session to resume: only what came after its last run.
        t.session_id = Some("s".to_string());
        assert_eq!(prompt(&t), "two");
    }
}
