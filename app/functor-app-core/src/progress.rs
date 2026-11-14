//! Every request shows where it is (D43): one record per request, from its
//! first sign to its outcome, held here so the chat, the headset's toast
//! and its panel read the same thing. A spoken request starts when its clip
//! starts arriving; a typed one at `Processing`. The agent's live activity
//! is `AppState::activity[task]` (D34).

use serde::{Deserialize, Serialize};

use crate::state::{connected, AppState};
use crate::tasks::{Item, Task};
use crate::voice::{self, Stage};

/// How long a request stays after its outcome.
pub const KEPT_MS: u64 = 60_000;
/// The most kept.
const MAX: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Request {
    pub id: u64,
    pub project: String,
    /// The headset it was said in, by name; `None` when typed.
    pub headset: Option<String>,
    /// What was said (once transcribed) or typed.
    pub text: Option<String>,
    /// The task it went to, once placed.
    pub task: Option<u64>,
    /// Where it went: "Started a new task", "Steered “…”".
    pub line: Option<String>,
    /// The steps reached so far, each with when (`AppState::now`, wall ms):
    /// a done step took until the next one began.
    pub steps: Vec<Reached>,
    pub outcome: Option<Outcome>,
    /// When (wall ms) its outcome was last set.
    pub ended: Option<u64>,
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub(crate) device: Option<String>,
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub(crate) utterance: Option<u64>,
    /// Its message's place in the task's transcript.
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub(crate) item: Option<usize>,
    /// The run that answers it, once started.
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub(crate) run: Option<u64>,
    /// When (`AppState::clock`) it ended, for dropping it.
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub(crate) ended_clock: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename = "RequestReached"))]
pub struct Reached {
    pub step: Step,
    pub at: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename = "RequestStep"))]
#[serde(rename_all = "snake_case")]
pub enum Step {
    /// Its clip is arriving.
    Received,
    /// Its clip is whole and being turned into text.
    Transcribing,
    /// Its text is known; it is being placed.
    Transcribed,
    /// Placed in a task: the agent is starting (or it waits its turn).
    Processing,
    /// The agent is working on it.
    Agent,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename = "RequestOutcome"))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Outcome {
    /// The agent answered without a change: its answer's first line.
    Answered { text: String },
    /// It changed the game.
    Changed { reload: Reload },
    /// Its change waits for the user to keep or revert it.
    NeedsYou,
    /// It ended; with changes untracked, whether it changed anything is
    /// not known.
    Done,
    /// Cancelled in the headset.
    Cancelled,
    /// It failed at step `at`.
    Failed { at: Step, reason: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename = "RequestReload"))]
#[serde(rename_all = "snake_case")]
pub enum Reload {
    /// Waiting for the headset's game to reload.
    Waiting,
    /// The headset is away: it loads when it is back.
    WhenBack,
    Reloaded,
}

fn reach(r: &mut Request, step: Step, now: u64) {
    if r.steps.last().is_none_or(|s| s.step < step) {
        r.steps.push(Reached { step, at: now });
    }
}

fn end(r: &mut Request, outcome: Outcome, state_now: u64, clock: u64) {
    if r.outcome.as_ref() != Some(&outcome) {
        r.outcome = Some(outcome);
        r.ended = Some(state_now);
        r.ended_clock = clock;
    }
}

/// The last step `r` reached.
fn at(r: &Request) -> Step {
    r.steps.last().map_or(Step::Received, |s| s.step)
}

fn spoken<'a>(state: &'a mut AppState, device: &str, utterance: u64) -> Option<&'a mut Request> {
    let found = |r: &&mut Request| {
        r.device.as_deref() == Some(device) && r.utterance == Some(utterance)
    };
    state.requests.iter_mut().find(found)
}

/// Spoken request `utterance` from `device` reached `step` (its clip, from
/// the shell): a new record at its first sign.
pub(crate) fn heard(state: &mut AppState, device: &str, utterance: u64, step: Step) {
    let now = state.now;
    if let Some(r) = spoken(state, device, utterance) {
        return reach(r, step, now);
    }
    let headset = state.devices.list.iter().find(|d| d.id == device);
    let Some(headset) = headset.map(|d| d.name.clone()) else {
        return;
    };
    let project = state.active.clone().unwrap_or_default();
    let mut r = new(state, project, Some(headset));
    r.device = Some(device.to_string());
    r.utterance = Some(utterance);
    r.steps = [Step::Received, step]
        .iter()
        .filter(|s| **s <= step)
        .map(|&step| Reached { step, at: now })
        .collect();
    r.steps.dedup_by_key(|s| s.step);
    state.requests.push(r);
}

/// Its text: transcribed.
pub(crate) fn transcribed(state: &mut AppState, device: &str, utterance: u64, text: &str) {
    heard(state, device, utterance, Step::Transcribed);
    if let Some(r) = spoken(state, device, utterance) {
        r.text = Some(text.to_string());
    }
}

/// It ended before it became a message: cancelled (`None`) or failed.
pub(crate) fn stopped(state: &mut AppState, device: &str, utterance: u64, reason: Option<&str>) {
    let (now, clock) = (state.now, state.clock);
    if let Some(r) = spoken(state, device, utterance).filter(|r| r.outcome.is_none()) {
        let outcome = match reason {
            None => Outcome::Cancelled,
            Some(reason) => Outcome::Failed {
                at: at(r),
                reason: reason.to_string(),
            },
        };
        end(r, outcome, now, clock);
    }
}

/// It landed as message `item` of `task`, which `line` says.
pub(crate) fn placed(state: &mut AppState, device: &str, utterance: u64, task: u64, item: usize, line: &str) {
    let now = state.now;
    if let Some(r) = spoken(state, device, utterance) {
        r.task = Some(task);
        r.item = Some(item);
        r.line = Some(line.to_string());
        // Moved by the user: placed again, its outcome not known yet.
        r.outcome = None;
        reach(r, Step::Processing, now);
    }
}

/// A typed message, the last of task `id`, entering at `Processing`.
pub(crate) fn typed(state: &mut AppState, id: u64) {
    let Some(task) = state.tasks.iter().find(|t| t.id == id) else {
        return;
    };
    let Some((item, text)) = last_message(task) else {
        return;
    };
    let (project, now) = (task.project.clone(), state.now);
    let mut r = new(state, project, None);
    r.text = Some(text);
    r.task = Some(id);
    r.item = Some(item);
    r.steps = vec![Reached {
        step: Step::Processing,
        at: now,
    }];
    state.requests.push(r);
}

fn last_message(task: &Task) -> Option<(usize, String)> {
    task.transcript.iter().enumerate().rev().find_map(|(i, item)| match item {
        Item::User { text, .. } => Some((i, text.clone())),
        _ => None,
    })
}

fn new(state: &mut AppState, project: String, headset: Option<String>) -> Request {
    state.request_seq += 1;
    Request {
        id: state.request_seq,
        project,
        headset,
        text: None,
        task: None,
        line: None,
        steps: Vec::new(),
        outcome: None,
        ended: None,
        device: None,
        utterance: None,
        item: None,
        run: None,
        ended_clock: 0,
    }
}

/// A followed spoken run ended with `stage` on its headset.
pub(crate) fn voice_ended(state: &mut AppState, device: &str, utterance: u64, stage: &Stage) {
    let (now, clock) = (state.now, state.clock);
    if let Some(r) = spoken(state, device, utterance) {
        let outcome = match stage {
            Stage::Working => return,
            Stage::Reloaded => Outcome::Changed {
                reload: Reload::Reloaded,
            },
            Stage::Answered(text) => Outcome::Answered { text: text.clone() },
            Stage::Failed(reason) => Outcome::Failed {
                at: at(r),
                reason: reason.clone(),
            },
        };
        end(r, outcome, now, clock);
    }
}

/// After every event: each placed request's agent step and outcome from
/// its task, and old ones dropped; whether any changed.
pub(crate) fn refresh(state: &mut AppState) -> bool {
    let mut requests = std::mem::take(&mut state.requests);
    let before = requests.clone();
    let (now, clock) = (state.now, state.clock);
    for r in requests.iter_mut() {
        // Settled, unless it waits for the headset's reload.
        let waiting = matches!(r.outcome, Some(Outcome::Changed { reload: Reload::Waiting | Reload::WhenBack }));
        if r.outcome.is_some() && !waiting {
            continue;
        }
        let (Some(id), Some(item)) = (r.task, r.item) else {
            continue;
        };
        let Some(task) = state.tasks.iter().find(|t| t.id == id) else {
            continue;
        };
        // A typed one's place (a spoken one's is the router's line).
        if r.device.is_none() && r.run.is_none() {
            let line = match task.queued {
                Some(_) => "Queued: it starts in its turn",
                None => "Starting the agent",
            };
            r.line = Some(line.to_string());
        }
        // The run that answers it: the first to start at or after it.
        match &task.run {
            Some(run) if run.id as usize >= item && r.run.is_none_or(|r| r == run.id) => {
                r.run = Some(run.id);
                if run.started {
                    reach(r, Step::Agent, now);
                }
                continue;
            }
            _ => {}
        }
        let Some(run) = r.run else { continue };
        let job = state.voice.iter().any(|j| {
            Some(&j.device) == r.device.as_ref() && Some(j.utterance) == r.utterance
        });
        if let Some(outcome) = outcome(state, task, run, r, job) {
            end(r, outcome, now, clock);
        }
    }
    requests.retain(|r| r.outcome.is_none() || clock.saturating_sub(r.ended_clock) < KEPT_MS);
    let over = requests.len().saturating_sub(MAX);
    requests.drain(..over);
    let changed = requests != before;
    state.requests = requests;
    changed
}

/// Run `run`'s outcome for `r` once it ended, unless a followed spoken run
/// (`job`) still waits for its reload: then where that stands.
fn outcome(state: &AppState, task: &Task, run: u64, r: &Request, job: bool) -> Option<Outcome> {
    let failed = |reason: String| Outcome::Failed {
        at: Step::Agent,
        reason,
    };
    match voice::outcome(&task.transcript, run)? {
        Item::Finished { ok: true, .. } if job => {
            let device = r.device.as_deref().unwrap_or_default();
            let reload = match connected(&state.devices, device) {
                true => Reload::Waiting,
                false => Reload::WhenBack,
            };
            Some(Outcome::Changed { reload })
        }
        // A followed spoken one's end came from its job ([`voice_ended`]);
        // one not followed (superseded by a steer) ends as a typed one.
        Item::Finished { ok: true, .. } if !state.tracks_changes => Some(Outcome::Done),
        Item::Finished { ok: true, .. } => match &task.change {
            Some(change) if change.is_empty() => Some(Outcome::Answered {
                text: voice::answer(&task.transcript, run),
            }),
            Some(_) => Some(Outcome::NeedsYou),
            None => None,
        },
        Item::Finished { ok: false, summary } => Some(failed(
            summary.clone().unwrap_or_else(|| "the agent failed".to_string()),
        )),
        Item::Stopped => Some(failed("stopped".to_string())),
        _ => None,
    }
}
