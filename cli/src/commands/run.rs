use std::io::Error;
use std::path::Path;

use crate::util::{self, get_nearby_bin, ShellCommand, WasmDevServer};
use crate::Environment;

pub async fn execute(
    working_directory: &str,
    environment: &Environment,
    runner_args: &[String],
) -> Result<(), Error> {
    let cwd_path = Path::new(working_directory);
    let build_wasm_path = Path::new(&"build-wasm");
    let build_wasm_wd = cwd_path.join(build_wasm_path);

    let build_native_path = Path::new(&"build-native");
    let build_native_wd = cwd_path.join(build_native_path);

    let functor_runner_exe =
        get_nearby_bin(&"functor-runner").expect("functor-runner should be available");

    let target_dir = Path::new(&"target/debug");
    let library_name = libloading::library_filename("game_native");
    let game_lib = build_native_path.join(target_dir.join(Path::new(&library_name)));

    match environment {
        Environment::Native => {
            let mut args = vec!["--game-path", game_lib.to_str().unwrap()];
            args.extend(runner_args.iter().map(|s| s.as_str()));
            let commands = vec![ShellCommand {
                prefix: "[Functor Runner]",
                cmd: functor_runner_exe.to_str().unwrap(),
                cwd: working_directory,
                env: vec![],
                args,
            }];
            util::ShellCommand::run_sequential(commands).await
        }
        Environment::Wasm => {
            // `--no-open` keeps the dev server but skips launching the system
            // browser — for headless/scripted runs (e.g. the Playwright golden,
            // which drives its own browser; otherwise every run pops a stray
            // tab). Other args are still ignored on wasm.
            let no_open = runner_args.iter().any(|a| a == "--no-open");
            let ignored: Vec<&str> = runner_args
                .iter()
                .filter(|a| a.as_str() != "--no-open")
                .map(|s| s.as_str())
                .collect();
            if !ignored.is_empty() {
                eprintln!(
                    "warning: ignoring runner args (not supported for wasm): {}",
                    ignored.join(" ")
                );
            }

            let wasm_server_start = WasmDevServer::start(working_directory);
            if no_open {
                println!(
                    "[Functor] wasm server at http://127.0.0.1:8080 (--no-open: skipping browser)"
                );
            } else {
                let cmd = if std::env::consts::OS == "windows" {
                    "start"
                } else {
                    "open"
                };
                let commands = vec![ShellCommand {
                    prefix: "[Open Browser]",
                    cmd,
                    cwd: working_directory,
                    env: vec![],
                    args: vec!["http://127.0.0.1:8080"],
                }];
                util::ShellCommand::run_sequential(commands).await?;
            }
            wasm_server_start.await
        }
    }
}
