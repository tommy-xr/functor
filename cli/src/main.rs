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
        /// also accepted.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        runner_args: Vec<String>,
    },
    Develop {
        #[arg(value_enum)]
        environment: Option<Environment>,
    },
    /// Inspect assets headlessly (no GPU/GL context).
    Inspect {
        #[command(subcommand)]
        target: InspectTarget,
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
    },
}

#[tokio::main]
async fn main() -> tokio::io::Result<()> {
    let args = Args::parse();

    // `inspect` operates on an arbitrary asset path and does not need a game
    // project, so handle it before the functor.json validation below.
    if let Command::Inspect { target } = &args.command {
        let res = match target {
            InspectTarget::Model { path, time } => {
                commands::inspect::execute_model(path, *time).await
            }
        };
        return finish(res);
    }

    let working_directory = get_working_directory(&args);
    let functor_json_path = validate_metadata_path(&working_directory);

    let working_directory_os_str = working_directory.into_os_string();
    let working_directory_str = working_directory_os_str.into_string().unwrap();

    println!("Running command: {:?}", args.command);
    let res = match &args.command {
        Command::Init { template } => {
            // TODO: Handle init
            println!(
                "TODO: Initialize with template '{}' in directory: {}",
                template, &working_directory_str,
            );
            Ok(())
        }
        Command::Build { environment } => {
            commands::build::execute(&working_directory_str, &Environment::default(environment))
                .await
        }
        Command::Run {
            environment,
            runner_args,
        } => {
            commands::build::execute(&working_directory_str, &Environment::default(environment))
                .await?;
            commands::run::execute(
                &working_directory_str,
                &Environment::default(environment),
                runner_args,
            )
            .await
        }
        Command::Develop { environment } => {
            commands::build::execute(&working_directory_str, &Environment::default(environment))
                .await?;
            commands::develop::execute(&working_directory_str, &Environment::default(environment))
                .await
        }
        // Handled earlier (before functor.json validation).
        Command::Inspect { .. } => unreachable!(),
    };

    finish(res)
}

fn finish(res: io::Result<()>) -> tokio::io::Result<()> {
    match res {
        Ok(()) => {
            println!("Done");
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
