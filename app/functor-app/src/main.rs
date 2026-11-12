//! `functor-app`: the desktop app's shell around `functor-app-core`.

mod agent;
mod changes;
mod server;
mod session;
mod store;

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use clap::{Parser, Subcommand};
use functor_app_core::protocol::{ClientMsg, ServerMsg};
use functor_app_core::{
    print, print_change, print_item, update, AppState, Command, Event, Harness, Item, SessionStatus,
};
use tokio::sync::mpsc::{self, UnboundedReceiver};

use agent::Agents;
use session::Driver;

/// How long `--start-session` waits for the first `/state`.
const START_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Parser, Debug)]
#[command(about = "The Functor desktop app")]
struct Args {
    /// Start directory for project discovery (defaults to the current directory).
    #[arg(short, long, global = true)]
    dir: Option<PathBuf>,

    /// Run without a window.
    #[arg(long, global = true)]
    headless: bool,

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
    },
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<ExitCode> {
    let args = Args::parse();
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
    if !args.headless {
        eprintln!("functor-app: only --headless is supported yet");
        return Ok(ExitCode::from(2));
    }
    let start = std::fs::canonicalize(args.dir.unwrap_or_else(|| PathBuf::from(".")))?;
    let mut state = functor_app_core::discover(&start);
    state.no_gpu = args.no_gpu;
    let Some(home) = store::home(std::env::var_os("FUNCTOR_APP_HOME"), std::env::home_dir()) else {
        eprintln!("functor-app: no home directory; set FUNCTOR_APP_HOME");
        return Ok(ExitCode::from(2));
    };
    if let Err(e) = store::make_private(&home) {
        eprintln!("functor-app: cannot make {} private: {e}", home.display());
    }
    let (tasks, highest) = store::load(&home, &state);
    state = update(state, Event::TasksLoaded { tasks }).0;
    if args.print && !args.start_session {
        print!("{}", print(&state));
        return Ok(ExitCode::SUCCESS);
    }
    let functor = session::functor_bin(
        args.functor,
        std::env::var_os("FUNCTOR_BIN"),
        std::env::current_exe().ok(),
    );
    // Absolute: the fake agent's child runs in the project directory.
    let agents = match std::env::var_os("FUNCTOR_APP_FAKE_AGENT").filter(|f| !f.is_empty()) {
        Some(fixture) => Agents::Fake(std::path::absolute(fixture)?),
        None => Agents::Real,
    };
    state.harnesses = agents.available();
    state.harness_choice = store::load_harnesses(&home, &state);
    // `--print` changes no task, so it measures none.
    let git = match args.print {
        true => None,
        false => changes::Git::open(Path::new(&state.repo_root), &home).map(Arc::new),
    };
    state.tracks_changes = git.is_some();
    let (tx, events) = mpsc::unbounded_channel();
    let driver = Driver::new(functor, agents, home, highest, git, tx);
    if args.print {
        return print_session(state, driver, events).await;
    }
    if let Some(Cmd::Task {
        prompt,
        harness,
        follow,
        start_session,
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
        };
        return run_task(state, driver, events, task).await;
    }
    state.git = server::git_info(Path::new(&state.repo_root));
    let ui = match args.ui_dir {
        Some(dir) if !dir.join("index.html").is_file() => {
            eprintln!("functor-app: no index.html in --ui-dir {}", dir.display());
            return Ok(ExitCode::from(2));
        }
        Some(dir) => server::Ui::Dir(dir),
        None => server::Ui::Embedded,
    };
    let shutdown = shutdown_signal()?;
    server::serve(state, driver, events, args.port, ui, shutdown).await
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
            Some(SessionStatus::Running { frame: Some(_), .. }) => break Ok(()),
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
    let new = driver.new_task(args.prompt, args.harness);
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
    let signal = loop {
        if args.follow {
            printed = print_settled(&transcript(&state), printed);
        }
        if state.tasks.iter().any(|t| t.id == id && t.run.is_none()) {
            break None;
        }
        tokio::select! {
            biased;
            signal = &mut shutdown => break Some(signal),
            Some(event) = events.recv() => state = step(state, event, &mut driver).await,
        }
    };
    // Stops the run (on a signal) and the session.
    state = step(state, Event::Shutdown, &mut driver).await;
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

async fn step(state: AppState, event: Event, driver: &mut Driver) -> AppState {
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
