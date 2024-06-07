use colored::*;
use std::env;
use std::io::{self, BufRead, Error};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command as TokioCommand;

struct CommandSpec<'a> {
    prefix: &'a str,
    cmd: &'a str,
    cwd: &'a str,
    args: Vec<&'a str>,
    env: Vec<(&'a str, &'a str)>,
}

pub async fn execute(working_directory: &str) -> io::Result<()> {
    let cwd_path = Path::new(working_directory);
    let build_native_path = Path::new(&"build-native");
    let build_native_wd = cwd_path.join(build_native_path);

    let functor_runner_exe =
        get_nearby_bin(&"functor-runner").expect("functor-runner should be available");

    let target_dir = build_native_path.join(Path::new(&"target/debug"));

    let commands = vec![
        CommandSpec {
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
        CommandSpec {
            prefix: "[2: Build Rust]",
            cmd: "watchexec",
            cwd: build_native_wd.to_str().unwrap(),
            env: vec![],
            args: vec!["-w", "..", "-e", "rs", "--", "cargo build"],
        },
        CommandSpec {
            prefix: "[3: Functor Runner]",
            cmd: functor_runner_exe.to_str().unwrap(),
            cwd: build_native_wd.to_str().unwrap(),
            env: vec![],
            args: vec![],
        },
    ];

    // Spawn the processes
    let mut handles = vec![];
    for command_spec in commands {
        println!("Using working dir: {}", &command_spec.cwd);
        let mut command = TokioCommand::new(&command_spec.cmd);
        println!("-- Running command: {}", &command_spec.cmd);

        // Set environment variables if any
        for (key, value) in command_spec.env {
            command.env(key, value);
        }

        command
            .current_dir(Path::new(&command_spec.cwd))
            .args(&command_spec.args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn()?;
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        // Handle stdout
        let stdout_handle = handle_output(command_spec.prefix.to_string(), stdout, true);
        handles.push(tokio::spawn(stdout_handle));

        // Handle stderr
        let stderr_handle = handle_output(command_spec.prefix.to_string(), stderr, false);
        handles.push(tokio::spawn(stderr_handle));
    }

    // Wait for all processes to finish
    for handle in handles {
        handle.await?;
    }

    Ok(())
}
async fn handle_output(prefix: String, stream: impl tokio::io::AsyncRead + Unpin, is_stdout: bool) {
    let mut reader = BufReader::new(stream).lines();

    while let Some(line) = reader.next_line().await.unwrap_or(None) {
        let colored_prefix = if is_stdout {
            prefix.blue()
        } else {
            prefix.red()
        };

        println!("{}: {}", colored_prefix, line);
    }
}

fn get_nearby_bin(file: &str) -> io::Result<PathBuf> {
    let curent_exe = env::current_exe()?;

    let parent = curent_exe.parent().unwrap();

    let ret = parent.join(&file);

    Ok(ret)
}
