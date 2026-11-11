//! `functor-app`: the desktop app's shell around `functor-app-core`.

mod server;
mod session;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use clap::Parser;
use functor_app_core::protocol::ClientMsg;
use functor_app_core::{print, update, AppState, Event, SessionStatus};
use tokio::sync::mpsc;

use session::Driver;

/// How long `--start-session` waits for the first `/state`.
const START_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Parser, Debug)]
#[command(about = "The Functor desktop app")]
struct Args {
    /// Start directory for project discovery (defaults to the current directory).
    #[arg(short, long)]
    dir: Option<PathBuf>,

    /// Run without a window.
    #[arg(long)]
    headless: bool,

    /// Print the state as text and exit.
    #[arg(long)]
    print: bool,

    /// With --print: start the active project's game session, print once it
    /// answers `/state`, then stop it.
    #[arg(long, requires = "print")]
    start_session: bool,

    /// Run game sessions without a GPU (`--headless` instead of `--hidden`).
    #[arg(long)]
    no_gpu: bool,

    /// The `functor` binary for game sessions (else `FUNCTOR_BIN`, else one
    /// next to this executable, else `functor` on PATH).
    #[arg(long, value_name = "PATH")]
    functor: Option<PathBuf>,

    /// The port to serve the UI on, on 127.0.0.1 (0 picks a free one).
    #[arg(long, default_value_t = 0, conflicts_with = "print")]
    port: u16,

    /// Serve the UI from this directory instead of the one built in.
    #[arg(long, value_name = "PATH", conflicts_with = "print")]
    ui_dir: Option<PathBuf>,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<ExitCode> {
    let args = Args::parse();
    if !args.headless {
        eprintln!("functor-app: only --headless is supported yet");
        return Ok(ExitCode::from(2));
    }
    let start = std::fs::canonicalize(args.dir.unwrap_or_else(|| PathBuf::from(".")))?;
    let mut state = functor_app_core::discover(&start);
    state.no_gpu = args.no_gpu;
    if args.print && !args.start_session {
        print!("{}", print(&state));
        return Ok(ExitCode::SUCCESS);
    }
    let functor = session::functor_bin(
        args.functor,
        std::env::var_os("FUNCTOR_BIN"),
        std::env::current_exe().ok(),
    );
    if args.print {
        return print_session(state, functor).await;
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
    let (tx, events) = mpsc::unbounded_channel();
    let driver = Driver::new(functor, tx);
    server::serve(state, driver, events, args.port, ui, shutdown).await
}

/// `--start-session`: start, print once polled, stop. Exits 1 if the session
/// fails or never answers, 128 + n on signal n, after killing the child.
async fn print_session(mut state: AppState, functor: PathBuf) -> std::io::Result<ExitCode> {
    // Handlers go in before the child exists, so no signal can skip its kill.
    let shutdown = shutdown_signal()?;
    tokio::pin!(shutdown);
    let (tx, mut events) = mpsc::unbounded_channel();
    let mut driver = Driver::new(functor, tx);
    state = step(state, Event::Client(ClientMsg::StartSession), &mut driver).await;
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
            Some(event) = events.recv() => state = step(state, event, &mut driver).await,
            _ = &mut deadline => {
                let msg = format!("no answer from /state within {}s", START_TIMEOUT.as_secs());
                break Err((1, msg));
            }
        }
    };
    // Printed after the stop: a blocked stdout must not keep the child alive.
    let dump = print(&state);
    let tail = driver.output_tail();
    step(state, Event::Client(ClientMsg::StopSession), &mut driver).await;
    print!("{dump}");
    Ok(match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err((code, msg)) => {
            eprintln!("functor-app: {msg}");
            if !tail.is_empty() {
                eprintln!("functor-app: the game's last output lines:");
                for line in tail {
                    eprintln!("  {line}");
                }
            }
            ExitCode::from(code)
        }
    })
}

async fn step(state: AppState, event: Event, driver: &mut Driver) -> AppState {
    let (state, commands) = update(state, event);
    for command in commands {
        driver.run(command).await;
    }
    state
}

/// Resolves to the signal number on SIGINT, SIGTERM or SIGHUP (Ctrl-C off Unix).
fn shutdown_signal() -> std::io::Result<impl std::future::Future<Output = u8>> {
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
