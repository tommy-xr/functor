//! The headset's load status: one short card saying what the shell is doing
//! while there is nothing to look at yet (waiting for a desktop, receiving a
//! project, checking its code, loading its assets), while a reload or an
//! asset load outlasts [`QUIET`], and when a load fails. Shell-owned, like the
//! reload indicator: a pure function of what the shell knows each frame.

use std::time::{Duration, Instant};

use crate::link_overlay::{Line, BODY, CYAN, HEADING, PINK, WHITE};
use crate::overlay_font;

/// Work over a running game shows only once it has taken this long.
pub const QUIET: Duration = Duration::from_millis(150);
/// Uploads this far apart are separate pushes. Longer than one part of a
/// large file takes over a slow link (8 MiB at ~2 MB/s over Wi-Fi).
const BURST: Duration = Duration::from_secs(20);
/// Files arriving for a running game show once they have for this long: an
/// edit's source arrives just before its reload.
const SYNC: Duration = Duration::from_millis(500);
/// How long "RUNNING" shows, and its fade out.
const RAN: Duration = Duration::from_millis(1500);
const FADE: Duration = Duration::from_millis(300);
/// How long a failure shows over a running game (it stays over none).
const FAILED: Duration = Duration::from_secs(10);
/// A launched game with nothing to load says so after this.
const SETTLE: Duration = Duration::from_millis(300);
/// Frames this far apart had a doze between them: what arrived meanwhile
/// is not arriving now.
const DOZE: Duration = Duration::from_secs(1);

/// What the shell knows this frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Facts {
    /// The paired desktop's name, and whether its link is up.
    pub desktop: Option<String>,
    pub linked: bool,
    /// A pushed game is running (not the boot scene).
    pub game: bool,
    /// Project files and bytes received since the process started.
    pub files: u64,
    pub bytes: u64,
    /// The files and bytes the push in flight said it will send.
    pub announced: Option<(u64, u64)>,
    /// A `/load-project` is in flight.
    pub loading: bool,
    /// A pushed reload is in flight.
    pub reloading: bool,
    /// Assets settled (loaded or failed) of those requested, since the
    /// process started.
    pub assets: (usize, usize),
}

/// What the card says.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// No desktop is linked.
    Waiting,
    /// Linked, with nothing pushed yet.
    Connected(Option<String>),
    /// Files of this push so far, and of how many when the sender said.
    Receiving { files: u64, bytes: u64, of: Option<(u64, u64)> },
    Checking,
    Assets { settled: usize, total: usize },
    Running,
    Reloading,
    Failed(String),
}

/// A push's uploads: what had arrived before it, and when it began and last grew.
struct Burst {
    files: u64,
    bytes: u64,
    began: Instant,
    last: Instant,
}

#[derive(Default)]
pub struct LoadStatus {
    seen: Facts,
    stepped: Option<Instant>,
    /// The asset counts when the running game launched: the card counts from there.
    base: (usize, usize),
    burst: Option<Burst>,
    reloading_since: Option<Instant>,
    assets_since: Option<Instant>,
    /// A load landed and its game's assets have yet to settle.
    launched: Option<Instant>,
    ran: Option<Instant>,
    failed: Option<(String, Instant)>,
}

impl LoadStatus {
    /// A load or reload was rejected.
    pub fn fail(&mut self, message: String, now: Instant) {
        self.failed = Some((message, now));
        self.launched = None;
        self.burst = None;
    }

    /// A `/load-project` landed: its game's first frames ask for its assets.
    pub fn launched(&mut self, now: Instant) {
        self.succeeded();
        self.launched = Some(now);
        self.base = self.seen.assets;
    }

    /// A load or reload was accepted: its files are in, and an earlier
    /// failure is stale.
    pub fn succeeded(&mut self) {
        self.failed = None;
        self.burst = None;
    }

    /// The card for this frame and its opacity; `None` when hidden.
    pub fn step(&mut self, facts: &Facts, now: Instant) -> Option<(Step, f32)> {
        let seen = std::mem::replace(&mut self.seen, facts.clone());
        let since = |from: Instant| now.saturating_duration_since(from);

        let dozed = self.stepped.replace(now).is_some_and(|last| since(last) > DOZE);
        if (facts.files, facts.bytes) != (seen.files, seen.bytes) && !dozed {
            let burst = self.burst.get_or_insert(Burst { files: seen.files, bytes: seen.bytes, began: now, last: now });
            burst.last = now;
        }
        if self.burst.as_ref().is_some_and(|burst| since(burst.last) > BURST) {
            self.burst = None;
        }
        let started = (facts.loading && !seen.loading) || (facts.reloading && !seen.reloading);
        if started {
            // The push's files are in; an older failure is stale.
            self.burst = None;
            self.failed = None;
            self.ran = None;
        }
        self.reloading_since = match facts.reloading {
            true => self.reloading_since.or(Some(now)),
            false => None,
        };
        let pending = facts.assets.0 < facts.assets.1;
        self.assets_since = match pending {
            true => self.assets_since.or(Some(now)),
            false => None,
        };

        if facts.loading {
            return Some((Step::Checking, 1.0));
        }
        if self.reloading_since.is_some_and(|from| since(from) >= QUIET) {
            return Some((Step::Reloading, 1.0));
        }
        if let Some(burst) = self.burst.as_ref().filter(|burst| !facts.game || since(burst.began) >= SYNC) {
            let (files, bytes) = (facts.files - burst.files, facts.bytes - burst.bytes);
            return Some((Step::Receiving { files, bytes, of: facts.announced }, 1.0));
        }
        if let Some((message, at)) = &self.failed {
            if !facts.game || since(*at) < FAILED {
                return Some((Step::Failed(message.clone()), 1.0));
            }
            self.failed = None;
        }
        let assets = Step::Assets {
            settled: facts.assets.0.saturating_sub(self.base.0),
            total: facts.assets.1.saturating_sub(self.base.1),
        };
        if let Some(launched) = self.launched {
            // The new game's first frames ask for its assets.
            if pending {
                return Some((assets, 1.0));
            }
            if since(launched) < SETTLE && self.assets_since.is_none() && seen.assets == facts.assets {
                return Some((Step::Checking, 1.0));
            }
            self.launched = None;
            self.ran = Some(now);
        }
        if self.assets_since.is_some_and(|from| since(from) >= QUIET) {
            return Some((assets, 1.0));
        }
        if let Some(left) = self.ran.and_then(|ran| RAN.checked_sub(since(ran))) {
            return Some((Step::Running, (left.as_secs_f32() / FADE.as_secs_f32()).min(1.0)));
        }
        self.ran = None;
        if facts.game {
            return None;
        }
        Some(match facts.linked {
            true => (Step::Connected(facts.desktop.clone()), 1.0),
            false => (Step::Waiting, 1.0),
        })
    }
}

/// A failure's line width in metres, and its most lines.
const WRAP: f32 = 0.5;
const WRAP_LINES: usize = 6;

/// The card's lines.
pub fn lines(step: &Step) -> Vec<Line> {
    let line = |text: &str, size, color| Line { text: text.to_string(), size, color };
    let card = |heading: &str, body: &str| vec![line(heading, HEADING, CYAN), line(body, BODY, WHITE)];
    match step {
        Step::Waiting => card("WAITING FOR A DESKTOP", "Open Functor on your computer"),
        Step::Connected(Some(desktop)) => card("CONNECTED", &format!("to {desktop}. Run a project to see it here")),
        Step::Connected(None) => card("CONNECTED", "Run a project to see it here"),
        Step::Receiving { files, bytes, of } => {
            let mb = |bytes: u64| bytes as f64 / (1024.0 * 1024.0);
            let count = |n: u64| if n == 1 { "1 file".to_string() } else { format!("{n} files") };
            let body = match of {
                // Envelope framing counts a little over the files' own bytes.
                Some((all, total)) if *total > 0 => {
                    let bytes = (*bytes).min(*total);
                    let percent = bytes * 100 / total;
                    format!("{:.1} of {:.1} MB ({percent}%), {} of {}", mb(bytes), mb(*total), (*files).min(*all), count(*all))
                }
                _ => format!("{}, {:.1} MB", count(*files), mb(*bytes)),
            };
            card("RECEIVING PROJECT", &body)
        }
        Step::Checking => card("STARTING", "Checking code"),
        Step::Assets { settled, total } => card("LOADING ASSETS", &format!("{settled} of {total}")),
        Step::Running => vec![line("RUNNING", HEADING, CYAN)],
        Step::Reloading => vec![line("RELOADING...", HEADING, CYAN)],
        Step::Failed(message) => std::iter::once(line("LOAD FAILED", HEADING, PINK))
            .chain(overlay_font::wrap(message, BODY, WRAP, WRAP_LINES).iter().map(|text| line(text, BODY, WHITE)))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The steps shown over `frames`: each a time in ms and that frame's facts.
    fn run(status: &mut LoadStatus, start: Instant, frames: &[(u64, Facts)]) -> Vec<Option<Step>> {
        frames
            .iter()
            .map(|(ms, facts)| status.step(facts, start + Duration::from_millis(*ms)).map(|(step, _)| step))
            .collect()
    }

    #[test]
    fn a_first_load_walks_every_step() {
        let start = Instant::now();
        let idle = Facts::default();
        let linked = Facts { desktop: Some("Studio".into()), linked: true, ..idle.clone() };
        let one = Facts { files: 1, bytes: 3 << 20, ..linked.clone() };
        let two = Facts { files: 2, bytes: 50 << 20, ..linked.clone() };
        let loading = Facts { loading: true, ..two.clone() };
        let landed = Facts { game: true, ..two.clone() };
        let asking = Facts { assets: (0, 4), ..landed.clone() };
        let some = Facts { assets: (3, 4), ..landed.clone() };
        let all = Facts { assets: (4, 4), ..landed.clone() };
        let mut status = LoadStatus::default();
        let mut shown = run(
            &mut status,
            start,
            &[
                (0, idle),
                (10, linked),
                (20, one),
                // Part of a large file: bytes without a file.
                (30, Facts { bytes: 20 << 20, ..one_file() }),
                (40, two),
                (50, loading),
            ],
        );
        status.launched(start + Duration::from_millis(55));
        let frames = [(60, landed), (70, asking), (900, some), (1000, all.clone()), (2000, all.clone()), (2600, all)];
        shown.extend(run(&mut status, start, &frames));
        let receiving = |files, mb: u64| Some(Step::Receiving { files, bytes: mb << 20, of: None });
        assert_eq!(
            shown,
            vec![
                Some(Step::Waiting),
                Some(Step::Connected(Some("Studio".into()))),
                receiving(1, 3),
                receiving(1, 20),
                receiving(2, 50),
                Some(Step::Checking),
                Some(Step::Checking),
                Some(Step::Assets { settled: 0, total: 4 }),
                Some(Step::Assets { settled: 3, total: 4 }),
                Some(Step::Running),
                Some(Step::Running),
                None,
            ]
        );
    }

    fn one_file() -> Facts {
        Facts { desktop: Some("Studio".into()), linked: true, files: 1, ..Facts::default() }
    }

    #[test]
    fn a_game_without_assets_runs_once_its_first_frames_ask_for_none() {
        let start = Instant::now();
        let loading = Facts { loading: true, ..Facts::default() };
        let landed = Facts { game: true, ..Facts::default() };
        let mut status = LoadStatus::default();
        assert_eq!(run(&mut status, start, &[(0, loading)]), vec![Some(Step::Checking)]);
        status.launched(start + Duration::from_millis(5));
        let shown = run(&mut status, start, &[(10, landed.clone()), (200, landed.clone()), (400, landed)]);
        assert_eq!(shown, vec![Some(Step::Checking), Some(Step::Checking), Some(Step::Running)]);
    }

    #[test]
    fn work_over_a_running_game_shows_only_once_it_outlasts_quiet() {
        let start = Instant::now();
        let mut status = LoadStatus::default();
        let game = |files, bytes| Facts { game: true, files, bytes, assets: (2, 2), ..Facts::default() };
        // An edit's source and its quick reload show nothing: the reload
        // seen in flight, or answered between two frames.
        let reloading = Facts { reloading: true, ..game(1, 100) };
        let shown = run(&mut status, start, &[(0, game(0, 0)), (5, game(1, 100)), (10, reloading), (60, game(1, 100))]);
        assert_eq!(shown, vec![None; 4]);
        assert_eq!(run(&mut status, start, &[(70, game(2, 200))]), vec![None]);
        status.succeeded();
        assert_eq!(run(&mut status, start, &[(80, game(2, 200)), (800, game(2, 200))]), vec![None; 2]);
        // Files that take a while to arrive say so.
        let shown = run(&mut status, start, &[(1000, game(3, 9 << 20)), (1400, game(3, 9 << 20)), (1500, game(3, 9 << 20))]);
        assert_eq!(shown, vec![None, None, Some(Step::Receiving { files: 1, bytes: (9 << 20) - 200, of: None })]);
        status.succeeded();
        // So does a slow reload, then it stops.
        let (idle, reloading) = (game(3, 9 << 20), Facts { reloading: true, ..game(3, 9 << 20) });
        let frames = [(2000, reloading.clone()), (2100, reloading.clone()), (2160, reloading), (2200, idle.clone())];
        assert_eq!(run(&mut status, start, &frames), vec![None, None, Some(Step::Reloading), None]);
        // And an asset the new code asks for.
        let asking = Facts { assets: (2, 3), ..idle.clone() };
        let frames = [(3000, asking.clone()), (3100, asking.clone()), (3160, asking), (3200, idle)];
        assert_eq!(run(&mut status, start, &frames), vec![None, None, Some(Step::Assets { settled: 2, total: 3 }), None]);
    }

    #[test]
    fn a_failure_shows_its_message_until_the_next_push() {
        let start = Instant::now();
        let loading = Facts { loading: true, linked: true, ..Facts::default() };
        let idle = Facts { linked: true, ..Facts::default() };
        let mut status = LoadStatus::default();
        assert_eq!(run(&mut status, start, &[(0, loading.clone())]), vec![Some(Step::Checking)]);
        status.fail("game.fun:3:1: unknown name `tik`".into(), start + Duration::from_millis(5));
        let failed = Some(Step::Failed("game.fun:3:1: unknown name `tik`".into()));
        // No game: it stays.
        assert_eq!(run(&mut status, start, &[(10, idle.clone()), (60_000, idle.clone())]), vec![failed.clone(), failed]);
        assert_eq!(run(&mut status, start, &[(60_010, loading)]), vec![Some(Step::Checking)]);
        // Over a running game (a rejected reload) it clears itself.
        let game = Facts { game: true, ..idle };
        status.fail("boom".into(), start + Duration::from_millis(70_000));
        let shown = run(&mut status, start, &[(70_010, game.clone()), (79_000, game.clone()), (80_010, game)]);
        assert_eq!(shown, vec![Some(Step::Failed("boom".into())), Some(Step::Failed("boom".into())), None]);
        // So does a reload that works, however quick.
        let game = Facts { game: true, linked: true, ..Facts::default() };
        status.fail("boom".into(), start + Duration::from_millis(90_000));
        status.succeeded();
        assert_eq!(run(&mut status, start, &[(90_010, game)]), vec![None]);
    }

    #[test]
    fn a_push_that_stops_arriving_stops_showing() {
        let start = Instant::now();
        let got = Facts { files: 1, bytes: 10, ..Facts::default() };
        let mut status = LoadStatus::default();
        let shown = run(&mut status, start, &[(0, Facts::default()), (10, got.clone()), (19_000, got.clone()), (20_100, got)]);
        let receiving = Some(Step::Receiving { files: 1, bytes: 10, of: None });
        assert_eq!(shown, vec![Some(Step::Waiting), receiving.clone(), receiving, Some(Step::Waiting)]);
    }

    #[test]
    fn a_second_game_counts_its_own_assets_and_a_doze_is_not_a_push() {
        let start = Instant::now();
        let mut status = LoadStatus::default();
        let first = Facts { game: true, assets: (6, 6), ..Facts::default() };
        assert_eq!(run(&mut status, start, &[(0, first)]), vec![None]);
        status.launched(start + Duration::from_millis(5));
        let second = Facts { game: true, assets: (7, 9), ..Facts::default() };
        assert_eq!(run(&mut status, start, &[(10, second)]), vec![Some(Step::Assets { settled: 1, total: 3 })]);
        // Files pushed while the headset dozed have arrived by the time it wakes.
        let settled = Facts { game: true, assets: (9, 9), ..Facts::default() };
        let woke = Facts { files: 4, bytes: 9 << 20, ..settled.clone() };
        let frames = [(5000, settled.clone()), (9000, settled), (60_000, woke.clone()), (60_600, woke.clone()), (61_000, woke)];
        assert_eq!(run(&mut status, start, &frames), vec![Some(Step::Running), None, None, None, None]);
    }

    #[test]
    fn a_failure_wraps_and_running_fades() {
        let long = "cannot load game.fun:12:5: `Scene.modle` is not a member of Scene; did you mean `Scene.model`?";
        let card = lines(&Step::Failed(long.into()));
        assert_eq!(card[0].text, "LOAD FAILED");
        assert!(card.len() > 2 && card[1..].iter().all(|line| overlay_font::width(&line.text, BODY) <= WRAP));
        assert_eq!(lines(&Step::Receiving { files: 3, bytes: 5 << 19, of: None })[1].text, "3 files, 2.5 MB");
        // A sender that says what it will send gets a count against it.
        let of = Some((40, 71 << 20));
        let line = |files, bytes| lines(&Step::Receiving { files, bytes, of })[1].text.clone();
        assert_eq!(line(12, 23 << 20), "23.0 of 71.0 MB (32%), 12 of 40 files");
        assert_eq!(line(40, (71 << 20) + 900), "71.0 of 71.0 MB (100%), 40 of 40 files");
        let none = lines(&Step::Receiving { files: 1, bytes: 10, of: Some((0, 0)) })[1].text.clone();
        assert_eq!(none, "1 file, 0.0 MB");

        let start = Instant::now();
        let mut status = LoadStatus::default();
        let loading = Facts { loading: true, ..Facts::default() };
        let game = Facts { game: true, ..Facts::default() };
        status.step(&loading, start);
        status.launched(start);
        status.step(&game, start);
        let at = |ms| start + Duration::from_millis(ms);
        assert_eq!(status.step(&game, at(400)), Some((Step::Running, 1.0)));
        let (_, alpha) = status.step(&game, at(400 + 1350)).expect("fading");
        assert!((alpha - 0.5).abs() < 0.01, "{alpha}");
    }
}
