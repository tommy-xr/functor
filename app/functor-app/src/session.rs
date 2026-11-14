//! Runs the core's commands other than replies: one `functor run native`
//! (or, on a headset, `functor run vr`) child at a time, its output read for
//! the debug server's address, `/state`
//! polled and input and transport calls posted (in one ordered queue), its
//! input log read for marks; agent runs (`agent::Runner`); saving tasks,
//! layouts and recent projects; project file listings and reads; opening
//! folders (`folders`); and tasks' changes (`changes::Git`).

use std::collections::{HashMap, VecDeque};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use functor_app_core::creation::NewProjectRequest;

use functor_app_core::projects::read_project_files;
use functor_app_core::protocol::ServerMsg;
use functor_app_core::transport::enqueue;
use functor_app_core::{
    listening_url, Command, Event, Harness, Layout, LogPage, Pointer, Polled, PolledClock,
    Scope, SessionCall, SessionEvent, TransportCall,
};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::process::Child;
use tokio::sync::mpsc::{self, Sender, UnboundedSender};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::agent::{Agents, Runner, ViewRoute};
use crate::changes::Git;

/// Between `/state` polls (`?model=0`, so the model is never serialized);
/// shorter while a page shows the transport bar, for a smooth playhead.
const POLL_INTERVAL: Duration = Duration::from_millis(250);
const FAST_POLL_INTERVAL: Duration = Duration::from_millis(50);
/// Between input-log reads for the bar's marks.
const MARKS_INTERVAL: Duration = Duration::from_millis(250);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
/// How long output written just before an exit is still collected.
pub(crate) const DRAIN_TIMEOUT: Duration = Duration::from_millis(500);
/// Output lines kept for a failure report.
pub(crate) const TAIL_LINES: usize = 20;
/// Between checks of an ended session's project files for a change.
const SOURCE_POLL: Duration = Duration::from_millis(500);
/// How long a layout change waits to be written, so a burst (a splitter
/// stepped by key) is one write.
const LAYOUT_DEBOUNCE: Duration = Duration::from_millis(300);

/// The `functor` to run: `--functor`, else `FUNCTOR_BIN`, else one next to
/// this executable (`exe`), else `Functor.app`'s `Contents/Helpers/functor`,
/// else `functor` on `PATH`.
pub fn functor_bin(flag: Option<PathBuf>, env: Option<OsString>, exe: Option<PathBuf>) -> PathBuf {
    let name = format!("functor{}", std::env::consts::EXE_SUFFIX);
    // Helpers, not MacOS: there the game child would share the app's bundle
    // id (its own Functor Dock tile, and Quit by id could reach it instead).
    let local = |dir: &Path| {
        [dir.join(&name), dir.parent()?.join("Helpers").join(&name)]
            .into_iter()
            .find(|p| p.is_file())
    };
    flag.or_else(|| env.filter(|e| !e.is_empty()).map(PathBuf::from))
        .or_else(|| local(exe?.parent()?))
        .unwrap_or_else(|| name.into())
}

pub struct Driver {
    cli: Arc<crate::cli::Cli>,
    events: UnboundedSender<Event>,
    live: Option<Live>,
    runs: Runner,
    /// Where tasks are saved (`store::home`).
    home: PathBuf,
    /// Where the next task id reservation starts: above every id loaded
    /// or reserved here.
    next_task: u64,
    /// The open workspace's git, when changes are tracked.
    git: Option<Arc<Git>>,
    /// Layouts not written yet, by project dir; flushed on drop too.
    layouts: Arc<Mutex<HashMap<PathBuf, Layout>>>,
    opened: Arc<Opened>,
    /// `functor init --list --json`'s answer, with the binary's modification
    /// time it was read at (`None`: not known, so not kept).
    templates: Arc<Mutex<Option<(SystemTime, TemplateList)>>>,
    /// The device link, once started.
    pub link: Option<crate::link::Hub>,
    /// Takes tasks' path evidence.
    evidence: crate::evidence::Worker,
    /// Games being stopped on headsets, by tunnel, until one answers.
    unloads: HashMap<String, JoinHandle<()>>,
    /// Where speech goes, when a model can say (D29).
    pub router: Option<crate::router::Router>,
}

pub type TemplateList = Result<Vec<functor_app_core::creation::Template>, String>;

/// Workspaces read for `Event::Opened`, by request id, until the core takes
/// one (`Command::UseWorkspace`): its git and highest task id.
type Opened = Mutex<HashMap<u64, (Option<Git>, u64)>>;

/// Writes and forgets every pending layout.
fn write_layouts(home: &Path, pending: &Mutex<HashMap<PathBuf, Layout>>) {
    let layouts = std::mem::take(&mut *pending.lock().unwrap());
    for (dir, layout) in layouts {
        if let Err(e) = crate::store::save_layout(home, &dir, &layout) {
            eprintln!("functor-app: cannot save the layout: {e}");
        }
    }
}

impl Drop for Driver {
    /// A layout changed just before exit is still written.
    fn drop(&mut self) {
        write_layouts(&self.home, &self.layouts);
    }
}

/// The current child's supervisor. Dropping it kills the child.
struct Live {
    id: u64,
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
    tail: Arc<Mutex<VecDeque<String>>>,
    /// The child's poster of input, transport calls and polls, started by
    /// the first of them.
    poster: Option<UnboundedSender<SessionCall>>,
    /// The headset's tunnel when the game runs there.
    tunnel: Option<String>,
}

impl Live {
    fn poster(
        &mut self,
        debug_url: String,
        events: &UnboundedSender<Event>,
    ) -> &UnboundedSender<SessionCall> {
        let id = self.id;
        self.poster
            .get_or_insert_with(|| post_calls(id, debug_url, events.clone()))
    }
}

impl Driver {
    /// With no workspace yet: `Command::UseWorkspace` brings its git and
    /// task ids.
    pub fn new(
        cli: Arc<crate::cli::Cli>,
        agents: Agents,
        home: PathBuf,
        events: UnboundedSender<Event>,
        views: Arc<ViewRoute>,
    ) -> Self {
        Self {
            runs: Runner::new(
                cli.clone(),
                agents,
                None,
                events.clone(),
                views,
                home.clone(),
            ),
            evidence: crate::evidence::Worker::new(cli.clone(), home.clone(), events.clone()),
            cli,
            events,
            live: None,
            home,
            next_task: 1,
            git: None,
            layouts: Arc::default(),
            opened: Arc::default(),
            templates: Arc::default(),
            link: None,
            unloads: HashMap::new(),
            router: None,
        }
    }

    /// Reads the folder `path` names for the core (blocking) as request 0,
    /// the one at startup; with `track`, changes are tracked.
    pub fn open(&self, path: String, track: bool) -> Event {
        open_folder(&self.home, &self.opened, 0, path, track)
    }

    /// A client's new task as the core takes it: with an id reserved in
    /// the store. A task the core then rejects leaves its id unused.
    pub fn new_task(
        &mut self,
        message: String,
        harness: Harness,
        scope: Option<Scope>,
        voice: Option<Box<functor_app_core::voice::Utterance>>,
    ) -> Event {
        let id = crate::store::reserve_id(&self.home, self.next_task);
        if let Ok(id) = id {
            self.next_task = id + 1;
        }
        Event::NewTask {
            id: id.map_err(|e| e.to_string()),
            message,
            harness,
            scope,
            voice,
        }
    }

    pub async fn run(&mut self, command: Command) {
        match command {
            Command::StartSession { id, dir, display } => {
                let mut args = vec!["run", "native"];
                args.extend(display.flags());
                args.extend(["--debug-port", "0"]);
                self.start(id, &dir, &args, None)
            }
            Command::StartHeadset { id, dir, tunnel } => {
                // This load supersedes a stop still waiting for the headset.
                if let Some(unload) = self.unloads.remove(&tunnel) {
                    unload.abort();
                }
                let args = ["run", "vr", "--device-url", tunnel.as_str()];
                self.start(id, &dir, &args, Some(tunnel.clone()))
            }
            Command::CheckCli => {
                let (cli, events) = (self.cli.clone(), self.events.clone());
                tokio::spawn(async move {
                    let problem = cli.check().await;
                    let _ = events.send(Event::CliChecked { problem });
                });
            }
            Command::StopSession { id } => {
                if self.live.as_ref().is_some_and(|l| l.id == id) {
                    self.stop().await;
                }
            }
            Command::PollState {
                id,
                debug_url,
                fast,
                after,
            } => {
                if let Some(live) = self.live.as_mut().filter(|l| l.id == id) {
                    schedule_poll(live.poster(debug_url, &self.events).clone(), fast, after);
                }
            }
            Command::Call {
                id,
                debug_url,
                call,
            } => {
                if let Some(live) = self.live.as_mut().filter(|l| l.id == id) {
                    let _ = live.poster(debug_url, &self.events).send(call);
                }
            }
            Command::ReadInputLog {
                id,
                debug_url,
                from,
            } => {
                let events = self.events.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(MARKS_INTERVAL).await;
                    let event = SessionEvent::InputLog(read_input_log(&debug_url, from).await);
                    let _ = events.send(Event::Session { id, event });
                });
            }
            Command::ListFiles { path, dir } => {
                let files = read_project_files(&dir);
                let _ = self.events.send(Event::Files { path, dir, files });
            }
            Command::OpenFolder { id, path } => {
                let (home, opened, events) =
                    (self.home.clone(), self.opened.clone(), self.events.clone());
                tokio::task::spawn_blocking(move || {
                    let _ = events.send(open_folder(&home, &opened, id, path, true));
                });
            }
            Command::UseWorkspace { id } => {
                let taken = {
                    let mut opened = self.opened.lock().unwrap();
                    let taken = opened.remove(&id);
                    // Earlier requests were superseded: the core drops their readings.
                    opened.retain(|&other, _| other > id);
                    taken
                };
                let Some((git, highest)) = taken else {
                    return;
                };
                self.git = git.map(Arc::new);
                self.runs.set_git(self.git.clone());
                self.next_task = self.next_task.max(highest + 1);
            }
            Command::PickFolder { from, parent } => {
                let events = self.events.clone();
                tokio::spawn(async move {
                    let path = crate::folders::pick(from).await;
                    let _ = events.send(Event::Picked { path, parent });
                });
            }
            Command::ListTemplates => {
                let (list, events) = (self.list_templates(), self.events.clone());
                tokio::spawn(async move {
                    let result = list.await;
                    let _ = events.send(Event::Templates { result });
                });
            }
            Command::CreateProject { request, folder } => {
                let (functor, events) = (self.cli.path(), self.events.clone());
                tokio::task::spawn_blocking(move || {
                    let made = functor
                        .map_err(|reason| functor_app_core::creation::CreateFailure {
                            reason,
                            done: Vec::new(),
                            not_done: Vec::new(),
                        })
                        .and_then(|functor| crate::creation::create(&functor, &request, &folder));
                    let (result, note) = match made {
                        Ok(made) => (Ok(made.path), made.note),
                        Err(failure) => (Err(failure), None),
                    };
                    let _ = events.send(Event::Created { result, note });
                });
            }
            Command::ReserveTask {
                message,
                harness,
                voice,
            } => {
                let event = self.new_task(message, harness, None, voice);
                let _ = self.events.send(event);
            }
            // Without a client to answer there is no one to check for.
            Command::CheckNewProject { .. } => {}
            Command::SaveRecents { recents } => {
                if let Err(e) = crate::store::save_recents(&self.home, &recents) {
                    eprintln!("functor-app: cannot save the recent projects: {e}");
                }
            }
            Command::SaveContextWindows { windows } => {
                if let Err(e) = crate::store::save_context_windows(&self.home, &windows) {
                    eprintln!("functor-app: cannot save the context windows: {e}");
                }
            }
            // Without a client to answer there is no one to list for.
            Command::ListDirs { .. } => {}
            Command::StartRun(spec) => self.runs.start(*spec),
            Command::StopRun { task, run } => self.runs.stop(task, run).await,
            Command::StartReview(spec) => self.runs.start_review(*spec),
            Command::StopReview { task, run } => self.runs.stop_review(task, run),
            Command::SaveReview { dir, setting } => {
                if let Err(e) = crate::store::save_review(&self.home, &dir, &setting) {
                    eprintln!("functor-app: cannot save the review setting: {e}");
                }
            }
            Command::SaveModelRoute { dir, on } => {
                if let Err(e) = crate::store::save_model_route(&self.home, &dir, on) {
                    eprintln!("functor-app: cannot save the speech setting: {e}");
                }
            }
            Command::SaveTask { dir, task } => {
                if let Err(e) = crate::store::save(&self.home, &dir, &task) {
                    eprintln!("functor-app: cannot save task {}: {e}", task.id);
                }
            }
            Command::DeleteTask { dir, task } => {
                if let Err(e) = crate::store::delete(&self.home, &dir, task) {
                    eprintln!("functor-app: cannot delete task {task}: {e}");
                }
            }
            Command::SaveHarness { dir, harness } => {
                if let Err(e) = crate::store::save_harness(&self.home, &dir, harness) {
                    eprintln!("functor-app: cannot save the agent choice: {e}");
                }
            }
            Command::SaveScope { dir, scope } => {
                if let Err(e) = crate::store::save_scope(&self.home, &dir, scope) {
                    eprintln!("functor-app: cannot save the instruction scope: {e}");
                }
            }
            Command::SaveLayout { dir, layout } => {
                let first = {
                    let mut pending = self.layouts.lock().unwrap();
                    pending.insert(dir, *layout).is_none() && pending.len() == 1
                };
                if first {
                    let (home, pending) = (self.home.clone(), self.layouts.clone());
                    tokio::spawn(async move {
                        tokio::time::sleep(LAYOUT_DEBOUNCE).await;
                        write_layouts(&home, &pending);
                    });
                }
            }
            // Without a client to answer there is no one to read it for.
            Command::ReadFile { .. } => {}
            // Without a client to answer, only the summary is refreshed.
            Command::Diff { task, baseline, .. } => self.diff(task, baseline, None),
            Command::Revert {
                task,
                baseline,
                paths,
            } => self.revert(task, baseline, paths),
            Command::AppContext { task, run, context } => self.runs.context(task, run, &context),
            Command::TakeEvidence {
                task,
                dir,
                project,
                baseline,
                tree,
            } => match self.git.clone() {
                Some(git) => self.evidence.take(crate::evidence::Job {
                    task,
                    git,
                    dir,
                    project,
                    baseline,
                    tree,
                }),
                None => {
                    let reason = "changes are tracked only in a git repository".to_string();
                    let state = functor_app_core::evidence::EvidenceState::Withheld { reason };
                    let _ = self.events.send(Event::Evidence { task, tree, state });
                }
            },
            Command::DropEvidence { files } => {
                let dir = crate::store::evidence_dir(&self.home);
                for file in files.iter().filter(|f| crate::evidence::is_name(f)) {
                    let _ = std::fs::remove_file(dir.join(file));
                }
            }
            Command::Link(command) => {
                if let Some(link) = &self.link {
                    link.command(command);
                }
            }
            Command::Route {
                device,
                utterance,
                request,
            } => {
                let (router, events) = (self.router.clone(), self.events.clone());
                tokio::spawn(async move {
                    let choice = match router {
                        Some(router) => router.choose(&request).await,
                        None => Err("no model router".to_string()),
                    };
                    // The rule decides instead: said, so a router that never
                    // answers in time can be noticed.
                    if let Err(e) = &choice {
                        eprintln!("functor-app: the router: {e}; the fixed rule decides");
                    }
                    let _ = events.send(Event::Routed {
                        device,
                        utterance,
                        choice,
                    });
                });
            }
            // Clients and view requests are the server's (`server::run`); `--print` has neither.
            Command::Reply(_) | Command::Broadcast(_) | Command::ViewReply(_) => {}
        }
    }

    /// Measures `task`'s change since `baseline` in the background; with
    /// `reply`, also sends its full diff there as `ServerMsg::Diff`.
    pub fn diff(&self, task: u64, baseline: String, reply: Option<Sender<String>>) {
        let Some(git) = self.git.clone() else {
            return;
        };
        let events = self.events.clone();
        tokio::spawn(async move {
            let Some(reply) = reply else {
                return crate::changes::measure(&git, task, baseline, &events).await;
            };
            let msg = match git.diff(&baseline).await {
                Ok((change, files, more)) => {
                    let tree = Some(change.tree.clone());
                    let _ = events.send(Event::Change {
                        task,
                        baseline,
                        change,
                    });
                    ServerMsg::Diff {
                        task,
                        tree,
                        files,
                        more,
                        error: None,
                    }
                }
                Err(error) => ServerMsg::Diff {
                    task,
                    tree: None,
                    files: vec![],
                    more: 0,
                    error: Some(error),
                },
            };
            let _ = reply.try_send(serde_json::to_string(&msg).expect("ServerMsg serializes"));
        });
    }

    /// Lists directories completing `path` in the background and sends them
    /// to `reply` as `ServerMsg::Dirs`, with why `path` cannot be opened.
    pub fn list_dirs(&self, path: String, reply: Option<Sender<String>>) {
        let Some(reply) = reply else {
            return;
        };
        tokio::task::spawn_blocking(move || {
            let home = std::env::home_dir();
            let msg = ServerMsg::Dirs {
                dirs: crate::folders::suggest(&path, home.as_deref()),
                error: crate::folders::resolve(&path, home.as_deref()).err(),
                path,
            };
            let _ = reply.try_send(serde_json::to_string(&msg).expect("ServerMsg serializes"));
        });
    }

    /// `functor init --list --json`'s templates, kept while the binary's
    /// modification time stays the same.
    pub fn list_templates(&self) -> impl std::future::Future<Output = TemplateList> + 'static {
        let (functor, cache) = (self.cli.path(), self.templates.clone());
        async move {
            let functor = functor?;
            let modified = std::fs::metadata(&functor).and_then(|m| m.modified()).ok();
            let kept = cache.lock().unwrap().clone();
            match (kept, modified) {
                (Some((at, list)), Some(now)) if at == now => list,
                _ => {
                    let list = crate::creation::list(&functor).await;
                    // A failure is asked again next time.
                    if let (Some(m), Ok(_)) = (modified, &list) {
                        *cache.lock().unwrap() = Some((m, list.clone()));
                    }
                    list
                }
            }
        }
    }

    /// Works out what `request` would do in the background and sends it to
    /// `reply` as `ServerMsg::NewProjectPlan`.
    pub fn check_new_project(&self, request: NewProjectRequest, reply: Option<Sender<String>>) {
        let Some(reply) = reply else {
            return;
        };
        tokio::task::spawn_blocking(move || {
            let (plan, error) = match crate::creation::check(&request) {
                Ok(plan) => (Some(plan), None),
                Err(error) => (None, Some(error)),
            };
            let msg = ServerMsg::NewProjectPlan {
                request,
                plan,
                error,
            };
            let _ = reply.try_send(serde_json::to_string(&msg).expect("ServerMsg serializes"));
        });
    }

    /// Reads `path` of `project` (at `dir`) in the background and sends it
    /// to `reply` as `ServerMsg::File`.
    pub fn read_file(
        &self,
        root: String,
        project: String,
        path: String,
        dir: PathBuf,
        reply: Option<Sender<String>>,
    ) {
        let Some(reply) = reply else {
            return;
        };
        tokio::task::spawn_blocking(move || {
            let (text, error) = match crate::files::read_project_file(&dir, &path) {
                Ok(text) => (Some(text), None),
                Err(error) => (None, Some(error)),
            };
            let msg = ServerMsg::File {
                root,
                project,
                path,
                text,
                error,
            };
            let _ = reply.try_send(serde_json::to_string(&msg).expect("ServerMsg serializes"));
        });
    }

    fn revert(&self, task: u64, baseline: String, paths: Vec<String>) {
        let git = self.git.clone();
        let events = self.events.clone();
        tokio::spawn(async move {
            let result = match git {
                Some(git) => git.revert(&baseline, &paths).await,
                None => Err("changes are tracked only in a git repository".to_string()),
            };
            let _ = events.send(Event::Reverted {
                task,
                baseline,
                result,
            });
        });
    }

    /// Where the app keeps its data (`store::home`).
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// The current child's last output lines, oldest first.
    pub fn output_tail(&self) -> Vec<String> {
        self.live
            .as_ref()
            .map(|l| l.tail.lock().unwrap().iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Spawns `functor -d <dir> <args>`; `tunnel` when it runs the game on
    /// a headset.
    fn start(&mut self, id: u64, dir: &Path, args: &[&str], tunnel: Option<String>) {
        let send = |event| {
            let _ = self.events.send(Event::Session { id, event });
        };
        let functor = match self.cli.path() {
            Ok(functor) => functor,
            Err(reason) => return send(SessionEvent::StartFailed { reason }),
        };
        let spawned = tokio::process::Command::new(&functor)
            .arg("-d")
            .arg(dir)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // Backstop for every exit path that drops the child unstopped
            // (a panic, the runtime shutting down).
            .kill_on_drop(true)
            .spawn();
        let child = match spawned {
            Ok(child) => child,
            Err(e) => {
                let reason = format!("cannot run {}: {e}", functor.display());
                return send(SessionEvent::StartFailed { reason });
            }
        };
        if let Some(pid) = child.id() {
            send(SessionEvent::Spawned { pid });
        }
        let (stop, stopped) = oneshot::channel();
        let tail = Arc::new(Mutex::new(VecDeque::new()));
        let task = tokio::spawn(supervise(
            id,
            dir.to_path_buf(),
            child,
            self.events.clone(),
            tail.clone(),
            stopped,
        ));
        // A previous child (none, if the core stopped it first) dies with its `Live`.
        self.live = Some(Live {
            id,
            stop,
            task,
            tail,
            poster: None,
            tunnel,
        });
    }

    /// Kills the current child and waits until it is gone. A headset is
    /// returned to its boot scene, the game stopping there too: at once, or
    /// once it is back if it is away.
    async fn stop(&mut self) {
        if let Some(live) = self.live.take() {
            let _ = live.stop.send(());
            let _ = live.task.await;
            if let Some(tunnel) = live.tunnel {
                self.unloads.retain(|_, task| !task.is_finished());
                let task = tokio::spawn(unload(tunnel.clone()));
                if let Some(old) = self.unloads.insert(tunnel, task) {
                    old.abort();
                }
            }
        }
    }
}

/// Between tries to stop a game on a headset that is away, and how long.
const UNLOAD_RETRY: Duration = Duration::from_secs(2);
const UNLOAD_FOR: Duration = Duration::from_secs(30 * 60);

/// `POST /load-project []` through `tunnel`: the headset's boot scene.
/// Retried while the headset does not answer (the tunnel's 502–504, or the
/// tunnel itself gone), for up to [`UNLOAD_FOR`].
async fn unload(tunnel: String) {
    let json = [("Content-Type", "application/json")];
    let until = tokio::time::Instant::now() + UNLOAD_FOR;
    loop {
        let reply = exchange(&tunnel, "POST", "/load-project", &json, "[]").await;
        let status = reply.as_ref().ok().and_then(|(head, _)| {
            head.split(' ').nth(1).and_then(|code| code.parse::<u16>().ok())
        });
        match status {
            Some(200..=299) => return,
            Some(502..=504) | None if tokio::time::Instant::now() < until => {
                tokio::time::sleep(UNLOAD_RETRY).await
            }
            _ => {
                let why = reply.map_or_else(|e| e, |(head, body)| format!("{head}: {body}"));
                return eprintln!("functor-app: cannot stop the game on the headset: {why}");
            }
        }
    }
}

/// `Event::Opened` for request `id`, the folder `path` names, keeping what
/// the core does not hold (its git, its highest task id) in `opened`.
fn open_folder(home: &Path, opened: &Opened, id: u64, path: String, track: bool) -> Event {
    let result = crate::folders::load(&path, home, track).map(|loaded| {
        let kept = (loaded.git, loaded.highest);
        opened.lock().unwrap().insert(id, kept);
        Box::new(loaded.state)
    });
    Event::Opened {
        id,
        folder: path,
        result,
    }
}

/// Reports the child's listening line and exit; kills it when `stop` fires
/// or its sender drops.
async fn supervise(
    id: u64,
    dir: PathBuf,
    mut child: Child,
    events: UnboundedSender<Event>,
    tail: Arc<Mutex<VecDeque<String>>>,
    mut stop: oneshot::Receiver<()>,
) {
    let send = |event| {
        let _ = events.send(Event::Session { id, event });
    };
    let sources = source_stamp(&dir);
    // Both streams: the runtime logs to stderr, but the CLI's own messages
    // (load errors included) go to stdout when it is a pipe.
    let (line_tx, mut lines) = mpsc::unbounded_channel();
    forward(child.stdout.take().unwrap(), line_tx.clone());
    forward(child.stderr.take().unwrap(), line_tx);
    let record = |line: String| {
        if let Some(debug_url) = listening_url(&line) {
            send(SessionEvent::Listening { debug_url });
        }
        let mut tail = tail.lock().unwrap();
        if tail.len() == TAIL_LINES {
            tail.pop_front();
        }
        tail.push_back(line);
    };
    let mut open = true;
    let status = loop {
        tokio::select! {
            _ = &mut stop => return kill(child).await,
            status = child.wait() => break status,
            line = lines.recv(), if open => match line {
                Some(line) => record(line),
                None => open = false,
            },
        }
    };
    // Lines written just before the exit may still be in the pipes; bounded,
    // in case something else holds them open.
    let deadline = tokio::time::Instant::now() + DRAIN_TIMEOUT;
    while let Ok(Some(line)) = tokio::time::timeout_at(deadline, lines.recv()).await {
        record(line);
    }
    let reason = match status {
        Ok(status) if status.success() => status.to_string(),
        Ok(status) => match cause(&tail.lock().unwrap()) {
            Some(line) => format!("{status}: {line}"),
            None => status.to_string(),
        },
        Err(e) => e.to_string(),
    };
    send(SessionEvent::Exited { reason });
    // Until stopped, says once that a project file changed: a fixed load
    // error starts the game again.
    loop {
        tokio::select! {
            _ = &mut stop => return,
            _ = tokio::time::sleep(SOURCE_POLL) => if source_stamp(&dir) != sources {
                return send(SessionEvent::SourceChanged);
            },
        }
    }
}

/// `dir`'s project files and their modification times: any edit, addition
/// or removal changes it.
fn source_stamp(dir: &Path) -> Vec<(String, Option<SystemTime>)> {
    read_project_files(dir)
        .into_iter()
        .map(|name| {
            let modified = std::fs::metadata(dir.join(&name)).and_then(|m| m.modified());
            (name, modified.ok())
        })
        .collect()
}

/// Sends `reader`'s lines until it closes. Reads to the end whatever the
/// bytes: a closed pipe would fail the child's next write and kill it.
pub(crate) fn forward(
    reader: impl AsyncRead + Unpin + Send + 'static,
    lines: UnboundedSender<String>,
) {
    tokio::spawn(async move {
        let mut reader = BufReader::new(reader).split(b'\n');
        while let Ok(Some(line)) = reader.next_segment().await {
            let line = String::from_utf8_lossy(&line);
            let _ = lines.send(line.trim_end_matches('\r').to_string());
        }
    });
}

/// The first `error…` or panic line, else the last line: a failed load
/// prints its cause, then `error: cannot load …`, then `✗ failed in 1ms`.
pub(crate) fn cause(tail: &VecDeque<String>) -> Option<&str> {
    let error = tail
        .iter()
        .find(|l| l.starts_with("error") || l.contains("panicked at"));
    error.or(tail.back()).map(String::as_str)
}

async fn kill(mut child: Child) {
    // Kills and reaps.
    let _ = child.kill().await;
}

/// One HTTP/1.0 request to `url` (`http://host:port`; the server closes
/// after its response): the body of a 2xx response, or why it failed.
pub async fn request(
    url: &str,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> Result<String, String> {
    let body = request_bytes(url, method, path, headers, body).await?;
    Ok(String::from_utf8_lossy(&body).into_owned())
}

/// As [`request`], the body as bytes (a `/capture`'s PNG).
pub async fn request_bytes(
    url: &str,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> Result<Vec<u8>, String> {
    let what = format!("{method} {path}");
    let (head, body) = exchange_bytes(url, method, path, headers, body).await?;
    let status = head.lines().next().unwrap_or_default();
    if !status
        .split(' ')
        .nth(1)
        .is_some_and(|code| code.starts_with('2'))
    {
        let text = String::from_utf8_lossy(&body);
        return Err(format!("{what}: {status}: {}", text.trim()));
    }
    Ok(body)
}

/// As [`request`], but the response's head and body whatever its status.
pub async fn exchange(
    url: &str,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> Result<(String, String), String> {
    let (head, body) = exchange_bytes(url, method, path, headers, body).await?;
    Ok((head, String::from_utf8_lossy(&body).into_owned()))
}

async fn exchange_bytes(
    url: &str,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> Result<(String, Vec<u8>), String> {
    let what = format!("{method} {path}");
    let addr = url
        .strip_prefix("http://")
        .ok_or_else(|| format!("not an http url: {url}"))?;
    let request = async {
        let mut stream = TcpStream::connect(addr).await?;
        let headers: String = headers
            .iter()
            .map(|(k, v)| format!("{k}: {v}\r\n"))
            .collect();
        let head = format!(
            "{method} {path} HTTP/1.0\r\nHost: {addr}\r\n{headers}Content-Length: {}\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(head.as_bytes()).await?;
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await?;
        Ok::<_, std::io::Error>(response)
    };
    let response = tokio::time::timeout(REQUEST_TIMEOUT, request)
        .await
        .map_err(|_| format!("{what} timed out"))?
        .map_err(|e| format!("{what}: {e}"))?;
    let split = response.windows(4).position(|w| w == b"\r\n\r\n");
    let (head, body) = match split {
        Some(at) => (&response[..at], &response[at + 4..]),
        None => (&response[..], &[][..]),
    };
    Ok((String::from_utf8_lossy(head).into_owned(), body.to_vec()))
}

/// `GET /state`'s `frame`, pointer surface and `clock`.
pub(crate) async fn poll_state(debug_url: &str) -> Result<Polled, String> {
    let body = request(debug_url, "GET", "/state?model=0", &[], "").await?;
    parse_state(&body)
}

fn parse_state(body: &str) -> Result<Polled, String> {
    let state: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("GET /state: {e}"))?;
    let frame = state["frame"]
        .as_u64()
        .ok_or_else(|| "GET /state: no `frame`".to_string())?;
    let mouse = &state["input"]["mouse"];
    let size = |key: &str| mouse[key].as_u64().and_then(|n| u32::try_from(n).ok());
    let surface = match (size("surface_width"), size("surface_height")) {
        (Some(w), Some(h)) if w > 0 && h > 0 => Some([w, h]),
        _ => None,
    };
    // Absent before debug protocol v37.
    let clock = state.get("clock").map(|c| {
        let recorded = &c["recorded"];
        PolledClock {
            paused: c["paused"].as_bool().unwrap_or(false),
            recorded: recorded["oldest"]
                .as_u64()
                .zip(recorded["newest"].as_u64())
                .map(|(o, n)| [o, n]),
            generation: c["generation"].as_u64().unwrap_or(0),
            program_revision: c["program_revision"].as_u64().unwrap_or(0),
        }
    });
    // Absent before debug protocol v44: a visible cursor, as the view had.
    let pointer = match state["pointer"].as_str() {
        Some("captured") => Pointer::Captured,
        _ => Pointer::Visible,
    };
    Ok(Polled {
        frame,
        surface,
        pointer,
        clock,
    })
}

/// `GET /input-log?from=`: each step's marks (steps without any left out).
async fn read_input_log(debug_url: &str, from: u64) -> Result<LogPage, String> {
    let body = request(
        debug_url,
        "GET",
        &format!("/input-log?from={from}"),
        &[],
        "",
    )
    .await?;
    parse_input_log(&body)
}

fn parse_input_log(body: &str) -> Result<LogPage, String> {
    let log: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("GET /input-log: {e}"))?;
    let number = |key: &str| log[key].as_u64().unwrap_or(0);
    let steps = log["steps"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let steps = steps
        .iter()
        .filter_map(|step| {
            let marks: Vec<(String, String)> = step["marks"]
                .as_array()?
                .iter()
                .filter_map(|m| {
                    Some((
                        m["kind"].as_str()?.to_string(),
                        m["label"].as_str()?.to_string(),
                    ))
                })
                .collect();
            Some((step["frame"].as_u64()?, marks)).filter(|(_, m)| !m.is_empty())
        })
        .collect();
    Ok(LogPage {
        generation: number("generation"),
        oldest: number("oldest"),
        newest: number("newest"),
        steps,
    })
}

/// Queues a poll on `poster` after the interval (`fast`: shorter).
fn schedule_poll(poster: UnboundedSender<SessionCall>, fast: bool, after: u64) {
    tokio::spawn(async move {
        tokio::time::sleep(if fast {
            FAST_POLL_INTERVAL
        } else {
            POLL_INTERVAL
        })
        .await;
        let _ = poster.send(SessionCall::Poll { after });
    });
}

/// Posts session `id`'s calls in order, one request at a time: the debug
/// server answers each on its next frame, so a seek or move that queues
/// meanwhile is collapsed to the latest (`enqueue`), and a poll reports what
/// every call before it did. Ends when the sender drops (the session stops).
fn post_calls(
    id: u64,
    debug_url: String,
    events: UnboundedSender<Event>,
) -> UnboundedSender<SessionCall> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let mut queue = VecDeque::new();
        loop {
            while let Ok(call) = rx.try_recv() {
                enqueue(&mut queue, call);
            }
            let call = match queue.pop_front() {
                Some(call) => call,
                None => match rx.recv().await {
                    Some(call) => call,
                    None => return,
                },
            };
            let (path, body) = match call {
                SessionCall::Poll { after } => {
                    let result = poll_state(&debug_url).await;
                    let event = SessionEvent::Polled { after, result };
                    let _ = events.send(Event::Session { id, event });
                    continue;
                }
                SessionCall::Input(input) => (
                    "/input",
                    serde_json::to_string(&input).expect("GameInput serializes"),
                ),
                SessionCall::Transport(call) => transport_request(&call),
            };
            if let Err(e) = request(&debug_url, "POST", path, &[], &body).await {
                eprintln!("functor-app: {e}");
            }
        }
    });
    tx
}

/// The debug server's `POST` route and body for a transport call.
fn transport_request(call: &TransportCall) -> (&'static str, String) {
    let time = |command: serde_json::Value| ("/time", command.to_string());
    match call {
        // `cancel` pauses at the current time; `set` would need it read first.
        TransportCall::Pause => time(serde_json::json!({"type": "cancel"})),
        TransportCall::Resume => time(serde_json::json!({"type": "resume"})),
        // One step of the web timeline's 60 Hz.
        TransportCall::Step => time(serde_json::json!({"type": "advance", "dts": 1.0 / 60.0})),
        TransportCall::Seek { frame, .. } => {
            ("/seek", serde_json::json!({ "frame": frame }).to_string())
        }
        // `Timeline::call` turns a peek into the switch it stands for.
        TransportCall::Extrapolate { on } | TransportCall::Peek { on } => (
            "/preview",
            serde_json::json!({ "extrapolate": on }).to_string(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use functor_app_core::protocol::GameInput;

    /// Every `GameInput` the app posts reads back as the debug server's
    /// `InputCommand` with the same fields: the hand mirror cannot drift.
    #[test]
    fn game_inputs_are_debug_server_input_commands() {
        use functor_app_core::protocol::MouseButton;
        use functor_runtime_common::debug_protocol::InputCommand;
        let buttons = [MouseButton::Left, MouseButton::Right, MouseButton::Middle];
        let inputs = [
            GameInput::Key {
                key: "space".into(),
                down: true,
            },
            GameInput::MouseMove { x: -3, y: 7 },
            GameInput::MouseWheel { delta: -1 },
        ]
        .into_iter()
        .chain(buttons.map(|button| GameInput::MouseButton {
            button,
            down: false,
        }));
        for input in inputs {
            // Exhaustive: a new variant fails to compile until it is mapped.
            let expected = match input.clone() {
                GameInput::Key { key, down } => InputCommand::Key { key, down },
                GameInput::MouseMove { x, y } => InputCommand::MouseMove { x, y },
                GameInput::MouseWheel { delta } => InputCommand::MouseWheel { delta },
                GameInput::MouseButton { button, down } => {
                    let name = serde_json::to_value(button).unwrap();
                    let name = name.as_str().unwrap().to_string();
                    assert!(functor_runtime_common::MouseButton::from_name(&name).is_some());
                    InputCommand::MouseButton { button: name, down }
                }
            };
            let wire = serde_json::to_value(&input).unwrap();
            let read: InputCommand = serde_json::from_value(wire.clone())
                .unwrap_or_else(|e| panic!("{wire} is not an InputCommand: {e}"));
            assert_eq!(read, expected, "{wire}");
        }
    }

    #[test]
    fn cause_prefers_an_error_or_panic_line() {
        let tail = |lines: &[&str]| lines.iter().map(|l| l.to_string()).collect::<VecDeque<_>>();
        let failed = tail(&[
            "▸ run examples/bad (native)",
            "error: ./game.fun:2:1: expected an expression, found end of input",
            "error: cannot load the ./game.fun project",
            "✗ failed in 1ms",
        ]);
        assert_eq!(
            cause(&failed),
            Some("error: ./game.fun:2:1: expected an expression, found end of input")
        );
        let panicked = tail(&[
            "thread 'main' panicked at src/run.rs:10:5:",
            "note: run with `RUST_BACKTRACE=1`",
        ]);
        assert_eq!(
            cause(&panicked),
            Some("thread 'main' panicked at src/run.rs:10:5:")
        );
        assert_eq!(cause(&tail(&["a", "b"])), Some("b"));
        assert_eq!(cause(&tail(&[])), None);
    }

    /// `/state`'s clock and `/input-log`'s marks, as the runtime's own types
    /// serialize them, read back as the core keeps them.
    #[test]
    fn the_runtimes_clock_and_marks_parse() {
        use functor_runtime_common::debug_protocol::{
            ClockState, InputLog, InputLogStep, RecordedFrames, TimelineMark,
        };
        let clock = ClockState {
            paused: true,
            recorded: Some(RecordedFrames {
                oldest: 3,
                newest: 40,
            }),
            generation: 2,
            program_revision: 5,
        };
        let state = serde_json::json!({
            "frame": 12,
            "clock": clock,
            "input": {"mouse": {"surface_width": 800, "surface_height": 600}},
        });
        let polled = parse_state(&state.to_string()).unwrap();
        assert_eq!(
            polled,
            Polled {
                frame: 12,
                surface: Some([800, 600]),
                pointer: Pointer::Visible,
                clock: Some(PolledClock {
                    paused: true,
                    recorded: Some([3, 40]),
                    generation: 2,
                    program_revision: 5,
                }),
            }
        );
        // A runtime before the clock (v37) reports none; nothing recorded is no window.
        let old = serde_json::json!({"frame": 1, "input": {"mouse": {}}});
        assert_eq!(parse_state(&old.to_string()).unwrap().clock, None);
        // Nor its pointer (v44): a visible cursor, as before.
        assert_eq!(parse_state(&old.to_string()).unwrap().pointer, Pointer::Visible);
        let captured = serde_json::json!({"frame": 1, "pointer": "captured", "input": {}});
        assert_eq!(parse_state(&captured.to_string()).unwrap().pointer, Pointer::Captured);
        let bare = serde_json::json!({"frame": 0, "clock": ClockState::default(), "input": {}});
        assert_eq!(
            parse_state(&bare.to_string()).unwrap().clock,
            Some(PolledClock::default())
        );

        let step = |frame, marks: Vec<TimelineMark>| InputLogStep {
            frame,
            marks,
            ..InputLogStep::default()
        };
        let w = TimelineMark {
            kind: "key-down".into(),
            label: "W down".into(),
        };
        let log = InputLog {
            generation: 1,
            oldest: 3,
            newest: 5,
            steps: vec![step(3, vec![]), step(4, vec![w]), step(5, vec![])],
        };
        let page = parse_input_log(&serde_json::to_string(&log).unwrap()).unwrap();
        assert_eq!(
            page,
            LogPage {
                generation: 1,
                oldest: 3,
                newest: 5,
                steps: vec![(4, vec![("key-down".into(), "W down".into())])],
            }
        );
    }

    /// A debug server that records each request, answers `/state` with a
    /// frame, and takes `slow` before answering `POST /seek {"frame":1}`.
    struct FakeDebugServer {
        url: String,
        requests: Arc<Mutex<Vec<(String, String, String)>>>,
        /// The most requests it was answering at once.
        overlap: Arc<Mutex<(u32, u32)>>,
    }

    impl FakeDebugServer {
        async fn start(slow: Duration) -> Self {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let requests: Arc<Mutex<Vec<(String, String, String)>>> = Arc::default();
            let overlap: Arc<Mutex<(u32, u32)>> = Arc::default();
            let (log, busy) = (requests.clone(), overlap.clone());
            tokio::spawn(async move {
                loop {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    let (log, busy) = (log.clone(), busy.clone());
                    tokio::spawn(async move {
                        let mut buf = Vec::new();
                        let mut chunk = [0u8; 1024];
                        // Head and body (the client sends both, then waits).
                        let request = loop {
                            let n = stream.read(&mut chunk).await.unwrap();
                            buf.extend_from_slice(&chunk[..n]);
                            let text = String::from_utf8_lossy(&buf).to_string();
                            if let Some((head, body)) = text.split_once("\r\n\r\n") {
                                let length = head
                                    .lines()
                                    .find_map(|l| l.strip_prefix("Content-Length: "))
                                    .map_or(0, |n| n.parse().unwrap());
                                if body.len() >= length {
                                    break (head.to_string(), body.to_string());
                                }
                            }
                        };
                        let mut words = request.0.split(' ');
                        let (method, path) = (words.next().unwrap(), words.next().unwrap());
                        {
                            let mut b = busy.lock().unwrap();
                            b.0 += 1;
                            b.1 = b.1.max(b.0);
                        }
                        log.lock()
                            .unwrap()
                            .push((method.into(), path.into(), request.1.clone()));
                        if path == "/seek" && request.1 == r#"{"frame":1}"# {
                            tokio::time::sleep(slow).await;
                        }
                        let body = if path.starts_with("/state") {
                            r#"{"frame":9,"clock":{"paused":false,"generation":0,"program_revision":0},"input":{}}"#
                        } else {
                            "ok"
                        };
                        busy.lock().unwrap().0 -= 1;
                        let reply = format!(
                            "HTTP/1.0 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                            body.len()
                        );
                        let _ = stream.write_all(reply.as_bytes()).await;
                    });
                }
            });
            Self {
                url,
                requests,
                overlap,
            }
        }

        fn requests(&self) -> Vec<(String, String, String)> {
            self.requests.lock().unwrap().clone()
        }
    }

    /// The next session event, or a panic after a few seconds.
    async fn next_event(events: &mut mpsc::UnboundedReceiver<Event>) -> SessionEvent {
        match tokio::time::timeout(Duration::from_secs(5), events.recv()).await {
            Ok(Some(Event::Session { id: 7, event })) => event,
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn calls_are_posted_in_order_and_a_slow_one_is_not_overtaken() {
        let server = FakeDebugServer::start(Duration::from_millis(300)).await;
        let (events, mut rx) = mpsc::unbounded_channel();
        let poster = post_calls(7, server.url.clone(), events);
        let seek = |frame| SessionCall::Transport(TransportCall::Seek { frame, id: frame });
        poster.send(seek(1)).unwrap();
        // The slow seek is on its way while the rest queue behind it.
        tokio::time::sleep(Duration::from_millis(50)).await;
        let key = SessionCall::Input(functor_app_core::protocol::GameInput::Key {
            key: "w".into(),
            down: true,
        });
        let pause = SessionCall::Transport(TransportCall::Pause);
        let step = SessionCall::Transport(TransportCall::Step);
        let resume = SessionCall::Transport(TransportCall::Resume);
        for call in [seek(2), key, seek(3), seek(4), pause, step, resume] {
            poster.send(call).unwrap();
        }
        poster.send(SessionCall::Poll { after: 5 }).unwrap();
        // The poll answers last, with what every call before it did.
        let SessionEvent::Polled { after: 5, result } = next_event(&mut rx).await else {
            panic!("a poll's answer");
        };
        assert_eq!(result.unwrap().frame, 9);
        let wire = |method: &str, path: &str, body: &str| (method.into(), path.into(), body.into());
        assert_eq!(
            server.requests(),
            vec![
                wire("POST", "/seek", r#"{"frame":1}"#),
                wire("POST", "/seek", r#"{"frame":2}"#),
                wire("POST", "/input", r#"{"type":"key","key":"w","down":true}"#),
                // 3 waited behind the key and was replaced by 4.
                wire("POST", "/seek", r#"{"frame":4}"#),
                wire("POST", "/time", r#"{"type":"cancel"}"#),
                wire(
                    "POST",
                    "/time",
                    &format!(r#"{{"dts":{},"type":"advance"}}"#, 1.0f64 / 60.0)
                ),
                wire("POST", "/time", r#"{"type":"resume"}"#),
                wire("GET", "/state?model=0", ""),
            ]
        );
        assert_eq!(server.overlap.lock().unwrap().1, 1, "one request at a time");
    }

    #[tokio::test]
    async fn polls_come_faster_while_the_bar_shows() {
        let server = FakeDebugServer::start(Duration::ZERO).await;
        let (events, mut rx) = mpsc::unbounded_channel();
        let poster = post_calls(7, server.url.clone(), events);
        for fast in [true, false] {
            let asked = tokio::time::Instant::now();
            schedule_poll(poster.clone(), fast, 0);
            let SessionEvent::Polled { .. } = next_event(&mut rx).await else {
                panic!("a poll's answer");
            };
            let took = asked.elapsed();
            if fast {
                assert!(
                    took >= FAST_POLL_INTERVAL && took < POLL_INTERVAL,
                    "{took:?}"
                );
            } else {
                assert!(took >= POLL_INTERVAL, "{took:?}");
            }
        }
    }

    #[test]
    fn functor_bin_precedence() {
        let dir = tempfile::tempdir().unwrap();
        let name = format!("functor{}", std::env::consts::EXE_SUFFIX);
        let exe = dir.path().join("functor-app");
        let flag = || Some(PathBuf::from("/flag/functor"));
        let env = || Some(OsString::from("/env/functor"));
        // The flag, then the env var, win over everything after them.
        assert_eq!(
            functor_bin(flag(), env(), Some(exe.clone())),
            PathBuf::from("/flag/functor")
        );
        assert_eq!(
            functor_bin(None, env(), Some(exe.clone())),
            PathBuf::from("/env/functor")
        );
        // Without a sibling `functor`, an empty env var falls through to PATH.
        assert_eq!(
            functor_bin(None, Some(OsString::new()), Some(exe.clone())),
            PathBuf::from(&name)
        );
        std::fs::write(dir.path().join(&name), "").unwrap();
        assert_eq!(
            functor_bin(None, None, Some(exe.clone())),
            dir.path().join(&name)
        );
        assert_eq!(functor_bin(None, None, None), PathBuf::from(&name));
        // In Functor.app: Contents/MacOS/functor-app runs Contents/Helpers/functor.
        let contents = dir.path().join("Functor.app/Contents");
        std::fs::create_dir_all(contents.join("MacOS")).unwrap();
        std::fs::create_dir_all(contents.join("Helpers")).unwrap();
        std::fs::write(contents.join("Helpers").join(&name), "").unwrap();
        assert_eq!(
            functor_bin(None, None, Some(contents.join("MacOS/functor-app"))),
            contents.join("Helpers").join(&name)
        );
    }
}
