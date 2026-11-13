//! `functor-app`: the desktop app's shell around `functor-app-core`.

mod agent;
mod changes;
mod creation;
mod evidence;
mod files;
mod folders;
mod link;
mod server;
mod session;
mod store;
mod stub;
mod voice;
#[cfg(target_os = "macos")]
mod window;

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};
use functor_app_core::metrics::run_metrics;
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

    /// The `functor` binary for game sessions and agent tools (else
    /// `FUNCTOR_BIN`, else one next to this executable, else `functor` on PATH).
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

fn main() -> std::io::Result<ExitCode> {
    // First, while this is the only thread: it edits the environment.
    let token = server::Token::take();
    let mut args = Args::parse();
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
        return runtime()?.block_on(app(args, token, None));
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
            runtime()?.block_on(app(args, token, Some(link)))
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
    let functor = session::functor_bin(
        args.functor,
        std::env::var_os("FUNCTOR_BIN"),
        std::env::current_exe().ok(),
    );
    // Absolute, as children run in project directories; a bare name is
    // still looked up on PATH.
    let functor = match functor.components().count() {
        1 => functor,
        _ => std::path::absolute(functor)?,
    };
    // Absolute: the fake agent's child runs in the project directory.
    let agents = match std::env::var_os("FUNCTOR_APP_FAKE_AGENT").filter(|f| !f.is_empty()) {
        Some(fixture) => Agents::Fake(std::path::absolute(fixture)?),
        None => Agents::Real,
    };
    let mut state = AppState::none();
    state.no_gpu = args.no_gpu;
    state.harnesses = agents.available();
    state.native_picker = window.is_some();
    state.recents = store::load_recents(&home);
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
    let recents = if interactive { &state.recents[..] } else { &[] };
    let (tx, events) = mpsc::unbounded_channel();
    let hub = link::Hub::new(home.clone(), tx.clone())?;
    state.speech = hub.speaks();
    let views = std::sync::Arc::new(agent::ViewRoute::default());
    let mut driver = Driver::new(functor, agents, home, tx.clone(), views.clone());
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
        follow,
        start_session,
        metrics,
        max_input,
    }) = args.command
    {
        let harness = match harness.as_str() {
            "codex" => Harness::Codex,
            _ => Harness::Claude,
        };
        let task = TaskArgs {
            prompt,
            harness,
            follow,
            start_session,
            metrics,
            max_input,
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
    views.listen(&origin);
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
    follow: bool,
    start_session: bool,
    metrics: Option<PathBuf>,
    max_input: Option<u64>,
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
    let new = driver.new_task(args.prompt, args.harness, None);
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
    let signal = loop {
        if args.follow {
            printed = print_settled(&transcript(&state), printed);
        }
        let task = state.tasks.iter().find(|t| t.id == id).expect("the task");
        let Some(run) = &task.run else {
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
    let wall = started.elapsed();
    // Stops the run (on a signal or a cut) and the session.
    state = step(state, Event::Shutdown, &mut driver).await;
    if let Some(path) = &args.metrics {
        let task = state.tasks.iter().find(|t| t.id == id).expect("the task");
        let mut record = serde_json::to_value(run_metrics(task)).expect("metrics serialize");
        record["wall_ms"] = (wall.as_millis() as u64).into();
        record["first_event_ms"] = first_event.map(|d| d.as_millis() as u64).into();
        record["cut"] = cut.or(signal.map(|s| format!("signal {s}"))).into();
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

async fn step(mut state: AppState, event: Event, driver: &mut Driver) -> AppState {
    state.now = server::wall_ms();
    let (state, commands) = update(state, event);
    for command in commands {
        driver.run(command).await;
    }
    state
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
