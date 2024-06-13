use colored::*;
use std::env;
use std::io::{self, BufRead, Error};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command as TokioCommand;

pub struct ShellCommand<'a> {
    pub prefix: &'a str,
    pub cmd: &'a str,
    pub cwd: &'a str,
    pub args: Vec<&'a str>,
    pub env: Vec<(&'a str, &'a str)>,
}

impl<'A> ShellCommand<'A> {
    pub async fn run_sequential(commands: Vec<ShellCommand<'A>>) -> Result<(), Error> {
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

            // Handle stderr
            let stderr_handle = handle_output(command_spec.prefix.to_string(), stderr, false);

            // Wait for process to finish
            tokio::spawn(stdout_handle).await?;
            tokio::spawn(stderr_handle).await?;
        }

        Ok(())
    }
    pub async fn run_parallel(commands: Vec<ShellCommand<'A>>) -> Result<(), Error> {
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
