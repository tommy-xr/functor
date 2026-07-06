use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;
use std::{env, io, process};

use tokio::macros::*;

mod commands;

pub mod util;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Directory to override the current working directory
    #[arg(short, long)]
    dir: Option<PathBuf>,

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
}

#[derive(Subcommand, Debug)]
enum Command {
    Init {
        #[arg()]
        template: String,
    },
    Build {
        #[arg(value_enum)]
        environment: Option<Environment>,
    },
    Run {
        #[arg(value_enum)]
        environment: Option<Environment>,

        /// Extra arguments forwarded to functor-runner (native only). E.g.
        /// `run native --fixed-time 2 --capture-frame f.png`. A leading `--` is
        /// also accepted. On wasm these are ignored except `--no-open`, which
        /// keeps the dev server but skips launching the browser.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        runner_args: Vec<String>,
    },
    Develop {
        #[arg(value_enum)]
        environment: Option<Environment>,

        /// Extra arguments forwarded to functor-runner (native only). E.g.
        /// `develop native --debug-port 8077`. A leading `--` is also accepted.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        runner_args: Vec<String>,
    },
    /// Inspect assets headlessly (no GPU/GL context).
    Inspect {
        #[command(subcommand)]
        target: InspectTarget,
    },
    /// Push the game's MLE source to a running functor-runner over the
    /// network (POST /reload-source on its debug server) — the remote develop
    /// loop. The runner can be on another machine or device; reloads preserve
    /// the model. MLE projects only.
    Push {
        /// The runner's debug server, host:port. Start the runner with
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

#[tokio::main]
async fn main() -> tokio::io::Result<()> {
    let args = Args::parse();

    // `inspect` operates on an arbitrary asset path and does not need a game
    // project, so handle it before the functor.json validation below.
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
        return finish(res);
    }

    let working_directory = get_working_directory(&args);
    let functor_json_path = validate_metadata_path(&working_directory);

    let working_directory_os_str = working_directory.into_os_string();
    let working_directory_str = working_directory_os_str.into_string().unwrap();

    println!("Running command: {:?}", args.command);

    // An MLE project (functor.json: `"language": "mle"`) routes build/run/
    // develop/push to the interpreter — no Fable, no cargo, hot reload built
    // in. Only those are language-routed; anything else (Init, and Inspect
    // handled earlier) falls through to the normal dispatch.
    let is_routed = matches!(
        &args.command,
        Command::Build { .. }
            | Command::Run { .. }
            | Command::Develop { .. }
            | Command::Push { .. }
    );
    if let Some(project) = commands::mle_project::detect(&working_directory_str)
        .filter(|_| is_routed)
    {
        let res = match &args.command {
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
        return finish(res);
    }

    let res = match &args.command {
        Command::Init { template } => {
            // TODO: Handle init
            println!(
                "TODO: Initialize with template '{}' in directory: {}",
                template, &working_directory_str,
            );
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
    };

    finish(res)
}

fn finish(res: io::Result<()>) -> tokio::io::Result<()> {
    match res {
        Ok(()) => {
            // Status goes to stderr so stdout stays pure data (e.g. JSON from
            // `inspect ... --format json` is directly pipeable).
            eprintln!("Done");
            Ok(())
        }
        Err(error) => {
            eprintln!("Failed: {}", error);
            process::exit(1);
        }
    }
}

fn validate_metadata_path(working_directory: &PathBuf) -> PathBuf {
    let functor_path = working_directory.join("functor.json");

    if functor_path.exists() {
        println!("Found functor.json at {}", functor_path.display());
        // Optional: Read and parse the JSON file
        // let content = fs::read_to_string(&functor_path).expect("Failed to read functor.json");
        // let json: serde_json::Value = serde_json::from_str(&content).expect("Failed to parse functor.json");
        // println!("Content of functor.json: {}", json);
    } else {
        eprintln!("functor.json not found in {}", working_directory.display());
        process::exit(1);
    }

    functor_path
}

fn get_working_directory(args: &Args) -> PathBuf {
    let dir = args
        .dir
        .clone()
        .unwrap_or_else(|| env::current_dir().expect("Failed to get current directory"));
    println!("Hello from directory: {}", dir.display());
    dir
}
