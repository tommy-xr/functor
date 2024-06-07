use colored::*;
use std::io::{self, BufRead};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command as TokioCommand;

pub async fn execute(working_directory: &str) -> io::Result<()> {
    let cwd_path = Path::new(working_directory);
    let build_native_path = Path::new(&"build-native");
    let build_native_wd = cwd_path.join(build_native_path);

    // Define the commands to run
    let commands = vec![
        (
            "[1: Build F#]",
            working_directory,
            "watchexec",
            vec![
                "-e",
                "fs",
                "--no-process-group",
                "--",
                "npm run build:examples:pong:rust",
            ],
        ),
        (
            "[2: Build Rust]",
            build_native_wd.to_str().unwrap(),
            "watchexec",
            vec!["-w", "..", "-e", "rs", "--", "cargo build"],
        ),
    ];

    // Spawn the processes
    let mut handles = vec![];
    for (prefix, cwd, cmd, args) in commands {
        println!("Using working dir: {}", cwd);
        let mut command = TokioCommand::new(cmd);
        command
            .current_dir(Path::new(cwd))
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
