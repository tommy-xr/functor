use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;
use std::time::Instant;
use std::{env, io, process};

mod commands;
mod output;

pub mod util;

use output::{emit, Event};

/// Functor — a functional toolkit for building 3D games in MLE.
///
/// Operates on a project directory (a `functor.json` with
/// `"language": "mle"`). Add `--json` to any command for a newline-delimited
/// JSON event stream instead of human text (see `docs/cli-output.md`).
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Project directory (defaults to the current working directory).
    #[arg(short, long, global = true)]
    dir: Option<PathBuf>,

    /// Emit newline-delimited JSON (one event per line) instead of human text.
    #[arg(long, global = true)]
    json: bool,

    /// Print only errors and the final status.
    #[arg(long, global = true)]
    quiet: bool,

    /// Disable ANSI color even on an interactive terminal.
    #[arg(long, global = true)]
    no_color: bool,

    /// Use ASCII-only glyphs (auto-detected on a dumb / non-UTF-8 terminal).
    #[arg(long, global = true)]
    ascii: bool,

    /// Show debug/info logs (default: warnings + errors only). `RUST_LOG=<level>`
    /// overrides.
    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(ValueEnum, Clone, Debug)]
enum Environment {
    Wasm,
    Native,
}

impl Environment {
    fn default(maybe_env: &Option<Environment>) -> Environment {
        maybe_env.clone().unwrap_or(Environment::Native)
    }

    fn as_str(&self) -> &'static str {
        match self {
            Environment::Wasm => "wasm",
            Environment::Native => "native",
        }
    }
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Scaffold a new project from a template (not yet implemented).
    Init {
        #[arg()]
        template: String,
    },
    /// Typecheck the MLE project (the strict build gate — diagnostics are
    /// errors). Target-independent. E.g. `functor -d examples/primitives build`.
    Build {
        #[arg(value_enum)]
        environment: Option<Environment>,
    },
    /// Run the game (default `native`, an OpenGL window; `wasm` serves a dev
    /// server). E.g. `functor -d examples/primitives run native`.
    Run {
        #[arg(value_enum)]
        environment: Option<Environment>,

        /// Extra arguments forwarded to the in-process desktop runtime (native
        /// only). E.g. `run native --fixed-time 2 --capture-frame f.png`. A
        /// leading `--` is also accepted. On wasm these are ignored except
        /// `--no-open`, which keeps the dev server but skips launching the browser.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        runner_args: Vec<String>,
    },
    /// Run with hot-reload (same as `run`; MLE reloads on save). E.g.
    /// `functor -d examples/lighting develop native`.
    Develop {
        #[arg(value_enum)]
        environment: Option<Environment>,

        /// Extra arguments forwarded to the in-process desktop runtime (native
        /// only). E.g. `develop native --debug-port 8077`. A leading `--` is
        /// also accepted.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        runner_args: Vec<String>,
    },
    /// Inspect assets headlessly (no GPU/GL context).
    Inspect {
        #[command(subcommand)]
        target: InspectTarget,
    },
    /// Push the game's MLE source to a running runtime over the network
    /// (POST /reload-source on its debug server) — the remote develop loop.
    /// The runtime can be on another machine or device; reloads preserve the
    /// model. MLE projects only.
    Push {
        /// The runtime's debug server, host:port. Start it with
        /// `--debug-port <PORT>` (plus `--debug-bind 0.0.0.0` when remote).
        addr: String,

        /// Keep watching the entry file and re-push on every save,
        /// instead of pushing once and exiting.
        #[arg(long)]
        watch: bool,
    },
}

#[derive(Subcommand, Debug)]
enum InspectTarget {
    /// Load a glTF/glb model and print a CPU-side text report.
    Model {
        /// Path to the .glb / .gltf file.
        #[arg()]
        path: String,

        /// Sample the skinned AABB at this time (seconds) for animated models.
        #[arg(long)]
        time: Option<f32>,

        /// Animation to sample for the skinned AABB, by name or index. Defaults
        /// to the first animation. Implies sampling even without --time (at t=0).
        #[arg(long)]
        animation: Option<String>,

        /// Output format for the report.
        #[arg(long, value_enum, default_value_t = commands::inspect::OutputFormat::Text)]
        format: commands::inspect::OutputFormat,
    },
}

// Must stay the default MULTI-THREAD runtime: `run native` drives the desktop
// runtime's GLFW loop in-process by BLOCKING this (main-thread) `block_on`
// future, while the debug server / HTTP / WebSocket work runs on `tokio::spawn`
// worker threads. A `current_thread` flavor would starve those tasks while the
// GL loop blocks (headless `--debug-port` would hang). See
// `functor_runtime_desktop::run`.
#[tokio::main]
async fn main() -> tokio::io::Result<()> {
    let started = Instant::now();
    let args = Args::parse();

    output::init(args.json, args.quiet, args.no_color, args.ascii, args.verbose);

    // When the live (ink-style) renderer is up, a Ctrl-C would otherwise kill
    // the process mid-draw and leave the sticky live region stranded on screen.
    // Arm a handler that wipes it and restores the terminal first. Only the live
    // path is affected — the plain/json signal behavior is unchanged.
    if output::live_active() {
        tokio::spawn(async {
            if tokio::signal::ctrl_c().await.is_ok() {
                output::cleanup();
                process::exit(130);
            }
        });
    }

    // `inspect` is a DATA command: it prints a report (its own `--format
    // text|json` is its dual mode) to stdout, so it bypasses the event stream
    // to keep that stdout payload pure. It also runs before functor.json
    // validation, since it operates on an arbitrary asset path.
    if let Command::Inspect { target } = &args.command {
        let res = match target {
            InspectTarget::Model {
                path,
                time,
                animation,
                format,
            } => {
                commands::inspect::execute_model(
                    path,
                    *time,
                    animation.as_deref(),
                    format.clone(),
                )
                .await
            }
        };
        return finish_inspect(res);
    }

    finish(run(&args).await, started)
}

/// Emit `CommandStarted`, validate the project, dispatch, and return the
/// command's result. All user-facing output flows through [`emit`].
async fn run(args: &Args) -> io::Result<()> {
    let working_directory = get_working_directory(args);
    let working_directory_str = working_directory
        .clone()
        .into_os_string()
        .into_string()
        .map_err(|_| io::Error::other("project directory path is not valid UTF-8"))?;

    emit(Event::CommandStarted {
        command: command_name(&args.command).to_string(),
        project: Some(working_directory_str.clone()),
        env: command_env(&args.command),
    });

    validate_metadata_path(&working_directory)?;

    // An MLE project (functor.json: `"language": "mle"`) routes build/run/
    // develop/push to the interpreter — no Fable, no cargo, hot reload built
    // in. Only those are language-routed; Init falls through below.
    let is_routed = matches!(
        &args.command,
        Command::Build { .. }
            | Command::Run { .. }
            | Command::Develop { .. }
            | Command::Push { .. }
    );
    if let Some(project) =
        commands::mle_project::detect(&working_directory_str).filter(|_| is_routed)
    {
        return match &args.command {
            Command::Init { .. } | Command::Inspect { .. } => unreachable!("is_routed excludes"),
            // `build` is target-independent for MLE: the strict typecheck
            // gate is the whole build — nothing compiles for either target
            // (native interprets the file; wasm serves it as text).
            Command::Build { .. } => project.build(&working_directory_str),
            Command::Run {
                environment,
                runner_args,
            } => {
                project.build(&working_directory_str)?;
                project
                    .run(
                        &working_directory_str,
                        &Environment::default(environment),
                        runner_args,
                        false,
                    )
                    .await
            }
            Command::Develop {
                environment,
                runner_args,
            } => {
                project.build(&working_directory_str)?;
                project
                    .run(
                        &working_directory_str,
                        &Environment::default(environment),
                        runner_args,
                        true,
                    )
                    .await
            }
            Command::Push { addr, watch } => {
                project.push(&working_directory_str, addr, *watch).await
            }
        };
    }

    match &args.command {
        Command::Init { template } => {
            // TODO: Handle init (currently a stub — see docs/todo.md).
            emit(Event::Info {
                message: format!(
                    "init is not yet implemented (template '{template}', directory {working_directory_str})"
                ),
            });
            Ok(())
        }
        // The F#/Fable pipeline was removed in E3: every Functor project is now
        // MLE (functor.json `"language": "mle"`), routed above. A project that
        // isn't MLE has no build/run/develop/push path.
        Command::Build { .. } | Command::Run { .. } | Command::Develop { .. } => {
            Err(io::Error::other(
                "not an MLE project: functor.json needs \"language\": \"mle\" \
(the F#/Fable pipeline was removed in E3)",
            ))
        }
        Command::Push { .. } => Err(io::Error::other(
            "push requires an MLE project (functor.json with \"language\": \"mle\")",
        )),
        // Handled earlier (before functor.json validation).
        Command::Inspect { .. } => unreachable!(),
    }
}

fn command_name(command: &Command) -> &'static str {
    match command {
        Command::Init { .. } => "init",
        Command::Build { .. } => "build",
        Command::Run { .. } => "run",
        Command::Develop { .. } => "develop",
        Command::Inspect { .. } => "inspect",
        Command::Push { .. } => "push",
    }
}

fn command_env(command: &Command) -> Option<String> {
    match command {
        Command::Run { environment, .. } | Command::Develop { environment, .. } => {
            Some(Environment::default(environment).as_str().to_string())
        }
        _ => None,
    }
}

/// Terminal handler for the routed commands: emit the final status through the
/// event stream, and exit non-zero on error.
fn finish(res: io::Result<()>, started: Instant) -> tokio::io::Result<()> {
    let duration_ms = started.elapsed().as_millis() as u64;
    match res {
        Ok(()) => {
            emit(Event::CommandFinished {
                ok: true,
                duration_ms,
            });
            Ok(())
        }
        Err(error) => {
            let message = error.to_string();
            let hint = hint_for(&message);
            emit(Event::Error { message, hint });
            emit(Event::CommandFinished {
                ok: false,
                duration_ms,
            });
            process::exit(1);
        }
    }
}

/// An actionable hint for the common, recognizable CLI failures — matched on the
/// (locally-defined) error message. Targeted on purpose: most errors have no
/// useful generic advice, so they get none.
fn hint_for(message: &str) -> Option<String> {
    if message.contains("functor.json not found") {
        Some(
            "point -d at an MLE project directory (one containing a functor.json), \
e.g. `functor -d examples/primitives build`"
                .to_string(),
        )
    } else if message.contains("not an MLE project") {
        Some("add `\"language\": \"mle\"` to the project's functor.json".to_string())
    } else if message.contains("mle entry not found") {
        Some("check the `entry` field in functor.json (defaults to game.mle)".to_string())
    } else {
        None
    }
}

/// Terminal handler for `inspect` — a data command that owns stdout with its
/// report, so it stays off the event stream. Errors go to stderr.
fn finish_inspect(res: io::Result<()>) -> tokio::io::Result<()> {
    match res {
        Ok(()) => Ok(()),
        Err(error) => {
            eprintln!("error: {error}");
            process::exit(1);
        }
    }
}

/// Validate that the project directory has a `functor.json`.
fn validate_metadata_path(working_directory: &PathBuf) -> io::Result<()> {
    if working_directory.join("functor.json").exists() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "functor.json not found in {}",
            working_directory.display()
        )))
    }
}

fn get_working_directory(args: &Args) -> PathBuf {
    args.dir
        .clone()
        .unwrap_or_else(|| env::current_dir().expect("Failed to get current directory"))
}
