//! Tasks: each is one conversation with an agent in a project. The agent's
//! events map onto the task's transcript here, purely; the shell runs the
//! agent and feeds them in.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agent_view::WalkthroughStep;
use crate::changes::Change;
use crate::decision::Report;
use crate::evidence::{Evidence, Moment};
use crate::review::Review;
use crate::session::loopback_url;

/// The persisted format's version; a file with any other is skipped.
pub const STORE_VERSION: u32 = 1;

/// The outcome of a run cut off by the app stopping.
pub const INTERRUPTED: &str = "interrupted: the app stopped during this run";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Harness {
    #[default]
    Claude,
    Codex,
}

impl Harness {
    pub fn name(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Task {
    /// Unique among the loaded tasks.
    pub id: u64,
    /// The project's path relative to the repo root.
    pub project: String,
    /// Short, from the first message.
    pub title: String,
    /// From [`derive_status`], after every change.
    pub status: TaskStatus,
    pub harness: Harness,
    /// As the agent reported it.
    pub model: Option<String>,
    /// The agent's session, from its `started`; a follow-up resumes it.
    pub session_id: Option<String>,
    /// The run in flight.
    pub run: Option<Run>,
    /// Waiting its turn in its project's queue (D30): lower goes first. A
    /// queued task has never run; its transcript is the user's messages.
    #[serde(default)]
    pub queued: Option<u64>,
    /// The snapshot (a git tree in the app's store) the task's change is
    /// measured from: taken as its first run, or the first after a keep or
    /// revert, started. `None` outside git.
    #[serde(default)]
    pub baseline: Option<String>,
    /// What changed since `baseline`, as last computed.
    #[serde(default)]
    pub change: Option<Change>,
    /// A revert is being written; no run or second revert may start.
    #[serde(default)]
    pub reverting: bool,
    /// The game as `baseline` was taken (D38), when there was a game to ask.
    #[serde(default)]
    pub moment: Option<Moment>,
    /// The predicted paths before and after `change` (D38).
    #[serde(default)]
    pub evidence: Option<Evidence>,
    /// The last run's account of the change (D35), from its `present`, or
    /// its review's (`Report::reviewed`), which replaces it.
    #[serde(default)]
    pub report: Option<Report>,
    /// The separate run preparing the report on the waiting change (D31).
    #[serde(default)]
    pub review: Option<Review>,
    /// When it was started: the shell's wall clock, ms since the Unix
    /// epoch. Last active: a message, a run's event or a review. Both absent
    /// from tasks saved before they were kept.
    #[serde(default)]
    pub created_at: Option<u64>,
    /// Last active, as `created_at`.
    #[serde(default)]
    pub updated_at: Option<u64>,
    /// Sent as `ServerMsg::Transcript` and `ServerMsg::Item`, never with the
    /// state, which is resent on every session poll.
    #[serde(skip)]
    pub transcript: Vec<Item>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TaskStatus {
    /// Waiting its turn in its project's queue (D30).
    Queued,
    InProgress,
    /// The last run ended and the next move is the user's.
    WaitingOnYou,
    /// The last run finished ok with a change to keep or revert.
    ReadyForReview,
    /// The last run finished ok and left no change to review (an answer).
    Done,
    Failed {
        summary: Option<String>,
    },
    /// The change was kept: the task is done until a follow-up.
    Kept,
    /// The change was reverted: the task is done until a follow-up.
    Reverted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Run {
    /// The index of the user message that started it; events for another
    /// run of the task are dropped.
    pub id: u64,
    /// The latest (cumulative) usage, added as an item when the run ends.
    pub usage: Option<Usage>,
    /// View requests the run made ([`crate::agent_view::MAX_VIEW_REQUESTS`]).
    #[serde(default)]
    pub views: u32,
    /// The agent reported its start, so it has the run's messages.
    #[serde(default)]
    pub started: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Usage {
    /// Includes cached prompt tokens.
    pub input: u64,
    pub output: u64,
}

/// Where a spoken message came from (plan 4.1/4.3): the headset, and what
/// the user was looking at when they spoke, as text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Spoken {
    /// The headset's name.
    pub headset: String,
    /// The capsule, rendered: "at frame 1234 (20.5 s), head at …".
    #[cfg_attr(feature = "ts", ts(optional))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// Where the router put it (D29): "Steered “…”", "Queued behind “…”".
    #[cfg_attr(feature = "ts", ts(optional))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,
}

/// One transcript entry, in the order the UI shows them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Item {
    User {
        text: String,
        /// Said aloud in a headset, not typed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        spoken: Option<Spoken>,
    },
    /// `partial` while its text is still streaming in.
    Agent {
        text: String,
        partial: bool,
    },
    /// A tool call; `result` once its answer arrives.
    Tool {
        name: String,
        input: Value,
        result: Option<String>,
    },
    Usage {
        input: u64,
        output: u64,
    },
    Finished {
        ok: bool,
        summary: Option<String>,
    },
    /// The user stopped the run.
    Stopped,
    /// The user kept the task's change.
    Kept,
    /// The user reverted the task's change.
    Reverted,
    /// The agent's walkthrough (`present`, D25), which the user steps
    /// through; `current` is the step shown last, `None` until one is.
    Walkthrough {
        steps: Vec<WalkthroughStep>,
        current: Option<usize>,
    },
}

/// The app's mirror of `mis-agent`'s `AgentEvent` (which has no TS types),
/// limited to what a transcript shows. Same JSON shape.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    Started {
        session: Option<String>,
        #[serde(default)]
        model: Option<String>,
    },
    /// The next piece of the text being written; a `Text` with all of it follows.
    TextDelta {
        text: String,
    },
    Text {
        text: String,
    },
    ToolUse {
        name: String,
        input: Value,
    },
    /// Carries no id: it answers the oldest unanswered call.
    ToolResult {
        content: String,
    },
    /// Cumulative for the run.
    Usage {
        input: u64,
        output: u64,
    },
    Finished {
        ok: bool,
        summary: Option<String>,
    },
}

/// What the shell observed about one run of a task.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunEvent {
    /// The working tree's snapshot, taken before the agent started: the
    /// task's baseline (sent only when it had none).
    Baseline {
        tree: String,
    },
    /// The game as the baseline was taken (sent with it when asked).
    Moment(Moment),
    Agent(AgentEvent),
    /// The agent and its tool server are gone; `error` if the run could not
    /// start or broke.
    Ended {
        error: Option<String>,
    },
}

/// Applies `event` to `task`'s current run; the indices of the transcript
/// items it added or changed. Nothing without a run.
pub fn apply(task: &mut Task, event: AgentEvent) -> Vec<usize> {
    let Some(run) = task.run.as_mut() else {
        return vec![];
    };
    let items = &mut task.transcript;
    match event {
        AgentEvent::Started { session, model } => {
            run.started = true;
            task.session_id = session.or(task.session_id.take());
            task.model = model.or(task.model.take());
            vec![]
        }
        AgentEvent::TextDelta { text } => match items.last_mut() {
            Some(Item::Agent {
                text: so_far,
                partial: true,
            }) => {
                so_far.push_str(&text);
                vec![items.len() - 1]
            }
            _ => push(
                items,
                Item::Agent {
                    text,
                    partial: true,
                },
            ),
        },
        AgentEvent::Text { text } => match items.last_mut() {
            // The whole block replaces its streamed pieces.
            Some(last @ Item::Agent { partial: true, .. }) => {
                *last = Item::Agent {
                    text,
                    partial: false,
                };
                vec![items.len() - 1]
            }
            _ => push(
                items,
                Item::Agent {
                    text,
                    partial: false,
                },
            ),
        },
        AgentEvent::ToolUse { name, input } => push(
            items,
            Item::Tool {
                name,
                input,
                result: None,
            },
        ),
        AgentEvent::ToolResult { content } => {
            // The oldest call of this run without a result; a stray result is dropped.
            let start = run.id as usize + 1;
            let call = items
                .iter_mut()
                .enumerate()
                .skip(start)
                .find(|(_, item)| matches!(item, Item::Tool { result: None, .. }));
            match call {
                Some((i, Item::Tool { result, .. })) => {
                    *result = Some(content);
                    vec![i]
                }
                _ => vec![],
            }
        }
        AgentEvent::Usage { input, output } => {
            run.usage = Some(Usage { input, output });
            vec![]
        }
        AgentEvent::Finished { ok, summary } => {
            let mut changed = flush_usage(items, run);
            changed.extend(push(items, Item::Finished { ok, summary }));
            changed
        }
    }
}

/// Ends `task`'s run. A run the agent never finished gets a failed outcome
/// naming `error`.
pub fn end_run(task: &mut Task, error: Option<String>) -> Vec<usize> {
    let Some(mut run) = task.run.take() else {
        return vec![];
    };
    let items = &mut task.transcript;
    let mut changed = settle(items, &run);
    if run_outcome(items).is_none() {
        changed.extend(flush_usage(items, &mut run));
        let summary = error.unwrap_or_else(|| "the agent ended without finishing".to_string());
        changed.extend(push(
            items,
            Item::Finished {
                ok: false,
                summary: Some(summary),
            },
        ));
    }
    task.status = derive_status(task);
    changed
}

/// Stops `task`'s run, ending it with `Stopped`.
pub fn stop_run(task: &mut Task) -> Vec<usize> {
    let Some(mut run) = task.run.take() else {
        return vec![];
    };
    let mut changed = settle(&mut task.transcript, &run);
    changed.extend(flush_usage(&mut task.transcript, &mut run));
    changed.extend(push(&mut task.transcript, Item::Stopped));
    task.status = derive_status(task);
    changed
}

/// The status from the run, a review that closed the task, and how the
/// last run ended (with a change to review, after an ok finish).
pub fn derive_status(task: &Task) -> TaskStatus {
    if task.run.is_some() {
        return TaskStatus::InProgress;
    }
    if task.queued.is_some() {
        return TaskStatus::Queued;
    }
    match task.transcript.last() {
        Some(Item::Kept) => return TaskStatus::Kept,
        Some(Item::Reverted) => return TaskStatus::Reverted,
        _ => {}
    }
    match run_outcome(&task.transcript) {
        Some(Item::Finished { ok: false, summary }) => TaskStatus::Failed {
            summary: summary.clone(),
        },
        Some(Item::Finished { ok: true, .. }) if has_change(task) => TaskStatus::ReadyForReview,
        Some(Item::Finished { ok: true, .. }) => TaskStatus::Done,
        Some(_) => TaskStatus::WaitingOnYou,
        None if task.transcript.is_empty() => TaskStatus::WaitingOnYou,
        // A run with no outcome and no run in flight: the app stopped mid-run.
        None => TaskStatus::Failed {
            summary: Some(INTERRUPTED.to_string()),
        },
    }
}

/// A non-empty change since the task's baseline.
pub fn has_change(task: &Task) -> bool {
    task.baseline.is_some() && task.change.as_ref().is_some_and(|c| !c.is_empty())
}

/// The last run's `Finished` or `Stopped`: the last one after its user message.
fn run_outcome(items: &[Item]) -> Option<&Item> {
    items
        .iter()
        .rev()
        .take_while(|i| !matches!(i, Item::User { .. }))
        .find(|i| matches!(i, Item::Finished { .. } | Item::Stopped))
}

/// Ends any text of `run` still streaming, as no more of it will come.
fn settle(items: &mut [Item], run: &Run) -> Vec<usize> {
    let start = run.id as usize + 1;
    let mut changed = vec![];
    for (i, item) in items.iter_mut().enumerate().skip(start) {
        if let Item::Agent { partial, .. } = item {
            if *partial {
                *partial = false;
                changed.push(i);
            }
        }
    }
    changed
}

fn push(items: &mut Vec<Item>, item: Item) -> Vec<usize> {
    items.push(item);
    vec![items.len() - 1]
}

fn flush_usage(items: &mut Vec<Item>, run: &mut Run) -> Vec<usize> {
    match run.usage.take() {
        Some(Usage { input, output }) => push(items, Item::Usage { input, output }),
        None => vec![],
    }
}

/// Longest title, in characters.
const TITLE_CHARS: usize = 60;

/// A task's title: `message`'s first non-blank line, spaces collapsed, cut
/// at a word to [`TITLE_CHARS`] with `…`.
pub fn title(message: &str) -> String {
    let line = message.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= TITLE_CHARS {
        return line;
    }
    // Room for the `…`; a cut inside a word backs up to its start.
    let cut: String = line.chars().take(TITLE_CHARS).collect();
    let cut = match cut.rfind(' ') {
        Some(space) if space > 0 => &cut[..space],
        _ => {
            &cut[..cut
                .char_indices()
                .nth(TITLE_CHARS - 1)
                .map_or(cut.len(), |(i, _)| i)]
        }
    };
    format!("{cut}…")
}

/// Longest title a rename may give, in characters.
pub const RENAME_CHARS: usize = 120;

/// A title the user typed, spaces collapsed; why it cannot be one.
pub fn renamed(title: &str) -> Result<String, String> {
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    match title.chars().count() {
        0 => Err("a task's title cannot be empty".to_string()),
        n if n > RENAME_CHARS => Err(format!(
            "a task's title is at most {RENAME_CHARS} characters"
        )),
        _ => Ok(title),
    }
}

/// The tool server's base URL from its stderr line:
/// `[mcp] listening on http://127.0.0.1:61234/mcp` → `http://127.0.0.1:61234`.
pub fn mcp_listening_url(line: &str) -> Option<String> {
    loopback_url(line.trim().strip_suffix("/mcp")?, "[mcp]")
}

/// A task file: the summary and its transcript, versioned.
#[derive(Serialize)]
struct StoredRef<'a> {
    version: u32,
    task: &'a Task,
    transcript: &'a [Item],
}

/// Read once its `version` is checked.
#[derive(Deserialize)]
struct Stored {
    task: Task,
    transcript: Vec<Item>,
}

/// `task` as its file's JSON.
pub fn to_stored(task: &Task) -> String {
    let stored = StoredRef {
        version: STORE_VERSION,
        task,
        transcript: &task.transcript,
    };
    serde_json::to_string_pretty(&stored).expect("a task serializes")
}

/// The task in a file's JSON, or why it cannot be read.
pub fn from_stored(text: &str) -> Result<Task, String> {
    let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    match value["version"].as_u64() {
        Some(v) if v == u64::from(STORE_VERSION) => {}
        Some(v) => return Err(format!("format version {v}, not {STORE_VERSION}")),
        None => return Err("no format version".to_string()),
    }
    let stored: Stored = serde_json::from_value(value).map_err(|e| e.to_string())?;
    Ok(Task {
        transcript: stored.transcript,
        ..stored.task
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A fixture's events, as the shell would hand them in: each line is
    /// `{"ms": …, "event": <mis-agent AgentEvent>}`; events the app does not
    /// mirror (`limits`) fail to parse and are skipped.
    fn fixture(text: &str) -> Vec<AgentEvent> {
        text.lines()
            .filter_map(|line| {
                let line: Value = serde_json::from_str(line).unwrap();
                serde_json::from_value(line["event"].clone()).ok()
            })
            .collect()
    }

    const CLAUDE_GAME_TOOL: &str = include_str!("../fixtures/agent/claude-game-tool.jsonl");
    const CLAUDE_EDIT: &str = include_str!("../fixtures/agent/claude-edit.jsonl");
    const CODEX_GAME_TOOL: &str = include_str!("../fixtures/agent/codex-game-tool.jsonl");
    const CLAUDE_MISSING: &str = include_str!("../fixtures/agent/claude-missing.jsonl");

    fn running(message: &str) -> Task {
        Task {
            id: 1,
            project: "examples/counter".to_string(),
            title: title(message),
            status: TaskStatus::InProgress,
            harness: Harness::Claude,
            model: None,
            session_id: None,
            run: Some(Run {
                id: 0,
                usage: None,
                views: 0,
                started: false,
            }),
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
            transcript: vec![Item::User {
                text: message.to_string(),
                spoken: None,
            }],
        }
    }

    /// Applies every event and returns the changed indices, in order.
    fn replay(task: &mut Task, events: Vec<AgentEvent>) -> Vec<usize> {
        events.into_iter().flat_map(|e| apply(task, e)).collect()
    }

    fn agent(text: &str) -> Item {
        Item::Agent {
            text: text.to_string(),
            partial: false,
        }
    }

    #[test]
    fn claude_streams_text_then_finalises_it_and_pairs_the_tool_call() {
        let mut task = running("What is the count?");
        let changed = replay(&mut task, fixture(CLAUDE_GAME_TOOL));
        // Tool call, its result, three deltas into one item then the whole
        // text over it, usage and the outcome.
        assert_eq!(changed, [1, 1, 2, 2, 2, 2, 3, 4]);
        let answer = "The game is on frame 180 and the count is 3.";
        assert_eq!(
            task.transcript[1..],
            [
                Item::Tool {
                    name: "mcp__mis__get_state".to_string(),
                    input: json!({"session": "s1"}),
                    result: Some(r#"{"frame":180,"model":{"count":3}}"#.to_string()),
                },
                agent(answer),
                Item::Usage {
                    input: 2400,
                    output: 75
                },
                Item::Finished {
                    ok: true,
                    summary: Some(answer.to_string())
                },
            ]
        );
        assert_eq!(task.session_id.as_deref(), Some("fake-session-1"));
        assert_eq!(task.model.as_deref(), Some("claude-model"));
    }

    #[test]
    fn deltas_accumulate_in_one_partial_item() {
        let mut task = running("hi");
        let events = fixture(CLAUDE_GAME_TOOL);
        let deltas = events
            .into_iter()
            .filter(|e| matches!(e, AgentEvent::TextDelta { .. }))
            .take(2);
        replay(&mut task, deltas.collect());
        assert_eq!(
            task.transcript[1..],
            [Item::Agent {
                text: "The game is on frame 180 and ".to_string(),
                partial: true
            }]
        );
    }

    #[test]
    fn results_pair_with_the_oldest_unanswered_call() {
        let mut task = running("Rename the counter's label");
        replay(&mut task, fixture(CLAUDE_EDIT));
        let calls: Vec<(&str, Option<&str>)> = task
            .transcript
            .iter()
            .filter_map(|i| match i {
                Item::Tool { name, result, .. } => Some((name.as_str(), result.as_deref())),
                _ => None,
            })
            .collect();
        // Two reads went out before the first result came back.
        assert_eq!(
            calls,
            [
                ("Read", Some("1\t// counter")),
                ("Read", Some("1\t{ \"language\": \"functor-lang\" }")),
                ("Edit", Some("The file has been updated.")),
            ]
        );
        assert!(matches!(
            task.transcript.last(),
            Some(Item::Finished { ok: true, .. })
        ));
    }

    #[test]
    fn a_stray_result_or_an_earlier_runs_call_is_not_paired() {
        let mut task = running("one");
        apply(
            &mut task,
            AgentEvent::ToolUse {
                name: "Read".to_string(),
                input: json!({}),
            },
        );
        // A second run starts after the first was stopped mid-call.
        stop_run(&mut task);
        task.transcript.push(Item::User {
            text: "two".to_string(),
            spoken: None,
        });
        task.run = Some(Run {
            id: task.transcript.len() as u64 - 1,
            usage: None,
            views: 0,
            started: false,
        });
        let result = AgentEvent::ToolResult {
            content: "late".to_string(),
        };
        let before = task.transcript.clone();
        assert_eq!(apply(&mut task, result), Vec::<usize>::new());
        assert_eq!(task.transcript, before);
    }

    #[test]
    fn codex_sends_whole_text_and_mcp_calls() {
        let mut task = running("What is the count?");
        replay(&mut task, fixture(CODEX_GAME_TOOL));
        assert_eq!(
            task.transcript[1..],
            [
                agent("I'll check the game state."),
                Item::Tool {
                    name: "mcp__mis__get_state".to_string(),
                    input: json!({"session": "s1"}),
                    result: Some(r#"{"frame":403,"model":{"count":0}}"#.to_string()),
                },
                agent("Frame 403; the count is 0."),
                Item::Usage {
                    input: 5000,
                    output: 90
                },
                Item::Finished {
                    ok: true,
                    summary: Some("Frame 403; the count is 0.".to_string())
                },
            ]
        );
        assert_eq!(task.session_id.as_deref(), Some("fake-thread-1"));
        assert_eq!(task.model, None);
    }

    #[test]
    fn a_failed_finish_fails_the_task_with_its_summary() {
        let mut task = running("hi");
        replay(&mut task, fixture(CLAUDE_MISSING));
        end_run(&mut task, None);
        assert_eq!(
            task.status,
            TaskStatus::Failed {
                summary: Some("spawning claude".to_string())
            }
        );
    }

    #[test]
    fn status_rules() {
        let mut task = running("hi");
        assert_eq!(derive_status(&task), TaskStatus::InProgress);
        // A run that finished ok and changed nothing is done (an answer).
        replay(&mut task, fixture(CLAUDE_GAME_TOOL));
        assert_eq!(derive_status(&task), TaskStatus::InProgress);
        assert_eq!(end_run(&mut task, None), Vec::<usize>::new());
        assert_eq!(task.status, TaskStatus::Done);

        // Stopped by the user: waiting on them.
        let mut task = running("hi");
        apply(
            &mut task,
            AgentEvent::Usage {
                input: 1,
                output: 2,
            },
        );
        assert_eq!(stop_run(&mut task), [1, 2]);
        assert_eq!(
            task.transcript[1..],
            [
                Item::Usage {
                    input: 1,
                    output: 2
                },
                Item::Stopped
            ]
        );
        assert_eq!(task.status, TaskStatus::WaitingOnYou);

        // Ended without an outcome: failed, with the shell's error.
        let mut task = running("hi");
        let error = "functor mcp exited before listening".to_string();
        assert_eq!(end_run(&mut task, Some(error.clone())), [1]);
        assert_eq!(
            task.status,
            TaskStatus::Failed {
                summary: Some(error)
            }
        );

        // Saved mid-run and loaded with no run: interrupted.
        let mut task = running("hi");
        task.run = None;
        assert_eq!(
            derive_status(&task),
            TaskStatus::Failed {
                summary: Some(INTERRUPTED.to_string())
            }
        );
    }

    #[test]
    fn an_ended_run_settles_its_streaming_text_and_late_events_keep_the_outcome() {
        let mut task = running("hi");
        let delta = AgentEvent::TextDelta {
            text: "Hal".to_string(),
        };
        apply(&mut task, delta);
        assert_eq!(stop_run(&mut task), [1, 2]);
        assert_eq!(task.transcript[1], agent("Hal"));

        // Text after the outcome, within the run, does not make it look cut off.
        let mut task = running("hi");
        replay(&mut task, fixture(CLAUDE_GAME_TOOL));
        apply(
            &mut task,
            AgentEvent::Text {
                text: "late".to_string(),
            },
        );
        end_run(&mut task, None);
        assert_eq!(task.status, TaskStatus::Done);
    }

    #[test]
    fn events_without_a_run_change_nothing() {
        let mut task = running("hi");
        end_run(&mut task, None);
        let before = task.clone();
        let text = AgentEvent::Text {
            text: "late".to_string(),
        };
        assert_eq!(apply(&mut task, text), Vec::<usize>::new());
        assert_eq!(stop_run(&mut task), Vec::<usize>::new());
        assert_eq!(task, before);
    }

    #[test]
    fn titles_are_the_first_line_cut_at_a_word() {
        assert_eq!(
            title("\n  Make the  cube spin\nfaster please"),
            "Make the cube spin"
        );
        let long = "Make the cube spin faster when the count goes up and slower when it goes down";
        let t = title(long);
        assert_eq!(
            t,
            "Make the cube spin faster when the count goes up and slower…"
        );
        assert!(t.chars().count() <= TITLE_CHARS, "{t}");
        assert_eq!(title(""), "");
    }

    #[test]
    fn a_renamed_title_is_collapsed_and_bounded() {
        assert_eq!(
            renamed("  Spin \n the  cube "),
            Ok("Spin the cube".to_string())
        );
        assert!(renamed(" \t ").is_err());
        let long = "x".repeat(RENAME_CHARS);
        assert_eq!(renamed(&long), Ok(long.clone()));
        assert!(renamed(&format!("{long}y")).is_err());
    }

    #[test]
    fn reads_the_tool_servers_url_from_its_listening_line() {
        assert_eq!(
            mcp_listening_url("[mcp] listening on http://127.0.0.1:61234/mcp\n").as_deref(),
            Some("http://127.0.0.1:61234")
        );
        for other in [
            "[mcp] listening on http://127.0.0.1:61234",
            "[mcp] listening on http://localhost:61234/mcp",
            "[debug-server] listening on http://127.0.0.1:61234",
            "",
        ] {
            assert_eq!(mcp_listening_url(other), None, "{other}");
        }
    }

    #[test]
    fn the_stored_form_round_trips_and_is_versioned() {
        let mut task = running("What is the count?");
        replay(&mut task, fixture(CLAUDE_GAME_TOOL));
        end_run(&mut task, None);
        let text = to_stored(&task);
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["version"], STORE_VERSION);
        assert_eq!(value["task"]["status"]["kind"], "done");
        assert_eq!(value["transcript"][0]["kind"], "user");
        assert_eq!(from_stored(&text), Ok(task.clone()));

        // A file from before tasks kept their times loads without them.
        let mut old = value.clone();
        let fields = old["task"].as_object_mut().unwrap();
        fields.remove("created_at").unwrap();
        fields.remove("updated_at").unwrap();
        let mut timed = task;
        timed.updated_at = Some(5);
        let loaded = from_stored(&old.to_string()).unwrap();
        assert_eq!((loaded.created_at, loaded.updated_at), (None, None));
        assert_eq!(
            Task {
                updated_at: Some(5),
                ..loaded
            },
            timed
        );

        let other = text.replacen("\"version\": 1", "\"version\": 2", 1);
        assert_eq!(
            from_stored(&other),
            Err("format version 2, not 1".to_string())
        );
        assert!(from_stored("{").is_err());
        assert!(from_stored(r#"{"version": 1}"#).is_err());
    }

    #[test]
    fn the_wire_form_leaves_the_transcript_out() {
        let task = running("hi");
        let value = serde_json::to_value(&task).unwrap();
        assert!(value.get("transcript").is_none(), "{value}");
        assert_eq!(value["status"]["kind"], "in_progress");
        assert_eq!(value["harness"], "claude");
    }
}
