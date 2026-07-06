use std::io::{self, Error};
use std::path::Path;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command as TokioCommand;

use crate::output::{emit, Event};

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
            let mut command = TokioCommand::new(&command_spec.cmd);

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

            let status = child.wait().await?;
            if !status.success() {
                let detail = describe_exit(&status);
                // The error bubbles to the CLI's terminal handler, which emits
                // it as an `Error` event — no direct print here.
                return Err(Error::new(
                    io::ErrorKind::Other,
                    format!("{} {}", command_spec.cmd, detail),
                ));
            }
        }

        Ok(())
    }
    pub async fn run_parallel(commands: Vec<ShellCommand<'A>>) -> Result<(), Error> {
        let mut handles = vec![];
        for command_spec in commands {
            let mut command = TokioCommand::new(&command_spec.cmd);

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

fn describe_exit(status: &std::process::ExitStatus) -> String {
    if let Some(code) = status.code() {
        return format!("failed with exit code {}", code);
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            let name = match signal {
                4 => " (SIGILL: illegal instruction)",
                6 => " (SIGABRT: abort — often a Rust panic)",
                8 => " (SIGFPE: arithmetic error)",
                10 => " (SIGBUS: bus error)",
                11 => " (SIGSEGV: segmentation fault)",
                _ => "",
            };
            return format!("terminated by signal {}{}", signal, name);
        }
    }

    "exited abnormally".to_string()
}

async fn handle_output(prefix: String, stream: impl tokio::io::AsyncRead + Unpin, is_stdout: bool) {
    let mut reader = BufReader::new(stream).lines();

    while let Some(line) = reader.next_line().await.unwrap_or(None) {
        let message = format!("{prefix} {line}");
        if is_stdout {
            emit(Event::Info { message });
        } else {
            emit(Event::Warning { message });
        }
    }
}
