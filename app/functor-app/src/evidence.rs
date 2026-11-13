//! Path evidence (D38): the game's moment as a task started, kept, then
//! replayed in a hidden session of the task's baseline files and another of
//! its current files, each captured with its predicted path drawn. The
//! user's own session and working tree are only read.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use functor_app_core::evidence::{EvidenceState, Moment};
use functor_app_core::{listening_url, Event};
use serde_json::{json, Value};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use crate::changes::Git;
use crate::session::{request, request_bytes};

/// The most one pair of captures may take.
const JOB_TIMEOUT: Duration = Duration::from_secs(60);
/// Between polls of a replaying session.
const POLL: Duration = Duration::from_millis(50);

/// The game's moment, as kept for a task: its frame, its state there and
/// its recorded input log (`GET /input-log`, kept from the first step).
#[derive(serde::Serialize, serde::Deserialize)]
struct Kept {
    frame: u64,
    model: Value,
    log: Value,
}

/// Reads the game's moment from the session at `debug_url` and keeps it as
/// `file`: what both paths replay. Only reads the session.
pub async fn take_moment(debug_url: &str, file: &Path) -> Moment {
    let missing = |reason: String| Moment::Missing { reason };
    let read = async {
        let state: Value = json_of(&request(debug_url, "GET", "/state", &[], "").await?)?;
        let log: Value = json_of(&request(debug_url, "GET", "/input-log", &[], "").await?)?;
        Ok::<_, String>((state, log))
    };
    let (state, log) = match read.await {
        Ok(read) => read,
        Err(e) => return missing(format!("the game could not be read ({e})")),
    };
    let frame = state["frame"].as_u64().unwrap_or(0);
    let (oldest, steps) = (log["oldest"].as_u64(), log["newest"].as_u64().unwrap_or(0));
    if steps > 0 && oldest != Some(1) {
        return missing(
            "the game had run longer than its recorded inputs reach back (about 15 s), \
             so the run cannot be replayed from its start"
                .to_string(),
        );
    }
    let kept = Kept {
        frame,
        model: state["model"].clone(),
        log,
    };
    let text = serde_json::to_vec(&kept).expect("the moment serializes");
    if let Err(e) = crate::store::save_private(file, &text) {
        return missing(format!("the moment could not be kept ({e})"));
    }
    Moment::Taken { frame, steps }
}

fn json_of(text: &str) -> Result<Value, String> {
    serde_json::from_str(text).map_err(|e| e.to_string())
}

/// One pair of captures to take.
pub struct Job {
    pub task: u64,
    pub git: Arc<Git>,
    /// The project's directory, and its path in the repo.
    pub dir: PathBuf,
    pub project: String,
    pub baseline: String,
    pub tree: String,
}

/// Takes captures one job at a time, off the core loop; a job for a task
/// replaces that task's pending or running one.
pub struct Worker(UnboundedSender<Job>);

impl Worker {
    pub fn new(functor: PathBuf, home: PathBuf, events: UnboundedSender<Event>) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(work(rx, functor, home, events));
        Self(tx)
    }

    pub fn take(&self, job: Job) {
        let _ = self.0.send(job);
    }
}

async fn work(
    mut jobs: UnboundedReceiver<Job>,
    functor: PathBuf,
    home: PathBuf,
    events: UnboundedSender<Event>,
) {
    let mut pending: VecDeque<Job> = VecDeque::new();
    loop {
        let job = match pending.pop_front() {
            Some(job) => job,
            None => match jobs.recv().await {
                Some(job) => job,
                None => return,
            },
        };
        let taking = tokio::time::timeout(JOB_TIMEOUT, take(&job, &functor, &home));
        tokio::pin!(taking);
        let state = loop {
            tokio::select! {
                taken = &mut taking => break Some(taken.unwrap_or_else(|_| withheld(format!(
                    "taking the paths took longer than {} s", JOB_TIMEOUT.as_secs()
                )))),
                Some(next) = jobs.recv() => {
                    pending.retain(|j| j.task != next.task);
                    if next.task == job.task {
                        // Dropping the capture kills its sessions.
                        pending.push_front(next);
                        break None;
                    }
                    pending.push_back(next);
                }
            }
        };
        if let Some(state) = state {
            let (task, tree) = (job.task, job.tree.clone());
            let _ = events.send(Event::Evidence { task, tree, state });
        }
    }
}

/// Whether `name` is one this module names a capture (`<uuid>-before.png`):
/// all a request or a stored task may point the app at.
pub fn is_name(name: &str) -> bool {
    let id = name
        .strip_suffix("-before.png")
        .or_else(|| name.strip_suffix("-after.png"));
    id.is_some_and(|id| id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn withheld(reason: String) -> EvidenceState {
    EvidenceState::Withheld { reason }
}

/// Both captures, compared: shown, the same, or withheld with the reason.
async fn take(job: &Job, functor: &Path, home: &Path) -> EvidenceState {
    let started = std::time::Instant::now();
    let state = match pair(job, functor, home).await {
        Ok(None) => EvidenceState::Same,
        Ok(Some((before, after))) => {
            let dir = crate::store::evidence_dir(home);
            let name = |side: &str| format!("{}-{side}.png", uuid::Uuid::new_v4().simple());
            let (b, a) = (name("before"), name("after"));
            let saved = crate::store::save_private(&dir.join(&b), &before)
                .and_then(|()| crate::store::save_private(&dir.join(&a), &after));
            match saved {
                Ok(()) => EvidenceState::Shown {
                    before: b,
                    after: a,
                },
                Err(e) => withheld(format!("the captures could not be kept ({e})")),
            }
        }
        Err(reason) => withheld(reason),
    };
    eprintln!(
        "functor-app: task {}'s paths: {} in {:.1} s",
        job.task,
        match &state {
            EvidenceState::Shown { .. } => "taken".to_string(),
            EvidenceState::Same => "the same".to_string(),
            EvidenceState::Withheld { reason } => format!("withheld: {reason}"),
            EvidenceState::Taking => unreachable!(),
        },
        started.elapsed().as_secs_f32()
    );
    state
}

/// The before and after captures, or `None` when they are the same.
async fn pair(job: &Job, functor: &Path, home: &Path) -> Result<Option<(Vec<u8>, Vec<u8>)>, String> {
    let file = crate::store::moment_file(home, &job.dir, job.task);
    let kept: Kept = std::fs::read(&file)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .ok_or("the game's moment was not kept")?;
    if kept.model.is_null() {
        return Err("the game's state cannot be compared, so a replay cannot be checked".into());
    }
    let work = Scratch::new()?;
    let log = work.0.join("input-log.json");
    std::fs::write(&log, kept.log.to_string())
        .map_err(|e| format!("cannot write the inputs: {e}"))?;
    let moment = (kept.frame, kept.log["newest"].as_u64().unwrap_or(0));
    let mut sides = Vec::new();
    for (name, tree, check) in [
        ("before", &job.baseline, Some(&kept.model)),
        ("after", &job.tree, None),
    ] {
        let dir = work.0.join(name);
        materialize(job, tree, &dir).await?;
        let side = replay(functor, &dir, &log, moment, check)
            .await
            .map_err(|e| format!("the {} code: {e}", if check.is_some() { "old" } else { "new" }))?;
        sides.push(side);
    }
    let after = sides.pop().expect("two sides");
    let before = sides.pop().expect("two sides");
    if before.camera != after.camera {
        return Err("the camera differs between the old and new code at that moment".into());
    }
    Ok((before.png != after.png).then_some((before.png, after.png)))
}

/// Snapshot `tree`'s project files as `dir`, with the working tree's ignored
/// files (fetched assets) linked in: no snapshot holds them, and the change
/// cannot have changed them.
async fn materialize(job: &Job, tree: &str, dir: &Path) -> Result<(), String> {
    job.git
        .export(tree, &job.project, dir)
        .await
        .map_err(|e| format!("cannot write its files: {e}"))?;
    let root = job.git.root();
    let prefix = match job.project.as_str() {
        "." => String::new(),
        p => format!("{p}/"),
    };
    for path in job.git.ignored(&job.project).await? {
        let path = path.trim_end_matches('/');
        let Some(rest) = path.strip_prefix(&prefix).filter(|r| !r.is_empty()) else {
            continue;
        };
        let link = dir.join(rest);
        if link.symlink_metadata().is_ok() {
            continue;
        }
        if let Some(parent) = link.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        symlink(&root.join(path), &link).map_err(|e| format!("cannot link {rest}: {e}"))?;
    }
    Ok(())
}

#[cfg(unix)]
fn symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    match target.is_dir() {
        true => std::os::windows::fs::symlink_dir(target, link),
        false => std::os::windows::fs::symlink_file(target, link),
    }
}

/// One side's capture, with the camera it was drawn from.
struct Side {
    png: Vec<u8>,
    camera: Value,
}

/// Replays `log` (`steps` long) in a hidden session of the project at
/// `dir`, shows `frame` and captures it with the predicted path drawn. With
/// `check`, the game's state there must be that one: the replay reproduced
/// the run.
async fn replay(
    functor: &Path,
    dir: &Path,
    log: &Path,
    (frame, steps): (u64, u64),
    check: Option<&Value>,
) -> Result<Side, String> {
    let mut child = tokio::process::Command::new(functor)
        .arg("-d")
        .arg(dir)
        .args(["run", "native", "--hidden", "--preview-presence", "1"])
        .arg("--replay-input")
        .arg(log)
        .args(["--debug-port", "0"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("cannot run {}: {e}", functor.display()))?;
    let (line_tx, mut lines) = mpsc::unbounded_channel();
    crate::session::forward(child.stdout.take().expect("piped"), line_tx.clone());
    crate::session::forward(child.stderr.take().expect("piped"), line_tx);
    // Its last lines, for a failure.
    let tail = Arc::new(Mutex::new(VecDeque::new()));
    let push = |tail: &Mutex<VecDeque<String>>, line: String| {
        let mut tail = tail.lock().unwrap();
        if tail.len() == 20 {
            tail.pop_front();
        }
        tail.push_back(line);
    };
    let failed = |tail: &Mutex<VecDeque<String>>| {
        let tail = tail.lock().unwrap();
        let cause = crate::session::cause(&tail).unwrap_or("it exited");
        format!("the game did not load or replay: {cause}")
    };
    let url = loop {
        let Some(line) = lines.recv().await else {
            let _ = child.wait().await;
            return Err(failed(&tail));
        };
        let url = listening_url(&line);
        push(&tail, line);
        if let Some(url) = url {
            break url;
        }
    };
    let kept = tail.clone();
    tokio::spawn(async move {
        while let Some(line) = lines.recv().await {
            push(&kept, line);
        }
    });
    // Replayed: paused on the last step.
    loop {
        if let Ok(Some(_)) = child.try_wait() {
            return Err(failed(&tail));
        }
        let state = request(&url, "GET", "/state?model=0", &[], "").await;
        if let Ok(state) = state.and_then(|s| json_of(&s)) {
            let newest = state["clock"]["recorded"]["newest"].as_u64().unwrap_or(0);
            if state["clock"]["paused"] == true
                && state["frame"].as_u64() == Some(steps)
                && newest == steps
            {
                break;
            }
        }
        tokio::time::sleep(POLL).await;
    }
    if frame != steps {
        request(&url, "POST", "/seek", &[], &json!({ "frame": frame }).to_string()).await?;
    }
    let state = json_of(&request(&url, "GET", "/state", &[], "").await?)?;
    if state["frame"].as_u64() != Some(frame) {
        return Err(format!("it could not show frame {frame}"));
    }
    if check.is_some_and(|model| state["model"] != *model) {
        return Err("replaying the recorded inputs did not reach the game's state at that \
             moment, so its paths could mislead"
            .into());
    }
    let scene = json_of(&request(&url, "GET", "/scene", &[], "").await?)?;
    request(&url, "POST", "/preview", &[], &json!({ "extrapolate": true }).to_string()).await?;
    let png = request_bytes(&url, "POST", "/capture", &[], "").await?;
    Ok(Side {
        png,
        camera: scene["camera"].clone(),
    })
}

/// A temporary directory, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Result<Self, String> {
        let dir = std::env::temp_dir().join(format!("functor-paths-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).map_err(|e| format!("cannot make a work directory: {e}"))?;
        Ok(Self(dir))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
