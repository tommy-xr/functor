//! Voice from a headset (plan 4.3/4.4, D8): a transcribed utterance steers
//! the task running for the game on that headset, or starts one, and its
//! progress goes back to the headset as stages. The shell sends `Heard` and
//! `Transcribed` itself; these are the stages after.

use crate::devices::LinkCommand;
use crate::route::By;
use crate::layout::Placement;
use crate::state::{connected, AppState, Command};
use crate::tasks::{Item, Spoken};

/// After a run ends ok, how long the game has to reload with its edit
/// (the push follows the save) before the headset is told it did not;
/// counted only while the headset is linked, as `functor run vr` holds
/// the push for a headset that is away until it is back.
pub const RELOAD_GRACE_MS: u64 = 15_000;

/// A spoken request's clip as the shell sees it, before it has text
/// (`LinkEvent::Hearing`): the first steps of its record (`progress`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HearingStage {
    /// Its clip is arriving.
    Arriving,
    /// Its clip is whole and being turned into text.
    Transcribing,
    /// Cancelled in the headset.
    Cancelled,
    Failed { reason: String },
}

/// A transcribed utterance from a linked headset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Utterance {
    /// The headset's id.
    pub device: String,
    /// The headset's number for it.
    pub utterance: u64,
    pub text: String,
    pub spoken: Spoken,
    /// Who chose its place; the fixed rule until the router or the user does.
    pub by: By,
    /// The model router's title, for a new task.
    pub title: Option<String>,
    /// The project of the game it was said to: it goes nowhere else.
    pub project: String,
}

/// The stages the core reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stage {
    /// A run started or was steered with it.
    Working,
    /// The game on the headset reloaded after the run's edit.
    Reloaded,
    /// The run answered without changing the game: its answer's first line.
    Answered(String),
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
    /// Since when (`AppState::clock`) the run has ended ok with the
    /// headset linked.
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
    // A steered task's earlier utterance is superseded, not failed; one
    // placed elsewhere is still followed.
    state.voice.retain(|j| j.task != task);
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
/// it started; `Answered` if it ended ok with nothing to reload; `Failed`
/// if the run failed, was stopped, or the game left the headset or did not
/// reload its change in time. A headset that is away is waited
/// for: the shell delivers what it missed when it relinks.
pub(crate) fn progress(state: &mut AppState, commands: &mut Vec<Command>) {
    let mut jobs = std::mem::take(&mut state.voice);
    let mut ended = Vec::new();
    jobs.retain_mut(|job| {
        let mut end = |stage: Stage| {
            status(commands, &job.device, job.utterance, stage.clone());
            ended.push((job.device.clone(), job.utterance, stage));
            false
        };
        let revision = match on_headset(state, &job.device) {
            Some((session, revision)) if session == job.session => revision,
            _ => return end(Stage::Failed("the game stopped on the headset".to_string())),
        };
        let linked = connected(&state.devices, &job.device);
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
        let running = task.run.as_ref().is_some_and(|r| r.id == run);
        match (revision, job.revision) {
            (Some(now), None) => job.revision = Some(now),
            (Some(now), Some(then)) if now > then => job.reloaded = true,
            // A fresh load (its app restarted while away): counted from
            // here, and after the run it carries what the run saved.
            (Some(now), Some(then)) if now < then => {
                job.revision = Some(now);
                job.reloaded |= !running;
            }
            _ => {}
        }
        if running {
            return true;
        }
        match outcome(&task.transcript, run) {
            Some(Item::Finished { ok: true, .. }) if job.reloaded => end(Stage::Reloaded),
            Some(Item::Finished { ok: true, .. })
                if state.tracks_changes && task.change.as_ref().is_some_and(|c| c.is_empty()) =>
            {
                end(Stage::Answered(answer(&task.transcript, run)))
            }
            // The change is ready; it loads when the headset is back.
            Some(Item::Finished { ok: true, .. }) if !linked => {
                job.ended = None;
                true
            }
            Some(Item::Finished { ok: true, .. }) => {
                let ended = *job.ended.get_or_insert(state.clock);
                if state.clock.saturating_sub(ended) < RELOAD_GRACE_MS {
                    return true;
                }
                // A failure only when a change is known to be waiting;
                // otherwise (changes untracked) it is the agent's answer.
                match state.tracks_changes && crate::tasks::has_change(task) {
                    true => end(Stage::Failed(
                        "the agent finished, but the game did not reload".to_string(),
                    )),
                    false => end(Stage::Answered(answer(&task.transcript, run))),
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
    for (device, utterance, stage) in ended {
        crate::progress::voice_ended(state, &device, utterance, &stage);
    }
}

/// Run `run`'s answer, as a headset's toast says it: its last reply's
/// first line, or where to read it.
pub(crate) fn answer(items: &[Item], run: u64) -> String {
    let reply = items
        .iter()
        .skip(run as usize + 1)
        .take_while(|i| !matches!(i, Item::User { .. }))
        .filter_map(|i| match i {
            Item::Agent { text, .. } => text.lines().find(|l| !l.trim().is_empty()),
            _ => None,
        })
        .last();
    reply.map_or("Answered: see the conversation".to_string(), |l| crate::activity::cut(l.trim()))
}

/// How run `run` (its user message's index) of a transcript ended.
pub(crate) fn outcome(items: &[Item], run: u64) -> Option<&Item> {
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
