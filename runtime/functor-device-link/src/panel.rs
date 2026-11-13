//! The headset panel (D28): what the desktop shows in it, and what its user
//! may do from it. Both ride [`Frame::Channel`].
//!
//! - [`UPDATE_CHANNEL`] carries [`Update`], desktop → headset: the whole
//!   [`Panel`] each time it changes and once a link comes up, or why an
//!   action was refused.
//! - [`ACTION_CHANNEL`] carries [`Action`], headset → desktop: the few
//!   things the panel can do, each one the desktop UI can do too.
//!
//! Every message starts with [`VERSION`]. Text is cut to its limit and
//! cleaned on encode, so every update decodes; `decode` refuses another
//! version, a list over its limit, and trailing bytes. Big-endian, like
//! [`crate::wire`].

use crate::voice::tidy;
use crate::wire::{Frame, Reader, Writer};
use crate::Error;

/// The first byte of every panel message.
pub const VERSION: u8 = 1;

/// Desktop → headset: [`Update`] messages.
pub const UPDATE_CHANNEL: u16 = 3;
/// Headset → desktop: [`Action`] messages.
pub const ACTION_CHANNEL: u16 = 4;

/// Most tasks in a panel.
pub const MAX_TASKS: usize = 6;
/// Most notes on a decision page.
pub const MAX_NOTES: usize = 3;
/// Most bytes of each text; longer is cut.
pub const MAX_PROJECT: usize = 128;
pub const MAX_TITLE: usize = 96;
pub const MAX_ACTIVITY: usize = 96;
pub const MAX_TREE: usize = 64;
pub const MAX_SIZE: usize = 96;
pub const MAX_REASON: usize = 192;
pub const MAX_SUMMARY: usize = 640;
pub const MAX_NOTE: usize = 192;

/// What an agent is doing, as its character shows it (D34).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stance {
    Thinking,
    Working,
    /// The next move is the user's.
    Waiting,
    Done,
    Failed,
}

/// The panel's content for one headset.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Panel {
    /// The app's open project; `None` with no folder open.
    pub project: Option<String>,
    /// Its game is placed on this headset.
    pub placed: bool,
    /// The task in front of the user (one of `tasks`): the running one,
    /// else the latest waiting on the user, else the latest.
    pub now: Option<u64>,
    /// The project's tasks, `now` first, then those waiting on the user,
    /// then the most recent; at most [`MAX_TASKS`].
    pub tasks: Vec<Task>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Task {
    pub id: u64,
    pub title: String,
    pub stance: Stance,
    /// What it is doing ("reading game.fun"), its error, or empty.
    pub activity: String,
    /// Its change waits for Keep or Revert.
    pub decision: Option<Decision>,
}

/// A waiting change's first page (D35), as text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision {
    /// The change the user is shown, which Revert names.
    pub tree: String,
    /// "+12 -3, 1 file, game logic".
    pub size: String,
    /// Approve at a glance, or look into it.
    pub glance: bool,
    pub reason: String,
    /// One paragraph; empty when the agent gave none.
    pub summary: String,
    /// At most [`MAX_NOTES`].
    pub notes: Vec<String>,
}

/// Desktop → headset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Update {
    Panel(Panel),
    /// The desktop refused the headset's last action.
    Refused {
        reason: String,
    },
}

/// Headset → desktop: the user's choice in the panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Make `task` the desktop chat's current task.
    Select {
        task: u64,
    },
    Keep {
        task: u64,
    },
    /// `tree` is the [`Decision::tree`] the user saw.
    Revert {
        task: u64,
        tree: String,
    },
    /// Stop its run.
    Stop {
        task: u64,
    },
}

impl Stance {
    fn code(self) -> u8 {
        match self {
            Stance::Thinking => 1,
            Stance::Working => 2,
            Stance::Waiting => 3,
            Stance::Done => 4,
            Stance::Failed => 5,
        }
    }

    fn from_code(code: u8) -> Result<Stance, Error> {
        Ok(match code {
            1 => Stance::Thinking,
            2 => Stance::Working,
            3 => Stance::Waiting,
            4 => Stance::Done,
            5 => Stance::Failed,
            _ => return Err(Error::Malformed("panel stance")),
        })
    }
}

impl Update {
    /// The bytes: text cut and cleaned, lists cut to their limits.
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::default();
        w.u8(VERSION);
        match self {
            Update::Panel(p) => {
                w.u8(1);
                match &p.project {
                    Some(project) => {
                        w.u8(1);
                        w.str16(&tidy(project, MAX_PROJECT));
                    }
                    None => w.u8(0),
                }
                w.u8(p.placed.into());
                match p.now {
                    Some(id) => {
                        w.u8(1);
                        w.u64(id);
                    }
                    None => w.u8(0),
                }
                let tasks = &p.tasks[..p.tasks.len().min(MAX_TASKS)];
                w.u8(tasks.len() as u8);
                for t in tasks {
                    w.u64(t.id);
                    w.str16(&tidy(&t.title, MAX_TITLE));
                    w.u8(t.stance.code());
                    w.str16(&tidy(&t.activity, MAX_ACTIVITY));
                    match &t.decision {
                        Some(d) => {
                            w.u8(1);
                            w.str16(&tidy(&d.tree, MAX_TREE));
                            w.str16(&tidy(&d.size, MAX_SIZE));
                            w.u8(d.glance.into());
                            w.str16(&tidy(&d.reason, MAX_REASON));
                            w.str16(&tidy(&d.summary, MAX_SUMMARY));
                            let notes = &d.notes[..d.notes.len().min(MAX_NOTES)];
                            w.u8(notes.len() as u8);
                            for n in notes {
                                w.str16(&tidy(n, MAX_NOTE));
                            }
                        }
                        None => w.u8(0),
                    }
                }
            }
            Update::Refused { reason } => {
                w.u8(2);
                w.str16(&tidy(reason, MAX_REASON));
            }
        }
        w.0
    }

    /// As a frame on [`UPDATE_CHANNEL`].
    pub fn frame(&self) -> Frame {
        Frame::Channel {
            channel: UPDATE_CHANNEL,
            data: self.encode(),
        }
    }

    pub fn decode(bytes: &[u8]) -> Result<Update, Error> {
        let mut r = Reader::new(bytes);
        version(&mut r)?;
        let update = match r.u8()? {
            1 => {
                let project = match r.flag()? {
                    true => Some(r.text16(MAX_PROJECT)?),
                    false => None,
                };
                let placed = r.flag()?;
                let now = match r.flag()? {
                    true => Some(r.u64()?),
                    false => None,
                };
                let n = count(&mut r, MAX_TASKS)?;
                let mut tasks = Vec::with_capacity(n);
                for _ in 0..n {
                    tasks.push(Task {
                        id: r.u64()?,
                        title: r.text16(MAX_TITLE)?,
                        stance: Stance::from_code(r.u8()?)?,
                        activity: r.text16(MAX_ACTIVITY)?,
                        decision: match r.flag()? {
                            true => Some(Decision {
                                tree: r.text16(MAX_TREE)?,
                                size: r.text16(MAX_SIZE)?,
                                glance: r.flag()?,
                                reason: r.text16(MAX_REASON)?,
                                summary: r.text16(MAX_SUMMARY)?,
                                notes: (0..count(&mut r, MAX_NOTES)?)
                                    .map(|_| r.text16(MAX_NOTE))
                                    .collect::<Result<_, _>>()?,
                            }),
                            false => None,
                        },
                    });
                }
                Update::Panel(Panel {
                    project,
                    placed,
                    now,
                    tasks,
                })
            }
            2 => Update::Refused {
                reason: r.text16(MAX_REASON)?,
            },
            _ => return Err(Error::Malformed("panel update kind")),
        };
        r.end()?;
        Ok(update)
    }
}

impl Action {
    pub fn task(&self) -> u64 {
        match self {
            Action::Select { task }
            | Action::Keep { task }
            | Action::Revert { task, .. }
            | Action::Stop { task } => *task,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::default();
        w.u8(VERSION);
        match self {
            Action::Select { task } => {
                w.u8(1);
                w.u64(*task);
            }
            Action::Keep { task } => {
                w.u8(2);
                w.u64(*task);
            }
            Action::Revert { task, tree } => {
                w.u8(3);
                w.u64(*task);
                w.str16(&tidy(tree, MAX_TREE));
            }
            Action::Stop { task } => {
                w.u8(4);
                w.u64(*task);
            }
        }
        w.0
    }

    pub fn decode(bytes: &[u8]) -> Result<Action, Error> {
        let mut r = Reader::new(bytes);
        version(&mut r)?;
        let action = match r.u8()? {
            1 => Action::Select { task: r.u64()? },
            2 => Action::Keep { task: r.u64()? },
            3 => Action::Revert {
                task: r.u64()?,
                tree: r.text16(MAX_TREE)?,
            },
            4 => Action::Stop { task: r.u64()? },
            _ => return Err(Error::Malformed("panel action")),
        };
        r.end()?;
        Ok(action)
    }
}

fn count(r: &mut Reader, max: usize) -> Result<usize, Error> {
    match usize::from(r.u8()?) {
        n if n > max => Err(Error::TooLarge),
        n => Ok(n),
    }
}

/// Another version is `Malformed("panel version")`: this channel's format,
/// not the link's.
fn version(r: &mut Reader) -> Result<(), Error> {
    match r.u8()? {
        VERSION => Ok(()),
        _ => Err(Error::Malformed("panel version")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: u64, decision: Option<Decision>) -> Task {
        Task {
            id,
            title: "Make the ball red".into(),
            stance: Stance::Waiting,
            activity: "reading game.fun".into(),
            decision,
        }
    }

    fn decision() -> Decision {
        Decision {
            tree: "4b825dc6".into(),
            size: "+1 -1, 1 file, game logic".into(),
            glance: true,
            reason: "1 changed line in one file. 1 check passes.".into(),
            summary: "The agent changed one colour value.".into(),
            notes: vec!["Not tried on the left hand.".into()],
        }
    }

    #[test]
    fn every_message_round_trips() {
        let updates = [
            Update::Panel(Panel::default()),
            Update::Panel(Panel {
                project: Some("vr-lab".into()),
                placed: true,
                now: Some(7),
                tasks: vec![task(7, Some(decision())), task(u64::MAX, None)],
            }),
            Update::Refused {
                reason: "task 7 is still running".into(),
            },
        ];
        for update in updates {
            assert_eq!(Update::decode(&update.encode()).unwrap(), update);
        }
        let actions = [
            Action::Select { task: 1 },
            Action::Keep { task: 2 },
            Action::Revert {
                task: 3,
                tree: "abc".into(),
            },
            Action::Stop { task: u64::MAX },
        ];
        for action in actions {
            assert_eq!(Action::decode(&action.encode()).unwrap(), action);
        }
    }

    #[test]
    fn a_full_panel_is_cut_to_its_limits_and_fits_one_message() {
        let long = |n: usize| "é\u{202e}\n".repeat(n);
        let big = Decision {
            tree: long(100),
            size: long(100),
            glance: false,
            reason: long(1000),
            summary: long(1000),
            notes: vec![long(1000); 9],
        };
        let mut t = task(1, Some(big));
        t.title = long(1000);
        t.activity = long(1000);
        let panel = Panel {
            project: Some(long(1000)),
            placed: false,
            now: Some(1),
            tasks: vec![t; 20],
        };
        let bytes = Update::Panel(panel).encode();
        // Noise adds a 16-byte tag; leave room for it and the frame.
        assert!(bytes.len() + 64 < crate::MAX_MESSAGE, "{}", bytes.len());
        let Update::Panel(back) = Update::decode(&bytes).unwrap() else {
            panic!("a panel");
        };
        assert_eq!(back.tasks.len(), MAX_TASKS);
        let d = back.tasks[0].decision.as_ref().unwrap();
        assert_eq!(d.notes.len(), MAX_NOTES);
        assert!(d.summary.len() <= MAX_SUMMARY && back.tasks[0].title.len() <= MAX_TITLE);
        assert!(!back.tasks[0].title.contains(['\n', '\u{202e}']));
    }

    #[test]
    fn decoding_refuses_versions_kinds_lists_and_trailing_bytes() {
        let mut bytes = Action::Keep { task: 1 }.encode();
        bytes[0] = 2;
        assert_eq!(
            Action::decode(&bytes),
            Err(Error::Malformed("panel version"))
        );
        assert!(Action::decode(&[VERSION, 9]).is_err());
        assert!(Update::decode(&[VERSION, 9]).is_err());
        assert!(Update::decode(&[]).is_err());
        let mut trailing = Action::Stop { task: 1 }.encode();
        trailing.push(0);
        assert_eq!(
            Action::decode(&trailing),
            Err(Error::Malformed("trailing bytes"))
        );
        // A forged task count over the limit.
        let mut many = Update::Panel(Panel::default()).encode();
        *many.last_mut().unwrap() = MAX_TASKS as u8 + 1;
        assert_eq!(Update::decode(&many), Err(Error::TooLarge));
        // A forged stance.
        let mut bytes = Update::Panel(Panel {
            tasks: vec![task(1, None)],
            ..Panel::default()
        })
        .encode();
        let stance = bytes.len() - 1 - 2 - "reading game.fun".len() - 1;
        bytes[stance] = 9;
        assert_eq!(
            Update::decode(&bytes),
            Err(Error::Malformed("panel stance"))
        );
    }
}
