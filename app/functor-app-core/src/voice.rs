//! Voice from a headset (plan 4.3/4.4, D8): a transcribed utterance steers
//! the task running for the game on that headset, or starts one, and its
//! progress goes back to the headset as stages. The shell sends `Heard` and
//! `Transcribed` itself; these are the stages after.

use crate::devices::LinkCommand;
use crate::layout::Placement;
use crate::state::{connected, AppState, Command};
use crate::tasks::{Item, Spoken};

/// After a run ends ok, how long the game has to reload with its edit
/// (the push follows the save) before the headset is told it did not.
pub const RELOAD_GRACE_MS: u64 = 15_000;

/// A transcribed utterance from a linked headset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Utterance {
    /// The headset's id.
    pub device: String,
    /// The headset's number for it.
    pub utterance: u64,
    pub text: String,
    pub spoken: Spoken,
}

/// The stages the core reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stage {
    /// A run started or was steered with it.
    Working,
    /// The game on the headset reloaded after the run's edit.
    Reloaded,
    Failed(String),
}

/// An utterance whose run is being followed until the game reloads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub device: String,
    pub utterance: u64,
    pub task: u64,
    /// `None` while the task waits its turn in the queue (D30).
    pub run: Option<u64>,
    /// The game session it reloads: another one (a restart, a project
    /// switch) is not this run's game.
    session: u64,
    /// The game's program revision when the run started (`None`: not
    /// polled yet; the first poll sets it).
    revision: Option<u64>,
    reloaded: bool,
    /// When (`AppState::clock`) the run ended ok.
    ended: Option<u64>,
}

/// Where an utterance goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    /// Stop this running task and resume it with the utterance (D29's steer).
    Steer(u64),
    /// A new task in the active project, queued while it is busy (D30).
    New,
    /// Nowhere, for this reason.
    Nowhere(String),
}

/// Where `text` from `device` goes: into the task running for the game on
/// that headset, else a new task in that game's project (started, or queued
/// behind a decision or a revert). The router's fixed rule (D29).
pub fn route(state: &AppState, device: &str, text: &str) -> Route {
    let Some(project) = state
        .active
        .as_deref()
        .filter(|_| on_headset(state, device).is_some())
    else {
        return Route::Nowhere(
            "This headset is not running a game from the Functor app, so there is nothing to \
change. Choose it as the game's place on your computer."
                .to_string(),
        );
    };
    if text.trim().is_empty() {
        return Route::Nowhere("the message is empty".to_string());
    }
    match state
        .tasks
        .iter()
        .find(|t| t.project == project && t.run.is_some())
    {
        Some(task) => Route::Steer(task.id),
        None => Route::New,
    }
}

/// The app's game session running on `device`, if one is: its id and
/// program revision (`None` until polled). The active project's game is
/// placed there and the session is live.
fn on_headset(state: &AppState, device: &str) -> Option<(u64, Option<u64>)> {
    let project = state.active.as_deref()?;
    let game = &state.layouts.get(project)?.game;
    if game.placement != Placement::Headset || game.headset.as_deref() != Some(device) {
        return None;
    }
    let session = state.session.as_ref().filter(|s| s.status.is_live())?;
    Some((session.id, session.status.revision()))
}

pub(crate) fn status(commands: &mut Vec<Command>, device: &str, utterance: u64, stage: Stage) {
    commands.push(Command::Link(LinkCommand::Voice {
        device: device.to_string(),
        utterance,
        stage,
    }));
}

/// Follows `task`'s run, started for `u` (`Working` now), or the one it
/// starts when its turn comes, if queued (`Working` then).
pub(crate) fn follow(state: &mut AppState, u: &Utterance, task: u64, commands: &mut Vec<Command>) {
    let Some(t) = state.tasks.iter().find(|t| t.id == task) else {
        return;
    };
    let run = t.run.as_ref().map(|r| r.id);
    let Some((session, revision)) = on_headset(state, &u.device) else {
        return;
    };
    // A steered task's earlier utterance is superseded, not failed.
    state
        .voice
        .retain(|j| j.task != task && j.device != u.device);
    state.voice.push(Job {
        device: u.device.clone(),
        utterance: u.utterance,
        task,
        run,
        session,
        revision,
        reloaded: false,
        ended: None,
    });
    if run.is_some() {
        status(commands, &u.device, u.utterance, Stage::Working);
    }
}

/// After every event: each followed run's outcome, once known, goes to its
/// headset — `Reloaded` once the run ended ok and the game reloaded since
/// it started; `Failed` if the run failed, was stopped, or the game left
/// the headset or did not reload in time.
pub(crate) fn progress(state: &mut AppState, commands: &mut Vec<Command>) {
    let mut jobs = std::mem::take(&mut state.voice);
    jobs.retain_mut(|job| {
        let mut end = |stage| {
            status(commands, &job.device, job.utterance, stage);
            false
        };
        let revision = match on_headset(state, &job.device) {
            Some((session, revision)) if session == job.session => revision,
            _ => return end(Stage::Failed("the game stopped on the headset".to_string())),
        };
        if !connected(&state.devices, &job.device) {
            return false;
        }
        let Some(task) = state.tasks.iter().find(|t| t.id == job.task) else {
            return end(Stage::Failed("the task was deleted".to_string()));
        };
        // Queued: followed from its start, its reloads counted from then.
        let run = match (job.run, &task.run) {
            (Some(run), _) => run,
            (None, _) if task.queued.is_some() => return true,
            (None, Some(run)) => {
                job.run = Some(run.id);
                job.revision = revision;
                status(commands, &job.device, job.utterance, Stage::Working);
                return true;
            }
            (None, None) => return end(Stage::Failed("the task did not start".to_string())),
        };
        match (revision, job.revision) {
            (Some(now), None) => job.revision = Some(now),
            (Some(now), Some(then)) if now > then => job.reloaded = true,
            _ => {}
        }
        if task.run.as_ref().is_some_and(|r| r.id == run) {
            return true;
        }
        match outcome(&task.transcript, run) {
            Some(Item::Finished { ok: true, .. }) if job.reloaded => end(Stage::Reloaded),
            Some(Item::Finished { ok: true, .. })
                if state.tracks_changes && task.change.as_ref().is_some_and(|c| c.is_empty()) =>
            {
                end(Stage::Failed(
                    "the agent answered without changing the game: see the computer".to_string(),
                ))
            }
            Some(Item::Finished { ok: true, .. }) => {
                let ended = *job.ended.get_or_insert(state.clock);
                match state.clock.saturating_sub(ended) >= RELOAD_GRACE_MS {
                    true => end(Stage::Failed(
                        "the agent finished, but the game did not reload".to_string(),
                    )),
                    false => true,
                }
            }
            Some(Item::Finished { ok: false, summary }) => end(Stage::Failed(
                summary
                    .clone()
                    .unwrap_or_else(|| "the agent failed".to_string()),
            )),
            Some(Item::Stopped) => end(Stage::Failed("stopped".to_string())),
            _ => end(Stage::Failed("the run ended".to_string())),
        }
    });
    state.voice = jobs;
}

/// How run `run` (its user message's index) of a transcript ended.
fn outcome(items: &[Item], run: u64) -> Option<&Item> {
    items
        .iter()
        .skip(run as usize + 1)
        .take_while(|i| !matches!(i, Item::User { .. }))
        .find(|i| matches!(i, Item::Finished { .. } | Item::Stopped))
}

/// What the agent is sent for a spoken message: the text, after where and
/// how it was said.
pub fn prompt(text: &str, spoken: &Spoken) -> String {
    let context = spoken
        .context
        .as_deref()
        .map_or(String::new(), |c| format!(" {c}"));
    format!(
        "(Said aloud in the headset ({}){context}. Transcribed by speech-to-text, so a word may \
be misheard.)\n\n{text}",
        spoken.headset
    )
}
