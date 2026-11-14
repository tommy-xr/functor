//! The decision page (D35): what the agent reports about a change, and what
//! the app derives from a change that waits for the user — its size, the
//! kinds of file it touches, and how deep to look, by an explicit rule.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::changes::{Change, FileChange};
use crate::evidence::EvidenceState;
use crate::projects::project_prefix;
use crate::tasks::{has_change, Task};

/// A glance needs at most this many changed lines (added + removed)…
pub const GLANCE_LINES: u32 = 20;
/// …in at most this many files.
pub const GLANCE_FILES: u32 = 1;

/// Longest summary, note, and evidence field, in characters.
const MAX_SUMMARY: usize = 1200;
const MAX_LINE: usize = 300;
/// Most notes, and most evidence lines.
pub const MAX_ITEMS: usize = 8;

/// The agent's account of its change (`present`), stored with the task; a
/// separate review's (`report`, D31) replaces it. A new run, a later edit to
/// the files or the end of the review clears it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Report {
    /// One paragraph in Simplified Technical English.
    pub summary: Option<String>,
    /// What was not verified, what was assumed, side effects; `None`: the
    /// agent did not say (unlike `Some([])`, "nothing to note").
    pub notes: Option<Vec<String>>,
    /// What was run, and the result.
    pub evidence: Vec<Check>,
    /// Written by a separate review, not the agent that made the change.
    #[serde(default)]
    pub reviewed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct Check {
    /// What was run, e.g. `functor test`.
    pub ran: String,
    /// What came of it, e.g. "14 expects pass".
    pub result: String,
    pub ok: bool,
}

impl Report {
    /// The report, `None` when it says nothing; or why it is malformed.
    pub fn new(
        summary: Option<String>,
        notes: Option<Vec<String>>,
        evidence: Vec<Check>,
    ) -> Result<Option<Self>, String> {
        let listed = notes.as_deref().unwrap_or_default();
        let summary = summary
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        if summary
            .as_ref()
            .is_some_and(|s| s.chars().count() > MAX_SUMMARY)
        {
            return Err(format!("a summary is at most {MAX_SUMMARY} characters"));
        }
        if listed.len() > MAX_ITEMS || evidence.len() > MAX_ITEMS {
            return Err(format!(
                "at most {MAX_ITEMS} notes and {MAX_ITEMS} evidence lines"
            ));
        }
        let line = |s: &str| !s.trim().is_empty() && s.chars().count() <= MAX_LINE;
        if !listed.iter().all(|n| line(n))
            || !evidence.iter().all(|c| line(&c.ran) && line(&c.result))
        {
            return Err(format!(
                "each note, `ran` and `result` is 1 to {MAX_LINE} characters"
            ));
        }
        let report = Self {
            summary,
            notes,
            evidence,
            reviewed: false,
        };
        Ok((report != Self::default()).then_some(report))
    }
}

/// Simplified Technical English (ASD-STE100) caps a sentence at this many
/// words…
pub const STE_SENTENCE_WORDS: usize = 20;
/// …and the summary is one short paragraph: at most this many.
pub const SUMMARY_WORDS: usize = 120;

/// What a mechanical look at `summary` finds against the STE rules it can
/// count: long sentences, a long or split paragraph. Shown, never enforced.
pub fn style_flags(summary: &str) -> Vec<String> {
    let mut flags = vec![];
    if summary.contains("\n\n") {
        flags.push("It has more than one paragraph.".to_string());
    }
    let words = summary.split_whitespace().count();
    if words > SUMMARY_WORDS {
        flags.push(format!("It has {words} words (at most {SUMMARY_WORDS})."));
    }
    // Words per sentence. A sentence ends at a word ending in `.`, `!` or
    // `?`, so `game.fun` and `2.5` stay whole.
    let mut sentences = vec![0];
    for word in summary.split_whitespace() {
        *sentences.last_mut().expect("never empty") += 1;
        if word.ends_with(['.', '!', '?']) {
            sentences.push(0);
        }
    }
    for (i, n) in sentences.iter().enumerate() {
        if *n > STE_SENTENCE_WORDS {
            let i = i + 1;
            flags.push(format!(
                "Sentence {i} has {n} words (at most {STE_SENTENCE_WORDS})."
            ));
        }
    }
    flags
}

/// What a changed file is, by its extension (and, for game logic, its place).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// `.fun` and `.funi` in the task's project.
    GameLogic,
    /// Models, textures, sounds, fonts, and any binary file.
    Assets,
    /// `.json` (`functor.json`, asset sidecars), `.toml`, `.yaml`.
    Config,
    Other,
}

const ASSETS: &[&str] = &[
    "png", "jpg", "jpeg", "webp", "gif", "hdr", "ktx2", "glb", "gltf", "bin", "obj", "wav", "ogg",
    "mp3", "flac", "ttf", "otf",
];

/// `file`'s kind in a change to the project at `prefix` ([`project_prefix`]).
pub fn kind(file: &FileChange, prefix: &str) -> Kind {
    let ext = file
        .path
        .rsplit_once('.')
        .filter(|(stem, _)| !stem.ends_with('/') && !stem.is_empty())
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "fun" | "funi" if file.path.starts_with(prefix) => Kind::GameLogic,
        _ if file.binary || ASSETS.contains(&ext.as_str()) => Kind::Assets,
        "json" | "toml" | "yaml" | "yml" => Kind::Config,
        _ => Kind::Other,
    }
}

/// A change's size: lines over the listed files, files including those
/// past the list (whose kinds and lines are unknown).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Size {
    pub added: u32,
    pub removed: u32,
    pub files: u32,
    /// Each kind once, in [`Kind`]'s order.
    pub kinds: Vec<Kind>,
}

/// `change`'s size, in project `project` (repo-relative, `.` the root).
pub fn size(change: &Change, project: &str) -> Size {
    let prefix = project_prefix(project);
    let mut kinds: Vec<Kind> = change.files.iter().map(|f| kind(f, &prefix)).collect();
    kinds.sort();
    kinds.dedup();
    Size {
        added: change.files.iter().map(|f| f.added).sum(),
        removed: change.files.iter().map(|f| f.removed).sum(),
        files: change.files.len() as u32 + change.more,
        kinds,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Depth {
    /// "Approve at a glance": the first page is enough.
    Glance,
    /// "Look into this".
    Look,
}

/// A waiting change's first page, as far as the app derives it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Decision {
    pub size: Size,
    pub depth: Depth,
    /// One or two short sentences.
    pub reason: String,
    /// The report's summary against the rules [`style_flags`] counts.
    pub style: Vec<String>,
}

/// The decision for each task whose change waits for the user.
pub fn decisions(tasks: &[Task]) -> BTreeMap<u64, Decision> {
    tasks
        .iter()
        .filter_map(|t| decision(t).map(|d| (t.id, d)))
        .collect()
}

/// `task`'s decision while its change waits: no run, and a change.
pub fn decision(task: &Task) -> Option<Decision> {
    if task.run.is_some() || !has_change(task) {
        return None;
    }
    let size = size(task.change.as_ref()?, &task.project);
    let path = task.evidence.as_ref().map(|e| &e.state);
    let report = task.report.as_ref();
    let (depth, reason) = verdict(&size, report, path);
    let summary = report.and_then(|r| r.summary.as_deref());
    Some(Decision {
        size,
        depth,
        reason,
        style: summary.map(style_flags).unwrap_or_default(),
    })
}

/// The rule: a glance needs no failed check and nothing to note, at most
/// [`GLANCE_LINES`] changed lines in at most [`GLANCE_FILES`] game-logic
/// file, a report that says there is nothing to note, and evidence — a
/// check the agent ran, or the predicted path (`path`) shown. Else "look",
/// for the first reason in that order.
pub fn verdict(
    size: &Size,
    report: Option<&Report>,
    path: Option<&EvidenceState>,
) -> (Depth, String) {
    let look = |reason: String| (Depth::Look, reason);
    let checks = report.map_or(&[][..], |r| &r.evidence[..]);
    if let Some(failed) = checks.iter().find(|c| !c.ok) {
        return look(format!("A check failed: {}.", failed.ran));
    }
    let who = match report {
        Some(Report { reviewed: true, .. }) => "The review",
        _ => "The agent",
    };
    let notes = report.and_then(|r| r.notes.as_ref()).map_or(0, Vec::len);
    if notes > 0 {
        return look(format!(
            "{who} notes {}.",
            count(notes as u32, "thing", "things")
        ));
    }
    let lines = size.added + size.removed;
    if size.kinds != [Kind::GameLogic] {
        return look("It changes more than game logic.".to_string());
    }
    if size.files > GLANCE_FILES {
        return look(format!("{} changed.", count(size.files, "file", "files")));
    }
    if lines > GLANCE_LINES {
        return look(format!(
            "{}.",
            count(lines, "changed line", "changed lines")
        ));
    }
    match report {
        None => return look("The agent gave no report.".to_string()),
        Some(Report { notes: None, .. }) => {
            return look(format!("{who} did not say if there is anything to note."))
        }
        Some(_) => {}
    }
    let shown = matches!(path, Some(EvidenceState::Shown { .. }));
    if checks.is_empty() && !shown {
        return look(
            match path {
                Some(EvidenceState::Taking) => {
                    "No check was reported; the predicted path is being taken."
                }
                Some(EvidenceState::Same) => {
                    "No check was reported, and the predicted path does not change."
                }
                _ => "No check was reported and no path was compared.",
            }
            .to_string(),
        );
    }
    let mut reason = format!(
        "{} in one file.",
        count(lines, "changed line", "changed lines")
    );
    if !checks.is_empty() {
        let n = checks.len() as u32;
        reason += &format!(
            " {} {}.",
            count(n, "check", "checks"),
            if n == 1 { "passes" } else { "pass" }
        );
    }
    if shown {
        reason += " The predicted path shows it.";
    }
    (Depth::Glance, reason)
}

fn count(n: u32, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, added: u32, removed: u32) -> FileChange {
        FileChange {
            path: path.to_string(),
            from: None,
            added,
            removed,
            binary: false,
            large: false,
        }
    }

    #[test]
    fn kinds_go_by_extension() {
        let binary = FileChange {
            binary: true,
            ..file("game/data.dat", 0, 0)
        };
        for (f, k) in [
            (file("game/game.fun", 1, 1), Kind::GameLogic),
            (file("game/hud.funi", 1, 0), Kind::GameLogic),
            (file("other/game.fun", 1, 1), Kind::Other),
            (file("functor-prelude/stdlib/list.fun", 1, 1), Kind::Other),
            (file("game/Hero.GLB", 0, 0), Kind::Assets),
            (file("game/jump.ogg", 0, 0), Kind::Assets),
            (binary, Kind::Assets),
            (file("game/functor.json", 1, 1), Kind::Config),
            (file("game/shark.asset.json", 1, 0), Kind::Config),
            (file("game/README.md", 3, 0), Kind::Other),
            (file("runtime/src/lib.rs", 3, 0), Kind::Other),
            (file("game/.gitignore", 1, 0), Kind::Other),
            (file("Makefile", 1, 0), Kind::Other),
        ] {
            assert_eq!(kind(&f, "game/"), k, "{}", f.path);
        }
    }

    #[test]
    fn size_sums_listed_lines_and_counts_every_file() {
        let change = Change {
            files: vec![
                file("g/a.fun", 3, 1),
                file("g/b.png", 0, 0),
                file("g/c.fun", 2, 2),
            ],
            more: 4,
            tree: "t".into(),
        };
        assert_eq!(
            size(&change, "g"),
            Size {
                added: 5,
                removed: 3,
                files: 7,
                kinds: vec![Kind::GameLogic, Kind::Assets],
            }
        );
    }

    fn report(notes: &[&str], checks: &[(&str, bool)]) -> Report {
        Report {
            summary: Some("The agent changed one value.".into()),
            notes: Some(notes.iter().map(|n| n.to_string()).collect()),
            evidence: checks
                .iter()
                .map(|(ran, ok)| Check {
                    ran: ran.to_string(),
                    result: "r".into(),
                    ok: *ok,
                })
                .collect(),
            reviewed: false,
        }
    }

    fn sized(files: u32, lines: u32, kinds: &[Kind]) -> Size {
        Size {
            added: lines,
            removed: 0,
            files,
            kinds: kinds.to_vec(),
        }
    }

    #[test]
    fn the_verdict_rule() {
        use Depth::{Glance, Look};
        const LOGIC: &[Kind] = &[Kind::GameLogic];
        let shown = Some(EvidenceState::Shown {
            before: "b".into(),
            after: "a".into(),
        });
        let (taking, same) = (Some(EvidenceState::Taking), Some(EvidenceState::Same));
        let ok = report(&[], &[("functor test", true)]);
        let two = report(&[], &[("functor test", true), ("functor build", true)]);
        let bare = report(&[], &[]);
        let unsaid = Report {
            notes: None,
            ..ok.clone()
        };
        let noted = report(&["Not tried on the left hand"], &[("functor test", true)]);
        let failed = report(&[], &[("functor test", false)]);
        let small = sized(1, 4, LOGIC);
        #[rustfmt::skip]
        let table: &[(&str, Size, Option<&Report>, &Option<EvidenceState>, Depth, &str)] = &[
            ("small, checked", small.clone(), Some(&ok), &None, Glance, "4 changed lines in one file. 1 check passes."),
            ("small, two checks and the path", small.clone(), Some(&two), &shown, Glance, "4 changed lines in one file. 2 checks pass. The predicted path shows it."),
            ("small, only the path", small.clone(), Some(&bare), &shown, Glance, "4 changed lines in one file. The predicted path shows it."),
            ("at the line limit", sized(1, GLANCE_LINES, LOGIC), Some(&ok), &None, Glance, "20 changed lines in one file. 1 check passes."),
            ("one line", sized(1, 1, LOGIC), Some(&ok), &None, Glance, "1 changed line in one file. 1 check passes."),
            ("past the line limit", sized(1, GLANCE_LINES + 1, LOGIC), Some(&ok), &shown, Look, "21 changed lines."),
            ("two files", sized(2, 4, LOGIC), Some(&ok), &shown, Look, "2 files changed."),
            ("an asset", sized(1, 0, &[Kind::Assets]), Some(&ok), &shown, Look, "It changes more than game logic."),
            ("logic and config", sized(2, 4, &[Kind::GameLogic, Kind::Config]), Some(&ok), &shown, Look, "It changes more than game logic."),
            ("unverified", small.clone(), Some(&bare), &None, Look, "No check was reported and no path was compared."),
            ("the path is being taken", small.clone(), Some(&bare), &taking, Look, "No check was reported; the predicted path is being taken."),
            ("the path does not move", small.clone(), Some(&bare), &same, Look, "No check was reported, and the predicted path does not change."),
            ("no report", small.clone(), None, &shown, Look, "The agent gave no report."),
            ("notes left out", small.clone(), Some(&unsaid), &shown, Look, "The agent did not say if there is anything to note."),
            ("a note", small.clone(), Some(&noted), &shown, Look, "The agent notes 1 thing."),
            ("a failed check first", sized(9, 400, LOGIC), Some(&failed), &shown, Look, "A check failed: functor test."),
        ];
        for (what, size, report, path, depth, reason) in table {
            assert_eq!(
                verdict(size, *report, path.as_ref()),
                (*depth, reason.to_string()),
                "{what}"
            );
        }
        // A separate review's report says so.
        let reviewed = |r: &Report| Report {
            reviewed: true,
            ..r.clone()
        };
        assert_eq!(
            verdict(&small, Some(&reviewed(&noted)), None).1,
            "The review notes 1 thing."
        );
        assert_eq!(
            verdict(&small, Some(&reviewed(&unsaid)), shown.as_ref()).1,
            "The review did not say if there is anything to note."
        );
        assert_eq!(verdict(&small, Some(&reviewed(&ok)), None).0, Glance);
    }

    #[test]
    fn style_flags_count_sentences_and_the_paragraph() {
        assert!(style_flags("The jump is higher. Check the chasm in `game.fun`.").is_empty());
        let long = vec!["word"; 21].join(" ");
        assert_eq!(
            style_flags(&format!("Short one. {long}. Short again")),
            ["Sentence 2 has 21 words (at most 20)."]
        );
        // The last sentence counts without its full stop; `2.5 m` is one sentence.
        assert_eq!(
            style_flags(&format!("It moves 2.5 m. {long}")),
            ["Sentence 2 has 21 words (at most 20)."]
        );
        let many = vec!["Six words in each sentence here."; 21].join(" ");
        assert_eq!(style_flags(&many), ["It has 126 words (at most 120)."]);
        assert_eq!(
            style_flags("One.\n\nTwo."),
            ["It has more than one paragraph."]
        );
    }

    #[test]
    fn a_report_is_trimmed_bounded_and_none_when_empty() {
        assert_eq!(Report::new(Some("  ".into()), None, vec![]), Ok(None));
        let r = Report::new(Some(" Two values changed. ".into()), None, vec![]).unwrap();
        assert_eq!(r.unwrap().summary.as_deref(), Some("Two values changed."));
        // An empty list says "nothing to note": a report.
        assert!(Report::new(None, Some(vec![]), vec![]).unwrap().is_some());
        let check = |ran: &str| Check {
            ran: ran.into(),
            result: "ok".into(),
            ok: true,
        };
        for (summary, notes, evidence) in [
            (Some("x".repeat(MAX_SUMMARY + 1)), None, vec![]),
            (None, Some(vec!["n".to_string(); MAX_ITEMS + 1]), vec![]),
            (None, Some(vec![" ".to_string()]), vec![]),
            (None, None, vec![check("")]),
            (None, None, vec![check(&"x".repeat(MAX_LINE + 1))]),
        ] {
            assert!(Report::new(summary, notes, evidence).is_err());
        }
    }

    #[test]
    fn only_a_waiting_change_has_a_decision() {
        let mut task: Task = serde_json::from_value(serde_json::json!({
            "id": 1, "project": "game", "title": "t", "status": {"kind": "ready_for_review"},
            "harness": "claude", "model": null, "session_id": null, "run": null,
        }))
        .unwrap();
        assert_eq!(decision(&task), None, "no change");
        task.baseline = Some("b".into());
        task.change = Some(Change::from_files(
            vec![file("game/game.fun", 2, 1)],
            "t".into(),
        ));
        let d = decision(&task).unwrap();
        assert_eq!((d.size.files, d.depth), (1, Depth::Look));
        task.report = Some(report(&[], &[("functor test", true)]));
        assert_eq!(decision(&task).unwrap().depth, Depth::Glance);
        task.run = Some(crate::tasks::Run {
            id: 0,
            ..Default::default()
        });
        assert_eq!(decision(&task), None, "running");
    }
}
