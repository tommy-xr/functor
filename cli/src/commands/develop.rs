use colored::*;
use std::io::{self, BufRead};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command as TokioCommand;

pub async fn execute(working_directory: &str) -> io::Result<()> {
    // Define the commands to run
    let commands = vec![
        (
            "[1: Build F#]",
            "watchexec",
            vec![
                "-w",
                working_directory,
                "-e",
                "fs",
                "--no-process-group",
                "--",
                "npm run build:examples:pong:rust",
            ],
        ),
        (
            "[2: Build Rust]",
            "watchexec",
            vec!["-w", working_directory, "-e", "rs", "cargo build"],
        ),
    ];

    // Spawn the processes
    let mut handles = vec![];
    for (prefix, cmd, args) in commands {
        let mut command = TokioCommand::new(cmd);
        command
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn()?;
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        // Handle stdout
        let stdout_handle = handle_output(prefix.to_string(), stdout, true);
        handles.push(tokio::spawn(stdout_handle));

        // Handle stderr
        let stderr_handle = handle_output(prefix.to_string(), stderr, false);
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
