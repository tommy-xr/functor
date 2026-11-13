//! The review (D31): when a run ends with a change for the user to decide
//! on, a separate, read-only agent run prepares the decision page's report.
//! Its setting per project, its state on the task, and the brief it reads.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::decision::{kind, Kind, Report};
use crate::evidence::EvidenceState;
use crate::projects::project_prefix;
use crate::tasks::{AgentEvent, Harness, Item, Task, Usage};

/// Longest model name and instructions a setting may hold, in characters.
const MAX_MODEL: usize = 100;
const MAX_INSTRUCTIONS: usize = 2000;

/// How a project's changes are reviewed; stored per project.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ReviewSetting {
    /// Prepare the report in a separate run; off, the agent's own is shown.
    pub on: bool,
    /// `None`: the task's own.
    pub harness: Option<Harness>,
    /// `None`: [`default_model`] for the harness.
    pub model: Option<String>,
    /// Added to the review's instructions, e.g. a skill to follow.
    pub instructions: Option<String>,
}

impl Default for ReviewSetting {
    fn default() -> Self {
        Self {
            on: true,
            harness: None,
            model: None,
            instructions: None,
        }
    }
}

impl ReviewSetting {
    /// The setting with blanks as `None`, or why it cannot be one.
    pub fn checked(self) -> Result<Self, String> {
        let blank = |s: Option<String>| s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        let model = blank(self.model);
        let instructions = blank(self.instructions);
        // One `--model=` argument: a name, not a flag or a phrase.
        let named = |m: &String| {
            m.chars().count() <= MAX_MODEL
                && !m.starts_with('-')
                && (m.chars()).all(|c| c.is_ascii_alphanumeric() || "-_.:/[]".contains(c))
        };
        if model.as_ref().is_some_and(|m| !named(m)) {
            return Err(format!(
                "a model is a name of at most {MAX_MODEL} letters, digits and -_.:/[]"
            ));
        }
        if instructions
            .as_ref()
            .is_some_and(|i| i.chars().count() > MAX_INSTRUCTIONS)
        {
            return Err(format!(
                "the instructions are at most {MAX_INSTRUCTIONS} characters"
            ));
        }
        Ok(Self {
            model,
            instructions,
            ..self
        })
    }
}

/// The model a review runs when the setting names none: a cheaper, faster
/// one where the harness has a name for it, else the harness's default.
pub fn default_model(harness: Harness) -> Option<&'static str> {
    match harness {
        Harness::Claude => Some("sonnet"),
        // Codex has no stable alias for a smaller model.
        Harness::Codex => None,
    }
}

/// A task's review of its waiting change, kept until the change is decided,
/// replaced or measured again differently.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Review {
    /// The run whose change it reviews; its events carry it.
    pub run: u64,
    /// That change (`Change::tree`).
    pub tree: String,
    pub harness: Harness,
    /// As asked, until the agent reports its own.
    pub model: Option<String>,
    pub state: ReviewState,
    /// The latest (cumulative) usage it reported.
    pub usage: Option<Usage>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReviewState {
    Preparing,
    /// Its report is the task's.
    Prepared,
    /// It left no report; the page is as it was.
    Failed { reason: String },
}

/// One review run for the shell to start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewSpec {
    pub task: u64,
    pub run: u64,
    pub harness: Harness,
    pub model: Option<String>,
    pub instructions: Option<String>,
    /// The task's project directory: the agent's working directory.
    pub dir: PathBuf,
    /// What the agent may read.
    pub repo_root: PathBuf,
    /// The diff it reviews runs from `baseline` to `tree`.
    pub baseline: String,
    pub tree: String,
    /// [`brief`]: the shell appends the diff.
    pub brief: String,
}

/// What the shell observed about a review run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReviewEvent {
    Agent(AgentEvent),
    /// The agent and its tool server are gone; `error` if it was cut, could
    /// not start or broke.
    Ended { error: Option<String> },
}

/// Longest user message, final text, and tool input and result, in
/// characters, and the most tool calls the brief lists.
const BRIEF_MESSAGE: usize = 2000;
const BRIEF_INPUT: usize = 200;
const BRIEF_RESULT: usize = 300;
const BRIEF_CALLS: usize = 40;

/// What the review reads first (`task_change`), as Markdown: the request,
/// the last run's calls and words, the change, the evidence so far and the
/// agent's own report. The shell adds the diff.
pub fn brief(task: &Task) -> String {
    let mut out = format!("# Task: {}\n\nProject: `{}`. Paths are relative to the repo root.\n", task.title, task.project);
    let users: Vec<&str> = (task.transcript.iter())
        .filter_map(|i| match i {
            Item::User { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    out += "\n## What the user asked\n\n";
    for (n, text) in users.iter().enumerate() {
        if n > 0 {
            out += "Then:\n\n";
        }
        out += &format!("{}\n\n", quote(&cut(text, BRIEF_MESSAGE)));
    }
    let start = (task.transcript.iter())
        .rposition(|i| matches!(i, Item::User { .. }))
        .map_or(0, |i| i + 1);
    let run = &task.transcript[start..];
    let calls: Vec<String> = (run.iter())
        .filter_map(|i| match i {
            Item::Tool {
                name,
                input,
                result,
            } => Some(format!(
                "- `{name}` {} → {}",
                cut(&compact(input), BRIEF_INPUT),
                result
                    .as_deref()
                    .map_or("(no result)".to_string(), |r| cut(&one_line(r), BRIEF_RESULT))
            )),
            _ => None,
        })
        .collect();
    out += "## What the agent did in its last run\n\n";
    if calls.is_empty() {
        out += "No tool calls.\n";
    }
    for line in calls.iter().take(BRIEF_CALLS) {
        out += &format!("{line}\n");
    }
    if calls.len() > BRIEF_CALLS {
        out += &format!("- … and {} more calls\n", calls.len() - BRIEF_CALLS);
    }
    let last = run.iter().rev().find_map(|i| match i {
        Item::Agent { text, .. } => Some(text.as_str()),
        _ => None,
    });
    if let Some(text) = last {
        out += &format!("\nIts last words:\n\n{}\n", quote(&cut(text, BRIEF_MESSAGE)));
    }
    if let Some(change) = &task.change {
        let prefix = project_prefix(&task.project);
        let (added, removed): (u32, u32) = (change.files.iter())
            .fold((0, 0), |(a, r), f| (a + f.added, r + f.removed));
        let files = change.files.len() as u32 + change.more;
        out += &format!("\n## The change\n\n{files} files, +{added} −{removed}:\n\n");
        for f in &change.files {
            let what = match kind(f, &prefix) {
                Kind::GameLogic => "game logic",
                Kind::Assets => "asset",
                Kind::Config => "config",
                Kind::Other => "other",
            };
            let from = f.from.as_deref().map_or(String::new(), |p| format!(" (from `{p}`)"));
            out += &format!("- `{}`{from} +{} −{} ({what})\n", f.path, f.added, f.removed);
        }
        if change.more > 0 {
            out += &format!("- … and {} more files\n", change.more);
        }
    }
    out += "\n## Evidence the app has\n\n";
    out += &match task.evidence.as_ref().map(|e| &e.state) {
        Some(EvidenceState::Shown { .. }) => "- The predicted path from the moment the task \
            started, under the old and the new code: they differ, and the page shows both.\n"
            .to_string(),
        Some(EvidenceState::Same) => {
            "- The predicted path is the same under the old and the new code.\n".to_string()
        }
        Some(EvidenceState::Taking) => "- The predicted path is being taken.\n".to_string(),
        Some(EvidenceState::Withheld { reason }) => format!("- No predicted path: {reason}.\n"),
        None => "- None.\n".to_string(),
    };
    out += "\n## The agent's own report\n\n";
    match &task.report {
        Some(Report {
            summary,
            notes,
            evidence,
            reviewed: false,
        }) => {
            if let Some(summary) = summary {
                out += &format!("Summary: {summary}\n\n");
            }
            match notes {
                Some(notes) if notes.is_empty() => out += "Notes: none.\n",
                Some(notes) => {
                    out += "Notes:\n";
                    for n in notes {
                        out += &format!("- {n}\n");
                    }
                }
                None => out += "Notes: not said.\n",
            }
            for c in evidence {
                let ok = if c.ok { "ok" } else { "FAILED" };
                out += &format!("- Ran `{}`: {} ({ok})\n", c.ran, c.result);
            }
        }
        _ => out += "None.\n",
    }
    let walked = run.iter().any(|i| matches!(i, Item::Walkthrough { .. }));
    out += &format!(
        "\nThe agent {} a walkthrough (highlights).\n",
        if walked { "gave" } else { "gave no" }
    );
    out
}

fn quote(text: &str) -> String {
    text.lines().map(|l| format!("> {l}")).collect::<Vec<_>>().join("\n")
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn compact(input: &Value) -> String {
    one_line(&input.to_string())
}

/// `text`'s first `max` characters, `…` marking a cut.
fn cut(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_setting_is_trimmed_and_checked() {
        let set = |model: &str, instructions: &str| {
            ReviewSetting {
                on: true,
                harness: None,
                model: Some(model.to_string()),
                instructions: Some(instructions.to_string()),
            }
            .checked()
        };
        assert_eq!(set(" ", "  "), Ok(ReviewSetting::default()));
        let ok = set("claude-sonnet-4-5", " Use the review skill. ").unwrap();
        assert_eq!(ok.model.as_deref(), Some("claude-sonnet-4-5"));
        assert_eq!(ok.instructions.as_deref(), Some("Use the review skill."));
        for model in ["--dangerously-skip-permissions", "two words", &"m".repeat(101)] {
            assert!(set(model, "").is_err(), "{model}");
        }
        assert!(set("haiku", &"x".repeat(2001)).is_err());
    }

    #[test]
    fn cuts_mark_themselves() {
        assert_eq!(cut("abcdef", 3), "abc…");
        assert_eq!(cut("abc", 3), "abc");
        assert_eq!(cut("ééé", 2), "éé…");
    }
}
