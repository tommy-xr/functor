//! The desktop app's shell around `functor-app-core`, as a library: run by
//! the `functor` CLI in-process (its `app` feature), and by the thin
//! `functor-app` binary.

mod agent;
mod changes;
mod cli;
mod creation;
mod evidence;
mod files;
mod folders;
mod link;
mod router;
mod server;
mod session;
mod store;
mod stub;
mod voice;
#[cfg(target_os = "macos")]
mod window;

use std::ffi::OsString;
use std::future::Future;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::pin::Pin;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};
use functor_app_core::metrics::{review_metrics, run_metrics};
use functor_app_core::protocol::{ClientMsg, ServerMsg};
use functor_app_core::{
    print, print_change, print_item, update, AppState, Command, Event, Harness, Item, RunEvent,
    SessionStatus,
};
use tokio::sync::mpsc::{self, UnboundedReceiver};

use agent::Agents;
use session::Driver;

/// How long `--start-session` waits for the first `/state`.
const START_TIMEOUT: Duration = Duration::from_secs(60);
/// How long a `task` run over `--max-input` has to finish before it is cut.
const CUT_GRACE: Duration = Duration::from_secs(1);

#[derive(Parser, Debug)]
#[command(about = "The Functor desktop app")]
struct Args {
    /// The folder to open (else the current directory if it is in or holds a
    /// project, else the most recent project).
    #[arg(short, long, global = true)]
    dir: Option<PathBuf>,

    /// Run without a window: serve the UI for a browser, at the printed URL.
    #[arg(long, global = true)]
    headless: bool,

    /// Open the window, wait until its page has connected, close it and exit
    /// 0; non-zero with the reason if that takes too long.
    #[arg(long, hide = true, conflicts_with = "headless")]
    window_smoke: bool,

    /// Print the state as text and exit.
    #[arg(long)]
    print: bool,

    /// With --print: start the active project's game session, print once it
    /// answers `/state`, then stop it.
    #[arg(long, requires = "print")]
    start_session: bool,

    /// Run game sessions without a GPU (`--headless` instead of `--hidden`).
    #[arg(long, global = true)]
    no_gpu: bool,

    /// The `functor` binary for game sessions and agent tools. Else, run by
    /// `functor`: itself; as `functor-app`: `FUNCTOR_BIN`, else one next to
    /// this executable, else `functor` on PATH.
    #[arg(long, value_name = "PATH", global = true)]
    functor: Option<PathBuf>,

    /// The port to serve the UI on, on 127.0.0.1 (0 picks a free one).
    #[arg(long, default_value_t = 0, conflicts_with = "print")]
    port: u16,

    /// Serve the UI from this directory instead of the one built in.
    #[arg(long, value_name = "PATH", conflicts_with = "print")]
    ui_dir: Option<PathBuf>,

    /// Turn off the device link: no LAN listener, no mDNS advert.
    #[arg(long, global = true)]
    no_device_link: bool,

    /// Where the device link listens for headsets (a free port on every
    /// interface by default).
    #[arg(long, value_name = "ADDR", default_value = "0.0.0.0:0", global = true)]
    device_link: std::net::SocketAddr,

    /// Listen for headsets without advertising (tests).
    #[arg(long, hide = true, global = true)]
    no_mdns: bool,

    /// The `adb` that finds headsets on a USB cable (tests pass a fake).
    #[arg(
        long,
        hide = true,
        value_name = "PATH",
        default_value = "adb",
        global = true
    )]
    adb: PathBuf,

    /// The fake agent's child: print a recorded run's events, then exit.
    #[arg(long, value_name = "FIXTURE", hide = true)]
    replay_events: Option<PathBuf>,

    /// With --replay-events: the session the run continues.
    #[arg(long, value_name = "SESSION", hide = true, requires = "replay_events")]
    replay_resume: Option<String>,

    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Run one agent task in the active project, print its transcript, and
    /// exit 0 if it finished ok. `FUNCTOR_APP_FAKE_AGENT=<fixture>` replays a
    /// recorded run instead of starting an agent.
    Task {
        /// The task's first message.
        prompt: String,

        #[arg(long, default_value = "claude", value_parser = ["claude", "codex"])]
        harness: String,

        /// How much of your own setup the run reads (D39); default: the
        /// project's setting. Codex always reads all of it.
        #[arg(long, value_parser = ["user_and_project", "project", "isolated"])]
        scope: Option<String>,

        /// Print each transcript item as it completes, not all at the end.
        #[arg(long)]
        follow: bool,

        /// Start the project's game session first, for the agent's tools.
        #[arg(long)]
        start_session: bool,

        /// Write the run's metrics here as JSON when it ends (plan 1.19).
        #[arg(long, value_name = "PATH")]
        metrics: Option<PathBuf>,

        /// Stop the run once its cumulative input tokens pass this. Claude
        /// reports usage as it goes; Codex only at the end, too late to stop.
        #[arg(long, value_name = "TOKENS")]
        max_input: Option<u64>,

        /// When the run leaves a change, review it as the app does (D31, as
        /// the project's setting says) and wait for that too; its numbers go
        /// in the metrics as `review`. `FUNCTOR_APP_FAKE_REVIEW=<fixture>`
        /// replays one under the fake agent.
        #[arg(long)]
        review: bool,
    },
    /// Speak the device side of the device link, for tests: pair, link,
    /// answer canned debug requests.
    #[command(hide = true)]
    DeviceStub(stub::StubArgs),
    /// Print the voice capsule (plan 4.1) a running game's state makes: its
    /// frame, game time and, on a headset, head and controller poses, as JSON.
    Capsule {
        /// The game's debug server, `http://127.0.0.1:<port>`.
        #[arg(long, value_name = "URL")]
        from_session: String,
    },
}

/// The window's side of the runtime thread: where to send the server's
/// origin once it listens, and the window's request to shut down.
type WindowLink = (
    std::sync::mpsc::Sender<String>,
    tokio::sync::oneshot::Receiver<()>,
);

/// The process the app runs in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Host {
    /// `functor-app`: runs the `functor` it finds (and checks).
    Shim,
    /// `functor` itself: every child is this executable again (D40).
    Cli,
}

static HOST: OnceLock<Host> = OnceLock::new();

/// This executable with what reaches the app's own child modes
/// (`--replay-events`): `functor-app …`, or `functor app …`.
pub(crate) fn this_app() -> std::io::Result<tokio::process::Command> {
    let mut command = tokio::process::Command::new(std::env::current_exe()?);
    if HOST.get() == Some(&Host::Cli) {
        command.arg("app");
    }
    Ok(command)
}

/// Runs the app with `argv` (`argv[0]` names it in usage messages) on the
/// calling thread, which must be the main one on macOS (the window's event
/// loop), with no async runtime around it (the app starts its own). Call it
/// while this is the only thread: it edits the environment.
pub fn run(argv: impl IntoIterator<Item = OsString>, host: Host) -> std::io::Result<ExitCode> {
    let _ = HOST.set(host);
    // First, while this is the only thread: it edits the environment.
    let token = server::Token::take();
    #[cfg(target_os = "macos")]
    use_login_path();
    let mut args = Args::parse_from(argv);
    if let Some(fixture) = &args.replay_events {
        return Ok(
            match agent::replay(fixture, args.replay_resume.as_deref()) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("functor-app: {e}");
                    ExitCode::FAILURE
                }
            },
        );
    }
    let runtime = || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
    };
    if matches!(args.command, Some(Cmd::DeviceStub(_))) {
        let Some(Cmd::DeviceStub(stub)) = args.command.take() else {
            unreachable!("just matched");
        };
        return Ok(runtime()?.block_on(stub::run(stub)));
    }
    if let Some(Cmd::Capsule { from_session }) = &args.command {
        let read = session::request(from_session, "GET", "/state?model=0", &[], "");
        let state = runtime()?
            .block_on(read)
            .and_then(|t| serde_json::from_str(&t).map_err(|e| e.to_string()));
        return Ok(match state {
            Ok(state) => {
                println!("{}", voice::capsule_from_state(&state).to_json());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("functor-app: cannot read {from_session}: {e}");
                ExitCode::FAILURE
            }
        });
    }
    if args.headless {
        return runtime()?.block_on(app(args, host, token, None));
    }
    if args.print || args.command.is_some() {
        eprintln!("functor-app: --print and task need --headless");
        return Ok(ExitCode::from(2));
    }
    #[cfg(target_os = "macos")]
    {
        let home = store::home(std::env::var_os("FUNCTOR_APP_HOME"), std::env::home_dir());
        let smoke = args.window_smoke;
        let reveal = token.clone();
        window::run(home, smoke, reveal, move |link| {
            runtime()?.block_on(app(args, host, token, Some(link)))
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("functor-app: the window is macOS-only for now; run with --headless");
        Ok(ExitCode::from(2))
    }
}

/// Everything but the window, on the runtime thread: `--print`, `task`, or
/// the server until a signal (or, with `window`, the window) stops it.
async fn app(
    args: Args,
    host: Host,
    token: server::Token,
    window: Option<WindowLink>,
) -> std::io::Result<ExitCode> {
    let Some(home) = store::home(std::env::var_os("FUNCTOR_APP_HOME"), std::env::home_dir()) else {
        eprintln!("functor-app: no home directory; set FUNCTOR_APP_HOME");
        return Ok(ExitCode::from(2));
    };
    if let Err(e) = store::make_private(&home) {
        eprintln!("functor-app: cannot make {} private: {e}", home.display());
    }
    let exe = std::env::current_exe().ok();
    let cli = std::sync::Arc::new(match (host, args.functor, exe) {
        // Itself, unless told otherwise: no lookup, nothing to check.
        (Host::Cli, None, Some(exe)) => cli::Cli::own(exe),
        (Host::Cli, flag, exe) => cli::Cli::new(flag, None, exe),
        (Host::Shim, flag, exe) => cli::Cli::new(flag, std::env::var_os("FUNCTOR_BIN"), exe),
    });
    // Absolute: the fake agent's child runs in the project directory.
    let fixture = |var| std::env::var_os(var).filter(|f: &std::ffi::OsString| !f.is_empty());
    let agents = match fixture("FUNCTOR_APP_FAKE_AGENT") {
        Some(run) => Agents::Fake {
            run: std::path::absolute(run)?,
            review: fixture("FUNCTOR_APP_FAKE_REVIEW")
                .map(std::path::absolute)
                .transpose()?,
        },
        None => Agents::Real,
    };
    let mut state = AppState::none();
    state.no_gpu = args.no_gpu;
    state.harnesses = agents.available();
    // `task` reviews only when asked; `--print` runs nothing.
    let asked = match &args.command {
        Some(Cmd::Task { review, .. }) => *review,
        _ => !args.print,
    };
    state.can_review = asked && agents.can_review();
    state.native_picker = window.is_some();
    state.recents = store::load_recents(&home);
    state.context_windows = store::load_context_windows(&home);
    // Before its first use: a problem starts nothing and is said (D40).
    let problem = cli.check().await;
    (state, _) = update(state, Event::CliChecked { problem });
    let dir = args.dir.map(std::fs::canonicalize).transpose()?;
    let cwd = match dir {
        Some(_) => None,
        None => std::env::current_dir()
            .ok()
            .filter(|cwd| folders::cwd_holds_project(cwd)),
    };
    // `--print` and `task` act on the folder they name or are run in, never
    // on whatever was open last.
    let interactive = !args.print && args.command.is_none();
    state.start_on_open = interactive;
    state.auto_start = interactive;
    let recents = if interactive { &state.recents[..] } else { &[] };
    let (tx, events) = mpsc::unbounded_channel();
    let hub = link::Hub::new(home.clone(), tx.clone())?;
    state.speech = hub.speaks();
    let views = std::sync::Arc::new(agent::ViewRoute::default());
    let router = router::Router::from_env(&agents);
    state.router = router.is_some();
    let mut driver = Driver::new(cli.clone(), agents, home, tx.clone(), views.clone());
    driver.router = router;
    driver.link = Some(hub.clone());
    if let Some(folder) = functor_app_core::folders::startup(dir, cwd, recents) {
        // `--print` changes no task, so it measures none.
        let opened = driver.open(folder.to_string_lossy().into_owned(), !args.print);
        if let Event::Opened {
            folder,
            result: Err(e),
            ..
        } = &opened
        {
            eprintln!("functor-app: cannot open {folder}: {e}");
        }
        let commands;
        (state, commands) = update(state, opened);
        for command in commands {
            // `--print` writes nothing to the store.
            if !(args.print && matches!(command, Command::SaveRecents { .. })) {
                driver.run(command).await;
            }
        }
    }
    if args.print && !args.start_session {
        // The stored pairings, all away: `--print` starts no link.
        (state, _) = update(state, hub.started(None));
        print!("{}", print(&state));
        return Ok(ExitCode::SUCCESS);
    }
    if args.print {
        return print_session(state, driver, events).await;
    }
    if let Some(Cmd::Task {
        prompt,
        harness,
        scope,
        follow,
        start_session,
        metrics,
        max_input,
        review,
    }) = args.command
    {
        let harness = match harness.as_str() {
            "codex" => Harness::Codex,
            _ => Harness::Claude,
        };
        let calls = match review {
            true => Some(server::serve_views(views.clone(), driver.home(), hub.clone()).await?),
            false => None,
        };
        let task = TaskArgs {
            prompt,
            harness,
            scope: scope.map(|s| serde_json::from_value(s.into()).expect("a scope clap allows")),
            follow,
            start_session,
            metrics,
            max_input,
            calls,
        };
        return run_task(state, driver, events, task).await;
    }
    let ui = match args.ui_dir {
        Some(dir) if !dir.join("index.html").is_file() => {
            eprintln!("functor-app: no index.html in --ui-dir {}", dir.display());
            return Ok(ExitCode::from(2));
        }
        Some(dir) => server::Ui::Dir(dir),
        None => server::Ui::Embedded,
    };
    // For the new-project modal; listed again whenever it opens.
    driver.run(Command::ListTemplates).await;
    let signal = shutdown_signal()?;
    let (listener, origin) = server::bind(args.port).await?;
    // Runs started from here on can show views (`task` and `--print` have no route).
    views.listen(&origin, false);
    // The rest of the app works without the link; why it is off is said
    // after the `listening on` line, which launchers read first.
    let link = match args.no_device_link {
        true => Ok(None),
        false => {
            let config = link::Config {
                bind: args.device_link,
                advertise: !args.no_mdns,
                adb: args.adb,
            };
            hub.listen(config).await.map(|addr| Some(addr.to_string()))
        }
    };
    let listening = link.as_ref().ok().cloned().flatten();
    let _ = tx.send(hub.started(listening.clone()));
    let say_link = || match &link {
        Ok(Some(addr)) => eprintln!("[functor-app] device link on {addr}"),
        Ok(None) => {}
        Err(e) => eprintln!("functor-app: the device link is off: {e}"),
    };
    let code = match window {
        // The full URL, for a person to open; see `server` on the token.
        None => {
            eprintln!(
                "[functor-app] listening on {origin}/#token={}",
                token.reveal()
            );
            say_link();
            let shutdown = async { 128 + signal.await };
            server::serve(
                state,
                driver,
                events,
                listener,
                token,
                views.clone(),
                ui,
                shutdown,
            )
            .await?
        }
        Some((window_origin, close)) => {
            eprintln!("[functor-app] listening on {origin}");
            say_link();
            let _ = window_origin.send(origin);
            // Closing the window stops everything the way a signal does.
            let shutdown = async {
                tokio::select! {
                    signal = signal => 128 + signal,
                    _ = close => 0,
                }
            };
            server::serve(
                state,
                driver,
                events,
                listener,
                token,
                views.clone(),
                ui,
                shutdown,
            )
            .await?
        }
    };
    hub.shutdown();
    Ok(ExitCode::from(code))
}

type Shutdown<'a> = Pin<&'a mut dyn Future<Output = u8>>;

/// Starts the session and steps events until it answers `/state`; else
/// why not, with the exit code (1, or 128 + n on signal n).
async fn start_and_poll(
    mut state: AppState,
    driver: &mut Driver,
    events: &mut UnboundedReceiver<Event>,
    mut shutdown: Shutdown<'_>,
) -> (AppState, Result<(), (u8, String)>) {
    state = step(state, Event::Client(ClientMsg::StartSession), driver).await;
    let deadline = tokio::time::sleep(START_TIMEOUT);
    tokio::pin!(deadline);
    let outcome = loop {
        match state.session.as_ref().map(|s| &s.status) {
            None => break Err((1, "no project to start a session for".to_string())),
            Some(SessionStatus::Running { timeline, .. }) if timeline.clock.is_some() => {
                break Ok(())
            }
            Some(SessionStatus::Starting | SessionStatus::Running { .. }) => {}
            Some(_) => break Err((1, "the session ended before answering /state".to_string())),
        }
        tokio::select! {
            // A terminal Ctrl-C also reaches the child; report the signal, not its exit.
            biased;
            signal = &mut shutdown => break Err((128 + signal, format!("signal {signal}"))),
            Some(event) = events.recv() => state = step(state, event, driver).await,
            _ = &mut deadline => {
                let msg = format!("no answer from /state within {}s", START_TIMEOUT.as_secs());
                break Err((1, msg));
            }
        }
    };
    (state, outcome)
}

/// Reports a failed `start_and_poll`, with the game's last output lines.
fn session_failed(msg: &str, tail: Vec<String>) {
    eprintln!("functor-app: {msg}");
    if !tail.is_empty() {
        eprintln!("functor-app: the game's last output lines:");
        for line in tail {
            eprintln!("  {line}");
        }
    }
}

/// `--start-session`: start, print once polled, stop. Exits 1 if the session
/// fails or never answers, 128 + n on signal n, after killing the child.
async fn print_session(
    state: AppState,
    mut driver: Driver,
    mut events: UnboundedReceiver<Event>,
) -> std::io::Result<ExitCode> {
    // Handlers go in before the child exists, so no signal can skip its kill.
    let shutdown = shutdown_signal()?;
    tokio::pin!(shutdown);
    let (state, outcome) = start_and_poll(state, &mut driver, &mut events, shutdown).await;
    // Printed after the stop: a blocked stdout must not keep the child alive.
    let dump = print(&state);
    let tail = driver.output_tail();
    step(state, Event::Client(ClientMsg::StopSession), &mut driver).await;
    print!("{dump}");
    Ok(match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err((code, msg)) => {
            session_failed(&msg, tail);
            ExitCode::from(code)
        }
    })
}

struct TaskArgs {
    prompt: String,
    harness: Harness,
    scope: Option<functor_app_core::Scope>,
    follow: bool,
    start_session: bool,
    metrics: Option<PathBuf>,
    max_input: Option<u64>,
    /// With `--review`: the view route its reviews report through.
    calls: Option<UnboundedReceiver<server::ViewCall>>,
}

/// `task`: runs one task to its end, printing its transcript. Exits 0 if it
/// finished ok, 1 otherwise, 128 + n on signal n (after stopping the run).
async fn run_task(
    mut state: AppState,
    mut driver: Driver,
    mut events: UnboundedReceiver<Event>,
    args: TaskArgs,
) -> std::io::Result<ExitCode> {
    let shutdown = shutdown_signal()?;
    tokio::pin!(shutdown);
    if args.start_session {
        let outcome;
        (state, outcome) = start_and_poll(state, &mut driver, &mut events, shutdown.as_mut()).await;
        if let Err((code, msg)) = outcome {
            let tail = driver.output_tail();
            step(state, Event::Shutdown, &mut driver).await;
            session_failed(&msg, tail);
            return Ok(ExitCode::from(code));
        }
    }
    let new = driver.new_task(args.prompt, args.harness, args.scope, None);
    let started = std::time::Instant::now();
    state.now = server::wall_ms();
    let (next, commands) = update(state, new);
    state = next;
    for command in commands {
        match command {
            Command::Reply(ServerMsg::Rejected { reason }) => {
                step(state, Event::Shutdown, &mut driver).await;
                eprintln!("functor-app: {reason}");
                return Ok(ExitCode::FAILURE);
            }
            command => driver.run(command).await,
        }
    }
    let id = state
        .tasks
        .iter()
        .map(|t| t.id)
        .max()
        .expect("the new task");
    let transcript = |state: &AppState| -> Vec<Item> {
        let task = state.tasks.iter().find(|t| t.id == id).expect("the task");
        task.transcript.clone()
    };
    let mut printed = 0;
    let mut first_event = None;
    // Why the run was stopped before it ended.
    let mut cut = None;
    // When the run went over `--max-input` without finishing.
    let mut over: Option<std::time::Instant> = None;
    // The run's time, and the review's start (`--review`).
    let mut wall = None;
    let mut review_started = None;
    let mut calls = args.calls;
    let signal = loop {
        if args.follow {
            printed = print_settled(&transcript(&state), printed);
        }
        let task = state.tasks.iter().find(|t| t.id == id).expect("the task");
        let preparing = (task.review.as_ref())
            .is_some_and(|r| r.state == functor_app_core::review::ReviewState::Preparing);
        if task.run.is_none() {
            wall.get_or_insert_with(|| started.elapsed());
        }
        if preparing {
            review_started.get_or_insert_with(std::time::Instant::now);
        }
        let Some(run) = &task.run else {
            if preparing {
                // Bounded by the review's own limits.
                tokio::select! {
                    biased;
                    signal = &mut shutdown => break Some(signal),
                    Some(call) = next_call(&mut calls) => state = answer(state, call, &mut driver).await,
                    Some(event) = events.recv() => state = step(state, event, &mut driver).await,
                }
                continue;
            }
            break None;
        };
        let finished = matches!(task.transcript.last(), Some(Item::Finished { .. }));
        if let (Some(usage), Some(max), false) = (run.usage, args.max_input, finished) {
            // A finishing run's last usage comes just before its `finished`,
            // so a run over budget gets a moment to finish before it is cut.
            if usage.input > max
                && over.get_or_insert_with(std::time::Instant::now).elapsed() >= CUT_GRACE
            {
                cut = Some(format!("over {max} input tokens"));
                break None;
            }
        }
        tokio::select! {
            biased;
            signal = &mut shutdown => break Some(signal),
            _ = tokio::time::sleep(CUT_GRACE), if over.is_some() => {}
            Some(event) = events.recv() => {
                let agent = matches!(&event, Event::Run { task, event: RunEvent::Agent(_), .. } if *task == id);
                if agent && first_event.is_none() {
                    first_event = Some(started.elapsed());
                }
                state = step(state, event, &mut driver).await
            }
        }
    };
    let wall = wall.unwrap_or_else(|| started.elapsed());
    let review_ms = review_started.map(|t| t.elapsed().as_millis() as u64);
    // Stops the run (on a signal or a cut), a review and the session.
    state = step(state, Event::Shutdown, &mut driver).await;
    if let Some(path) = &args.metrics {
        let task = state.tasks.iter().find(|t| t.id == id).expect("the task");
        let mut record = serde_json::to_value(run_metrics(task)).expect("metrics serialize");
        record["wall_ms"] = (wall.as_millis() as u64).into();
        record["first_event_ms"] = first_event.map(|d| d.as_millis() as u64).into();
        record["cut"] = cut.or(signal.map(|s| format!("signal {s}"))).into();
        if let Some(review) = &task.review {
            let mut r = serde_json::to_value(review_metrics(review, task.report.as_ref()))
                .expect("metrics serialize");
            r["wall_ms"] = review_ms.into();
            record["review"] = r;
        }
        if let Err(e) = std::fs::write(path, format!("{record}\n")) {
            eprintln!("functor-app: cannot write {}: {e}", path.display());
        }
    }
    let items = transcript(&state);
    for item in &items[if args.follow { printed } else { 0 }..] {
        say(item);
    }
    if state.tracks_changes {
        let task = state.tasks.iter().find(|t| t.id == id).expect("the task");
        let change = task.change.clone().unwrap_or_default();
        use std::io::Write;
        let _ = write!(std::io::stdout(), "{}", print_change(&change));
    }
    Ok(match (signal, items.last()) {
        (Some(signal), _) => ExitCode::from(128 + signal),
        (None, Some(Item::Finished { ok: true, .. })) => ExitCode::SUCCESS,
        (None, _) => ExitCode::FAILURE,
    })
}

/// Prints `items` from `from` while they are complete (no text still
/// streaming, no call waiting on its result); the next unprinted index.
fn print_settled(items: &[Item], from: usize) -> usize {
    let settled = items[from..]
        .iter()
        .take_while(|i| {
            !matches!(
                i,
                Item::Agent { partial: true, .. } | Item::Tool { result: None, .. }
            )
        })
        .count();
    for item in &items[from..from + settled] {
        say(item);
    }
    from + settled
}

/// Prints `item`, ignoring a closed stdout: the run must still be stopped.
fn say(item: &Item) {
    use std::io::Write;
    let _ = writeln!(std::io::stdout(), "{}", print_item(item));
}

/// The next view request to `task`'s route, if it serves one.
async fn next_call(calls: &mut Option<UnboundedReceiver<server::ViewCall>>) -> Option<server::ViewCall> {
    match calls {
        Some(calls) => calls.recv().await,
        None => std::future::pending().await,
    }
}

/// Steps `call`'s event, sending the core's answer back to its caller.
async fn answer(mut state: AppState, call: server::ViewCall, driver: &mut Driver) -> AppState {
    state.now = server::wall_ms();
    let (state, commands) = update(state, call.event);
    let mut reply = Some(call.reply);
    for command in commands {
        match command {
            Command::ViewReply(answer) => {
                if let Some(reply) = reply.take() {
                    let _ = reply.send(answer);
                }
            }
            command => driver.run(command).await,
        }
    }
    state
}

async fn step(mut state: AppState, event: Event, driver: &mut Driver) -> AppState {
    state.now = server::wall_ms();
    let (state, commands) = update(state, event);
    for command in commands {
        driver.run(command).await;
    }
    state
}

/// Mark the login shell's `PATH` in its output, apart from anything its
/// startup or logout files print.
#[cfg(any(target_os = "macos", test))]
const PATH_START: &str = "__FUNCTOR_PATH__";
#[cfg(any(target_os = "macos", test))]
const PATH_END: &str = "__FUNCTOR_PATH_END__";

/// In `Functor.app`, takes the login shell's `PATH`: Finder and the Dock
/// start apps with launchd's `/usr/bin:/bin:/usr/sbin:/sbin`, which has no
/// `claude`, `codex` or `adb`. Gives up after 3s, keeping the PATH it has.
#[cfg(target_os = "macos")]
fn use_login_path() {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    let exe = std::env::current_exe().unwrap_or_default();
    if !exe
        .parent()
        .is_some_and(|dir| dir.ends_with("Contents/MacOS"))
    {
        return;
    }
    let shell = std::env::var_os("SHELL").unwrap_or_else(|| "/bin/zsh".into());
    let Ok(mut child) = Command::new(shell)
        .args([
            "-ilc",
            &format!("printf '{PATH_START}%s{PATH_END}' \"$PATH\""),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        // Its own group: an interactive shell on a terminal would take the
        // terminal's foreground from this app.
        .process_group(0)
        .spawn()
    else {
        return;
    };
    // Reads until the PATH is complete, not to EOF: a background job its
    // startup files began can hold the pipe open after the shell exits.
    let mut stdout = child.stdout.take().expect("piped");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let (mut out, mut buf) = (Vec::new(), [0; 4096]);
        while let Ok(n @ 1..) = std::io::Read::read(&mut stdout, &mut buf) {
            out.extend_from_slice(&buf[..n]);
            if let Some(path) = login_path(&String::from_utf8_lossy(&out)) {
                let _ = tx.send(path.to_string());
                return;
            }
        }
    });
    let path = rx.recv_timeout(Duration::from_secs(3));
    let _ = child.kill();
    let _ = child.wait();
    if let Ok(path) = path {
        std::env::set_var("PATH", path);
    }
}

/// The `PATH` in a login shell's output, once it has printed all of it.
#[cfg(any(target_os = "macos", test))]
fn login_path(out: &str) -> Option<&str> {
    let (before, _) = out.split_once(PATH_END)?;
    let (_, path) = before.rsplit_once(PATH_START)?;
    // Absolute, so a shell that left `$PATH` unexpanded is not taken at its word.
    Some(path).filter(|p| p.starts_with('/'))
}

/// Resolves to the signal number on SIGINT, SIGTERM or SIGHUP (Ctrl-C off Unix).
fn shutdown_signal() -> std::io::Result<impl Future<Output = u8>> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut int = signal(SignalKind::interrupt())?;
        let mut term = signal(SignalKind::terminate())?;
        let mut hup = signal(SignalKind::hangup())?;
        Ok(async move {
            tokio::select! {
                _ = int.recv() => 2,
                _ = term.recv() => 15,
                _ = hup.recv() => 1,
            }
        })
    }
    #[cfg(not(unix))]
    Ok(async {
        let _ = tokio::signal::ctrl_c().await;
        2
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_path_skips_what_startup_and_logout_files_print() {
        let out = format!(
            "welcome {PATH_START} banner\n{PATH_START}/opt/bin:/usr/bin{PATH_END}bye\n"
        );
        assert_eq!(login_path(&out), Some("/opt/bin:/usr/bin"));
        // Not complete yet: more is still to be read.
        assert_eq!(login_path(&format!("{PATH_START}/opt/bin")), None);
        assert_eq!(login_path("no mark"), None);
        assert_eq!(login_path(&format!("{PATH_START}{PATH_END}")), None);
        assert_eq!(login_path(&format!("{PATH_START}$PATH{PATH_END}")), None);
    }
}
