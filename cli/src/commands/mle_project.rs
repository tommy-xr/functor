//! CLI support for MLE-language projects (docs/mle.md Track C4): a
//! `functor.json` with `"language": "mle"` routes `build`/`run`/`develop` to
//! the interpreter instead of the Fable→cargo pipeline.
//!
//! - `build` is the strict gate: parse + lower + typecheck, with `mle check`
//!   diagnostics as **errors** (the runner treats them as warnings so the
//!   dev loop stays permissive; the build command is where they block).
//! - `run` spawns `functor-runner --mle` on the entry file (cwd = the game
//!   dir, so asset paths resolve as usual).
//! - `develop` is `run`: the MLE producer hot-reloads on save by itself — no
//!   watchexec loop, no rebuild. State is preserved across edits.
//! - wasm is not wired yet (docs/mle.md C5).

use std::io::{Error, ErrorKind};
use std::path::{Path, PathBuf};

use crate::util::{self, get_nearby_bin, ShellCommand};
use crate::Environment;

/// The MLE project settings read from `functor.json`.
pub struct MleProject {
    /// The game source, relative to the project dir (default `game.mle`).
    pub entry: String,
}

/// Read `functor.json` and return the MLE project settings when
/// `"language": "mle"` — `None` (the F#/Fable pipeline) otherwise, including
/// for projects whose `functor.json` is empty or has no `language` field.
pub fn detect(working_directory: &str) -> Option<MleProject> {
    let path = Path::new(working_directory).join("functor.json");
    let content = std::fs::read_to_string(path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;
    if json.get("language").and_then(|v| v.as_str()) != Some("mle") {
        return None;
    }
    let entry = json
        .get("entry")
        .and_then(|v| v.as_str())
        .unwrap_or("game.mle")
        .to_string();
    Some(MleProject { entry })
}

impl MleProject {
    fn entry_path(&self, working_directory: &str) -> Result<PathBuf, Error> {
        let path = Path::new(working_directory).join(&self.entry);
        if !path.exists() {
            return Err(Error::new(
                ErrorKind::NotFound,
                format!("mle entry not found: {}", path.display()),
            ));
        }
        Ok(path)
    }

    /// Parse + lower + typecheck the entry; `mle check` diagnostics are
    /// build errors here (see the module doc).
    pub fn build(&self, working_directory: &str) -> Result<(), Error> {
        let path = self.entry_path(working_directory)?;
        let display = path.display().to_string();
        let src = std::fs::read_to_string(&path)?;
        let fail = |span: mle::Span, message: &str| {
            let (line, col) = mle::line_col(&src, span.start);
            Error::other(format!("{display}:{line}:{col}: {message}"))
        };
        let program = mle::parse(&src).map_err(|e| fail(e.span, &e.message))?;
        let module = mle::lower(program).map_err(|e| fail(e.span, &e.message))?;
        let diags = mle::check(&module);
        for diag in &diags {
            let (line, col) = mle::line_col(&src, diag.span.start);
            eprintln!("error: {display}:{line}:{col}: {}", diag.message);
        }
        if diags.is_empty() {
            println!("[mle] {display}: ok");
            Ok(())
        } else {
            Err(Error::other(format!(
                "{} type error(s) in {display}",
                diags.len()
            )))
        }
    }

    /// Spawn the runner on the entry (`run` and `develop` — hot reload is
    /// built into the producer, so there is no separate watch loop).
    pub async fn run(
        &self,
        working_directory: &str,
        environment: &Environment,
        runner_args: &[String],
        develop: bool,
    ) -> Result<(), Error> {
        if matches!(environment, Environment::Wasm) {
            return Err(Error::other(
                "mle on wasm is not wired yet (docs/mle.md Track C5) — use `run native`",
            ));
        }
        let entry = self.entry_path(working_directory)?;
        let functor_runner_exe =
            get_nearby_bin(&"functor-runner").expect("functor-runner should be available");
        if develop {
            println!("[mle] develop: hot reload is built in — edit {} and save", self.entry);
        }
        // The runner's cwd is the project dir, so the entry passes through
        // as-is — `src/game.mle` keeps its subdirectory.
        let _ = entry; // existence validated above
        let mut args = vec!["--mle", "--game-path", self.entry.as_str()];
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

    /// Push the entry source to a running runner's `POST /reload-source`
    /// (its debug server) — once, or on every save with `watch`. The runner
    /// validates the pushed source and keeps its old program on a broken
    /// push, so errors come back as the 400 body and the watch loop just
    /// keeps watching. A transport failure (runner not up yet, cable out)
    /// retries on the next poll rather than losing the edit.
    pub async fn push(
        &self,
        working_directory: &str,
        addr: &str,
        watch: bool,
    ) -> Result<(), Error> {
        let path = self.entry_path(working_directory)?;
        if !watch {
            let src = std::fs::read_to_string(&path)?;
            return match post_reload_source(addr, &src).map_err(|e| {
                Error::other(format!(
                    "cannot reach http://{addr}/reload-source: {e} — is the runner up \
with --debug-port (and --debug-bind 0.0.0.0 if remote)?"
                ))
            })? {
                (200, body) => {
                    println!("[mle] {body}");
                    Ok(())
                }
                (status, body) => Err(Error::other(format!("push rejected ({status}): {body}"))),
            };
        }

        println!(
            "[mle] watching {} — pushing to http://{addr}/reload-source on save (Ctrl-C to stop)",
            self.entry
        );
        // Track the last content ATTEMPTED, not the file mtime: coarse-mtime
        // filesystems can miss rapid saves, and atomic-save editors briefly
        // unlink the file mid-save (a failed read here just waits for the
        // next poll). A rejected push records its content too — that
        // revision's verdict is in; wait for the next edit.
        let mut attempted: Option<String> = None;
        loop {
            if let Ok(src) = std::fs::read_to_string(&path) {
                if attempted.as_deref() != Some(src.as_str()) {
                    match post_reload_source(addr, &src) {
                        Ok((200, body)) => {
                            println!("[mle] {body}");
                            attempted = Some(src);
                        }
                        Ok((status, body)) => {
                            eprintln!("[mle] push rejected ({status}): {body}");
                            attempted = Some(src);
                        }
                        // Transport failure: leave `attempted` unset so the
                        // same content retries on the next poll.
                        Err(e) => eprintln!("[mle] push failed ({e}); retrying…"),
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
    }
}

/// Minimal HTTP POST over std::net — one dependency-free request to the
/// runner's tiny_http server. Returns (status, body). `Connection: close`
/// keeps the read side trivial (read to EOF, split headers off).
fn post_reload_source(addr: &str, source: &str) -> Result<(u16, String), Error> {
    use std::io::{Read, Write};
    use std::net::ToSocketAddrs;
    let timeout = std::time::Duration::from_secs(5);
    // connect_timeout, not connect: a blackholed host must fail in 5s, not
    // the OS's ~75s TCP give-up.
    let sockaddr = addr
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| Error::other(format!("cannot resolve {addr}")))?;
    let mut stream = std::net::TcpStream::connect_timeout(&sockaddr, timeout)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    write!(
        stream,
        "POST /reload-source HTTP/1.1\r\nHost: {addr}\r\nContent-Type: text/plain\r\n\
Content-Length: {}\r\nConnection: close\r\n\r\n",
        source.len()
    )?;
    stream.write_all(source.as_bytes())?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    let status = response
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| Error::other(format!("malformed HTTP response: {response:.80}")))?;
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.trim().to_string())
        .unwrap_or_default();
    Ok((status, body))
}
