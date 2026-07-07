//! CLI support for MLE-language projects (docs/mle.md Track C4): a
//! `functor.json` with `"language": "mle"` routes `build`/`run`/`develop` to
//! the interpreter instead of the Fable→cargo pipeline.
//!
//! - `build` is the strict gate: parse + lower + typecheck, with `mle check`
//!   diagnostics as **errors** (the runner treats them as warnings so the
//!   dev loop stays permissive; the build command is where they block).
//! - `run` drives the desktop runtime's run loop IN-PROCESS on the entry file
//!   (cwd = the game dir, so asset paths resolve as usual) — post-E3 there is a
//!   single `functor` binary, no separate runner child process.
//! - `develop` is `run`: the MLE producer hot-reloads on save by itself — no
//!   watchexec loop, no rebuild. State is preserved across edits.
//! - `run wasm` serves the project with the MLE index page (docs/mle.md C5):
//!   nothing compiles — the `.mle` source ships as text, fetched and
//!   interpreted by the embedded web runtime. Hot reload is native-only;
//!   reload the page to pick up edits.

use std::io::{Error, ErrorKind};
use std::path::{Path, PathBuf};

use clap::Parser;

use crate::output::{emit, Event, Severity};
use crate::util::{self, ShellCommand, WasmDevServer};
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

    /// Load the project (B8: the entry plus every sibling `.mle` file —
    /// file = module) and typecheck the whole program; `mle check`
    /// diagnostics are build errors here (see the module doc).
    pub fn build(&self, working_directory: &str) -> Result<(), Error> {
        let path = self.entry_path(working_directory)?;
        let display = path.display().to_string();
        // A load failure (parse error, bad module name, cycle) is a positioned
        // diagnostic too — surface its file:line:col structurally, like a check
        // error, rather than flattening it into the final text-only error.
        let project = match mle::project::load(&path) {
            Ok(project) => project,
            Err(e) => {
                // A load error (parse / bad module / cycle) carries only its
                // position; re-read the file to recover the offending line for
                // the caret. Missing/short file → no source line (fail soft).
                let source_line = std::fs::read_to_string(&e.path)
                    .ok()
                    .and_then(|src| nth_line(&src, e.line));
                emit(Event::Diagnostic {
                    severity: Severity::Error,
                    file: Some(e.path.display().to_string()),
                    line: Some(e.line),
                    col: Some(e.col),
                    message: e.message.clone(),
                    source_line,
                });
                return Err(Error::other(format!("cannot load the {display} project")));
            }
        };
        let diags = project.check();
        for diag in &diags {
            let (file, line, col) = project.sources.resolve(diag.span.start);
            // The source is already in memory (the SourceFile) — no extra IO.
            let source_line = nth_line(&file.src, line);
            emit(Event::Diagnostic {
                severity: Severity::Error,
                file: Some(file.path.display().to_string()),
                line: Some(line),
                col: Some(col),
                message: diag.message.clone(),
                source_line,
            });
        }
        if diags.is_empty() {
            // The user's own sibling `.mle` files: exclude the entry and the
            // prelude-injected builtin (`<builtin>/Net.mle`).
            let sibling_count = project
                .sources
                .files()
                .iter()
                .filter(|f| !f.path.starts_with("<builtin>"))
                .count()
                .saturating_sub(1);
            emit(Event::MleLoaded {
                entry: self.entry.clone(),
                sibling_count,
            });
            Ok(())
        } else {
            Err(Error::other(format!(
                "{} type error(s) in the {display} project",
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
            return self.run_wasm(working_directory, runner_args, develop).await;
        }
        self.entry_path(working_directory)?; // existence validated up front
        if develop {
            emit(Event::Info {
                message: format!(
                    "develop: hot reload is built in — edit {} and save",
                    self.entry
                ),
            });
        }

        // Post-E3 there is one binary: drive the desktop runtime's run loop
        // IN-PROCESS instead of spawning a separate runner child. GLFW/Cocoa
        // needs the main thread, and `run` blocks on the game loop; the CLI's
        // `#[tokio::main]` drives this future on the main thread (block_on), so
        // the call lands on the main thread and inside a tokio runtime context
        // (net dispatch uses `tokio::spawn`).
        //
        // The former child ran with cwd = the project dir so the relative
        // `--game-path` and asset paths resolve; replicate that by chdir-ing
        // here (this is the terminal action, and `run` never returns for a
        // long-lived game, so the process cwd change is safe).
        std::env::set_current_dir(working_directory)?;

        // Build the runner argv (identical to what was passed to the child) and
        // parse it with the runtime's own clap `Args`, preserving the exact
        // arg-forwarding contract (`--capture-frame`, `--fixed-time`,
        // `--debug-port`, `--headless`, `--hidden`, …). argv[0] is a
        // placeholder program name for clap.
        let mut argv: Vec<String> = vec![
            "functor".to_string(),
            "--mle".to_string(),
            "--game-path".to_string(),
            self.entry.clone(),
        ];
        argv.extend(runner_args.iter().cloned());
        let runtime_args = functor_runtime_desktop::Args::parse_from(argv);

        // Route the in-process runtime's output through the CLI's event stream
        // instead of letting it `println!` raw lines (which would corrupt
        // `--json` ndjson and bypass the renderer). The runtime emits typed
        // `RuntimeEvent`s; we map each onto an `output::Event` and render it.
        // Dependency direction stays clean: the CLI knows the runtime, never
        // the reverse (see docs/cli-output.md).
        functor_runtime_common::events::set_sink(Box::new(|ev| {
            crate::output::emit(ev.into());
        }));
        functor_runtime_desktop::run(runtime_args);
        Ok(())
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
                    emit(Event::Info { message: body });
                    Ok(())
                }
                (status, body) => Err(Error::other(format!("push rejected ({status}): {body}"))),
            };
        }

        emit(Event::Info {
            message: format!(
                "watching {} — pushing to http://{addr}/reload-source on save (Ctrl-C to stop)",
                self.entry
            ),
        });
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
                            emit(Event::Info { message: body });
                            attempted = Some(src);
                        }
                        Ok((status, body)) => {
                            emit(Event::Warning {
                                message: format!("push rejected ({status}): {body}"),
                            });
                            attempted = Some(src);
                        }
                        // Transport failure: leave `attempted` unset so the
                        // same content retries on the next poll.
                        Err(e) => emit(Event::Warning {
                            message: format!("push failed ({e}); retrying…"),
                        }),
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
    }

    /// Serve the project at 127.0.0.1:8080 with the MLE index page (docs/
    /// mle.md C5). The `.mle` entry ships as text — the dev server's
    /// filesystem route serves it from the project dir and the embedded web
    /// runtime fetches + interprets it. Mirrors the F# wasm arm of
    /// `commands::run` (`--no-open` handling included).
    async fn run_wasm(
        &self,
        working_directory: &str,
        runner_args: &[String],
        develop: bool,
    ) -> Result<(), Error> {
        self.entry_path(working_directory)?; // fail before serving, not per fetch

        // The dev server can only serve files INSIDE the project dir — an
        // entry that escapes it (absolute, or `..`) is readable natively but
        // unfetchable by the page. Fail loud here, not as a browser 404.
        if entry_escapes_project(&self.entry) {
            return Err(Error::other(format!(
                "mle on wasm serves the project directory over HTTP, so `entry` must be a \
relative path inside it (got {})",
                self.entry
            )));
        }
        if develop {
            emit(Event::Info {
                message: "develop (wasm): hot reload is native-only — reload the page to pick up edits".to_string(),
            });
        }
        let no_open = runner_args.iter().any(|a| a == "--no-open");
        let ignored: Vec<&str> = runner_args
            .iter()
            .filter(|a| a.as_str() != "--no-open")
            .map(|s| s.as_str())
            .collect();
        if !ignored.is_empty() {
            emit(Event::Warning {
                message: format!(
                    "ignoring runner args (not supported for wasm): {}",
                    ignored.join(" ")
                ),
            });
        }

        let wasm_server_start = WasmDevServer::start_mle(working_directory, &self.entry);
        if no_open {
            emit(Event::Info {
                message: "--no-open: skipping browser launch".to_string(),
            });
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

/// The 1-based `line`th line of `src`, without its newline — `None` when the
/// line is out of range (a defensive fail-soft: the caret is a nicety, never a
/// hard dependency of surfacing the diagnostic).
fn nth_line(src: &str, line: usize) -> Option<String> {
    line.checked_sub(1)
        .and_then(|idx| src.lines().nth(idx))
        .map(str::to_string)
}

/// True when `entry` can't be served by the wasm dev server, which roots at
/// the project directory: absolute paths and any `..` component escape it.
fn entry_escapes_project(entry: &str) -> bool {
    Path::new(entry).is_absolute() || entry.split(['/', '\\']).any(|seg| seg == "..")
}

#[cfg(test)]
mod tests {
    use super::{entry_escapes_project, nth_line};

    #[test]
    fn nth_line_returns_the_1_based_line_without_newline() {
        let src = "one\ntwo\nthree";
        assert_eq!(nth_line(src, 1).as_deref(), Some("one"));
        assert_eq!(nth_line(src, 3).as_deref(), Some("three"));
    }

    #[test]
    fn nth_line_fails_soft_out_of_range() {
        // Line 0 (never valid, 1-based) and past-the-end → None, not a panic.
        assert_eq!(nth_line("only\n", 0), None);
        assert_eq!(nth_line("only\n", 5), None);
        assert_eq!(nth_line("", 1), None);
    }

    #[test]
    fn entries_inside_the_project_are_servable() {
        assert!(!entry_escapes_project("game.mle"));
        assert!(!entry_escapes_project("src/game.mle"));
    }

    #[test]
    fn escaping_entries_are_rejected() {
        assert!(entry_escapes_project("../shared/game.mle"));
        assert!(entry_escapes_project("src/../../game.mle"));
        assert!(entry_escapes_project("/tmp/game.mle"));
    }
}
