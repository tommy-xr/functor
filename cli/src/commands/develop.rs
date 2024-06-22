use colored::*;
use std::env;
use std::io::{self, BufRead, Error};
use std::path::{Path, PathBuf};

use crate::util::{self, get_nearby_bin, ShellCommand};

pub async fn execute(working_directory: &str) -> io::Result<()> {
    let cwd_path = Path::new(working_directory);
    let build_native_path = Path::new(&"build-native");
    let build_native_wd = cwd_path.join(build_native_path);

    let functor_runner_exe =
        get_nearby_bin(&"functor-runner").expect("functor-runner should be available");

    let target_dir = Path::new(&"target/debug");
    let library_name = libloading::library_filename("game_native");
    let game_lib = build_native_path.join(target_dir.join(Path::new(&library_name)));

    let commands = vec![
        ShellCommand {
            prefix: "[1: Build F#]",
            cmd: "watchexec",
            cwd: working_directory,
            env: vec![],
            args: vec![
                "-e",
                "fs",
                "--no-process-group",
                "--",
                "npm run build:examples:pong:rust",
            ],
        },
        ShellCommand {
            prefix: "[2: Build Rust]",
            cmd: "watchexec",
            cwd: build_native_wd.to_str().unwrap(),
            env: vec![],
            args: vec!["-w", "..", "-e", "rs", "--", "cargo build"],
        },
        ShellCommand {
            prefix: "[3: Functor Runner]",
            cmd: functor_runner_exe.to_str().unwrap(),
            cwd: working_directory,
            env: vec![],
            args: vec!["--hot", "--game-path", game_lib.to_str().unwrap()],
        },
    ];

    util::ShellCommand::run_parallel(commands).await?;

    Ok(())
}
