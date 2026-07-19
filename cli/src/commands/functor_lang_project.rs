//! CLI support for Functor Lang projects (docs/functor-lang.md Track C4): a
//! `functor.json` with `"language": "functor-lang"` routes `build`/`run`/`develop` to
//! the interpreter instead of the Fable→cargo pipeline.
//!
//! - `build` is the strict gate: parse + lower + typecheck, with `functor-lang check`
//!   diagnostics as **errors** (the runner treats them as warnings so the
//!   dev loop stays permissive; the build command is where they block).
//! - `run` drives the desktop runtime's run loop IN-PROCESS on the entry file
//!   (cwd = the game dir, so asset paths resolve as usual) — post-E3 there is a
//!   single `functor` binary, no separate runner child process.
//! - `develop` is `run`: the Functor Lang producer hot-reloads on save by itself — no
//!   external file watcher, no rebuild. State is preserved across edits.
//! - `run wasm` serves the project with the Functor Lang index page (docs/functor-lang.md C5):
//!   nothing compiles — the `.fun` source ships as text, fetched and
//!   interpreted by the embedded web runtime. Hot reload is native-only;
//!   reload the page to pick up edits.

use std::io::{Error, ErrorKind};
use std::path::{Path, PathBuf};

use clap::Parser;

use crate::output::{emit, Event, Severity};
// `util` (the shell-command runner + wasm dev server) is only used by the
// `web`-gated `run wasm` path.
#[cfg(feature = "web")]
use crate::util::{self, ShellCommand, WasmDevServer};
use crate::Environment;

/// The Functor Lang project settings read from `functor.json`.
#[derive(Debug)]
pub struct FunctorLangProject {
    /// The game source, relative to the project dir (default `game.fun`).
    pub entry: String,
}

/// The entry layout `functor.json` declares: the classic single `entry`, or a
/// named `entries` map for projects whose roles share one directory of modules
/// (e.g. `{"client": "client.fun", "server": "server.fun"}` beside a shared
/// `protocol.fun`). Selection happens in [`FunctorLangConfig::select`] so every
/// command resolves the same way.
enum FunctorLangEntries {
    Single(String),
    Named(Vec<(String, serde_json::Value)>),
    /// Both `entry` and `entries` were declared — ambiguous, refused at selection.
    Conflicting,
    /// `entries` was declared but is not an object — refused at selection.
    Malformed,
}

/// What `detect` reads from `functor.json`, before an entry is selected.
pub struct FunctorLangConfig {
    entries: FunctorLangEntries,
}

/// Read `functor.json` and return the Functor Lang project settings when
/// `"language": "functor-lang"` — `None` (the F#/Fable pipeline) otherwise, including
/// for projects whose `functor.json` is empty or has no `language` field.
pub fn detect(working_directory: &str) -> Option<FunctorLangConfig> {
    let path = Path::new(working_directory).join("functor.json");
    let content = std::fs::read_to_string(path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;
    if json.get("language").and_then(|v| v.as_str()) != Some("functor-lang") {
        return None;
    }
    let entries = match (json.get("entry"), json.get("entries")) {
        (Some(_), Some(_)) => FunctorLangEntries::Conflicting,
        (None, Some(serde_json::Value::Object(map))) => FunctorLangEntries::Named(
            map.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        ),
        // A non-object `entries` is shaped wrong; carry that so selection
        // reports it instead of silently running the default entry.
        (None, Some(_)) => FunctorLangEntries::Malformed,
        (entry, None) => FunctorLangEntries::Single(
            entry
                .and_then(|v| v.as_str())
                .unwrap_or("game.fun")
                .to_string(),
        ),
    };
    Some(FunctorLangConfig { entries })
}

impl FunctorLangConfig {
    /// Resolve which entry this invocation runs. `requested` is the CLI's
    /// `--entry <name>`; a `Named` project with no request defaults to
    /// `client`, or the sole entry.
    pub fn select(&self, requested: Option<&str>) -> Result<FunctorLangProject, Error> {
        match &self.entries {
            FunctorLangEntries::Conflicting => Err(Error::other(
                "functor.json declares both `entry` and `entries` — keep one",
            )),
            FunctorLangEntries::Malformed => Err(Error::other(
                "functor.json `entries` must be a map of name → .fun path \
(e.g. {\"client\": \"client.fun\", \"server\": \"server.fun\"})",
            )),
            FunctorLangEntries::Single(entry) => match requested {
                None => Ok(FunctorLangProject {
                    entry: entry.clone(),
                }),
                Some(name) => Err(Error::other(format!(
                    "--entry {name}: this project has a single `entry` — `--entry` picks from \
an `entries` map in functor.json"
                ))),
            },
            FunctorLangEntries::Named(map) => {
                let names = || {
                    map.iter()
                        .map(|(k, _)| k.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                let pick = |name: &str, value: &serde_json::Value| match value.as_str() {
                    Some(entry) if !entry.is_empty() => Ok(FunctorLangProject {
                        entry: entry.to_string(),
                    }),
                    _ => Err(Error::other(format!(
                        "functor.json entry `{name}` must be a path to a .fun file"
                    ))),
                };
                match requested {
                    Some(name) => match map.iter().find(|(k, _)| k == name) {
                        Some((k, v)) => pick(k, v),
                        None => Err(Error::other(format!(
                            "no entry named `{name}` in functor.json (available: {})",
                            names()
                        ))),
                    },
                    None => match map.as_slice() {
                        [] => Err(Error::other(
                            "functor.json `entries` must be a non-empty map of \
name → .fun path (e.g. {\"client\": \"client.fun\", \"server\": \"server.fun\"})",
                        )),
                        [(k, v)] => pick(k, v),
                        _ => match map.iter().find(|(k, _)| k == "client") {
                            Some((k, v)) => pick(k, v),
                            None => Err(Error::other(format!(
                                "functor.json declares multiple entries ({}) — pick one with \
--entry <name>",
                                names()
                            ))),
                        },
                    },
                }
            }
        }
    }

    /// Every declared entry, resolved — the example-sweep test typechecks each.
    #[cfg(test)]
    fn all(&self) -> Result<Vec<FunctorLangProject>, Error> {
        match &self.entries {
            FunctorLangEntries::Named(map) => map
                .iter()
                .map(|(k, _)| self.select(Some(k)))
                .collect(),
            _ => self.select(None).map(|p| vec![p]),
        }
    }
}

impl FunctorLangProject {
    fn entry_path(&self, working_directory: &str) -> Result<PathBuf, Error> {
        let path = Path::new(working_directory).join(&self.entry);
        if !path.exists() {
            return Err(Error::new(
                ErrorKind::NotFound,
                format!("functor-lang entry not found: {}", path.display()),
            ));
        }
        Ok(path)
    }

    /// Load the project (B8: the entry plus every sibling `.fun` file —
    /// file = module) and typecheck the whole program; `functor-lang check`
    /// diagnostics are build errors here (see the module doc).
    /// `verify_assets` gates the B.3 locator checks: true only for the BUILD
    /// command (the strict/ship gate). `run`/`develop` pass false — a missing
    /// gitignored model must not abort the dev loop (the runtime's fallback +
    /// logged error covers it), and cold-URL HEAD probes must not delay
    /// launch (the fast-inner-loop rule).
    pub fn build(&self, working_directory: &str, verify_assets: bool) -> Result<(), Error> {
        refresh_manifest(working_directory);
        let path = self.entry_path(working_directory)?;
        let display = path.display().to_string();
        // A load failure (parse error, bad module name, cycle) is a positioned
        // diagnostic too — surface its file:line:col structurally, like a check
        // error, rather than flattening it into the final text-only error.
        // Inject the host prelude `.funi` interfaces so the game's `Scene.*`
        // (etc.) externals typecheck against real types instead of `Unknown`
        // (docs/functor-lang-interfaces.md). Check-time only — runtime evaluation is unchanged
        // (the host provides the actual values).
        let project = match functor_lang::project::load_with_prelude(
            &path,
            &std::collections::HashMap::new(),
            &functor_prelude::modules(),
        ) {
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
        if !diags.is_empty() {
            return Err(Error::other(format!(
                "{} type error(s) in the {display} project",
                diags.len()
            )));
        }

        // B.3: the strict gate also PROVES the typed asset surface — every
        // literal Asset.* locator exists (file on disk, or a verifiable
        // URL). Findings carry spans, so they render exactly like type
        // diagnostics. (Bare-string consumer args are check-time type
        // errors since the flag day — no lint needed.)
        if !verify_assets {
            return self.finish_build(&project);
        }
        let findings = crate::util::asset_verify::verify_assets(
            &project.module,
            Path::new(working_directory),
            &mut crate::util::asset_verify::probe_url_live,
        );
        for (finding, severity) in findings
            .errors
            .iter()
            .map(|f| (f, Severity::Error))
            .chain(findings.warnings.iter().map(|f| (f, Severity::Warning)))
        {
            let (file, line, col) = project.sources.resolve(finding.span.start);
            emit(Event::Diagnostic {
                severity,
                file: Some(file.path.display().to_string()),
                line: Some(line),
                col: Some(col),
                message: finding.message.clone(),
                source_line: nth_line(&file.src, line),
            });
        }
        if !findings.errors.is_empty() {
            return Err(Error::other(format!(
                "{} missing asset(s) in the {display} project",
                findings.errors.len()
            )));
        }
        self.finish_build(&project)
    }

    /// The successful-build tail: report what loaded.
    fn finish_build(&self, project: &functor_lang::project::Project) -> Result<(), Error> {
        // The user's own sibling `.fun` files: exclude the entry and the
        // prelude-injected builtin (`<builtin>/Net.fun`).
        let sibling_count = project
            .sources
            .files()
            .iter()
            .filter(|f| !f.path.starts_with("<builtin>"))
            .count()
            .saturating_sub(1);
        emit(Event::FunctorLangLoaded {
            entry: self.entry.clone(),
            sibling_count,
        });
        Ok(())
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
        refresh_manifest(working_directory);
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
            "--functor-lang".to_string(),
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

    /// `build wasm`, after the typecheck gate: write the project as a
    /// self-contained static web bundle in `dist/web/` — the same file set
    /// the wasm dev server serves (see `util::wasm_export`). Zip the folder
    /// for itch.io (HTML5) or serve it from any static host.
    pub fn export_wasm(&self, working_directory: &str) -> Result<(), Error> {
        #[cfg(not(feature = "web"))]
        {
            let _ = working_directory;
            Err(Error::other(
                "the web runtime is not bundled in this build — rebuild with the `web` feature \
                 (`npm run build:cli`) to `build wasm`",
            ))
        }
        #[cfg(feature = "web")]
        {
            self.entry_path(working_directory)?;
            // Same constraint as `run wasm`: the bundle carries the project
            // directory, so the entry must live inside it.
            if entry_escapes_project(&self.entry) {
                return Err(Error::other(format!(
                    "functor-lang on wasm ships the project directory, so `entry` must be a \
relative path inside it (got {})",
                    self.entry
                )));
            }
            let export = util::export_functor_lang_wasm(working_directory, &self.entry)?;
            for name in &export.shadowed {
                emit(Event::Warning {
                    message: format!(
                        "project file `{name}` was not copied — that name is reserved for the \
bundle's runtime files"
                    ),
                });
            }
            for link in &export.skipped_symlinks {
                emit(Event::Warning {
                    message: format!(
                        "symlinked directory `{link}` was not copied into the bundle \
(following it could recurse or pull in files outside the project)"
                    ),
                });
            }
            for asset in &export.missing_assets {
                emit(Event::Warning {
                    message: format!(
                        "asset \"{asset}\" is referenced in the source but won't be in the bundle \
(missing from the project dir, or an absolute/`..` path) — it would load as the empty fallback"
                    ),
                });
            }
            emit(Event::Info {
                message: format!(
                    "exported static web bundle to {} ({} project files, {:.1} MB + {:.1} MB runtime) \
— zip the folder for itch.io (HTML5), or serve it from any static host",
                    export.out_dir.display(),
                    export.file_count,
                    export.project_bytes as f64 / 1e6,
                    export.runtime_bytes as f64 / 1e6,
                ),
            });
            Ok(())
        }
    }

    /// Serve the project at 127.0.0.1:8080 with the Functor Lang index page (docs/
    /// docs/functor-lang.md C5). The `.fun` entry ships as text — the dev server's
    /// filesystem route serves it from the project dir and the embedded web
    /// runtime fetches + interprets it. Mirrors the F# wasm arm of
    /// `commands::run` (`--no-open` handling included).
    async fn run_wasm(
        &self,
        working_directory: &str,
        runner_args: &[String],
        develop: bool,
    ) -> Result<(), Error> {
        #[cfg(not(feature = "web"))]
        {
            let _ = (working_directory, runner_args, develop);
            return Err(Error::other(
                "the web runtime is not bundled in this build — rebuild with the `web` feature \
                 (`npm run build:cli`) to `run wasm`",
            ));
        }
        #[cfg(feature = "web")]
        {
            self.entry_path(working_directory)?; // fail before serving, not per fetch

            // The dev server can only serve files INSIDE the project dir — an
            // entry that escapes it (absolute, or `..`) is readable natively but
            // unfetchable by the page. Fail loud here, not as a browser 404.
            if entry_escapes_project(&self.entry) {
                return Err(Error::other(format!(
                    "functor-lang on wasm serves the project directory over HTTP, so `entry` must be a \
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

            let wasm_server_start = WasmDevServer::start_functor_lang(working_directory, &self.entry);
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
}

/// Auto-reimport (B.2): regenerate a stale GENERATED `assets.fun` before the
/// project loads, so its constants match the on-disk assets (see
/// `commands::import::ensure_fresh` — projects opt in by running
/// `functor import` once; hand-written files are never touched). Never blocks
/// the command — a scan/inspect failure degrades to a warning.
fn refresh_manifest(working_directory: &str) {
    if let Err(e) = crate::commands::import::ensure_fresh(Path::new(working_directory)) {
        emit(Event::Warning {
            message: format!("asset-manifest refresh failed: {e}"),
        });
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
#[cfg(feature = "web")]
fn entry_escapes_project(entry: &str) -> bool {
    Path::new(entry).is_absolute() || entry.split(['/', '\\']).any(|seg| seg == "..")
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "web")]
    use super::entry_escapes_project;
    use super::{nth_line, FunctorLangConfig, FunctorLangEntries};

    fn single(entry: &str) -> FunctorLangConfig {
        FunctorLangConfig {
            entries: FunctorLangEntries::Single(entry.to_string()),
        }
    }

    fn named(pairs: &[(&str, &str)]) -> FunctorLangConfig {
        FunctorLangConfig {
            entries: FunctorLangEntries::Named(
                pairs
                    .iter()
                    .map(|(k, v)| (k.to_string(), serde_json::Value::from(*v)))
                    .collect(),
            ),
        }
    }

    #[test]
    fn single_entry_selects_by_default_and_rejects_the_flag() {
        assert_eq!(single("game.fun").select(None).unwrap().entry, "game.fun");
        let err = single("game.fun").select(Some("server")).unwrap_err();
        assert!(err.to_string().contains("single `entry`"), "{err}");
    }

    #[test]
    fn named_entries_select_by_name() {
        let config = named(&[("client", "client.fun"), ("server", "server.fun")]);
        assert_eq!(config.select(Some("server")).unwrap().entry, "server.fun");
        assert_eq!(config.select(Some("client")).unwrap().entry, "client.fun");
    }

    #[test]
    fn named_entries_default_to_client_or_the_sole_entry() {
        let config = named(&[("server", "server.fun"), ("client", "client.fun")]);
        assert_eq!(config.select(None).unwrap().entry, "client.fun");
        assert_eq!(
            named(&[("server", "server.fun")]).select(None).unwrap().entry,
            "server.fun"
        );
    }

    #[test]
    fn multiple_entries_without_client_need_the_flag() {
        let err = named(&[("alpha", "a.fun"), ("beta", "b.fun")])
            .select(None)
            .unwrap_err();
        assert!(err.to_string().contains("--entry"), "{err}");
        assert!(err.to_string().contains("alpha, beta"), "{err}");
    }

    #[test]
    fn unknown_entry_name_lists_the_available_ones() {
        let err = named(&[("client", "client.fun")])
            .select(Some("sever"))
            .unwrap_err();
        assert!(err.to_string().contains("no entry named `sever`"), "{err}");
        assert!(err.to_string().contains("client"), "{err}");
    }

    #[test]
    fn conflicting_entry_and_entries_are_refused() {
        let config = FunctorLangConfig {
            entries: FunctorLangEntries::Conflicting,
        };
        let err = config.select(None).unwrap_err();
        assert!(err.to_string().contains("both `entry` and `entries`"), "{err}");
    }

    #[test]
    fn empty_or_non_string_entries_are_refused() {
        let err = named(&[]).select(None).unwrap_err();
        assert!(err.to_string().contains("non-empty"), "{err}");
        let config = FunctorLangConfig {
            entries: FunctorLangEntries::Named(vec![(
                "client".to_string(),
                serde_json::Value::from(3),
            )]),
        };
        let err = config.select(None).unwrap_err();
        assert!(err.to_string().contains("must be a path"), "{err}");
    }

    /// Every shipped example typechecks against the engine prelude — the
    /// game-level half of the `.funi` ↔ implementation sync story: the drift
    /// tests (functor_runtime_common) pin interface ≡ registrations, and this
    /// sweep pins that the interface still describes what real games write.
    /// It runs `build`'s exact gate (prelude-injected load + whole-program
    /// check) minus the manifest/emit side effects, so it needs no fetched
    /// assets, GPU, or network. A prelude signature change that breaks any
    /// example now fails `cargo test` instead of waiting for a manual sweep.
    #[test]
    fn every_shipped_example_typechecks() {
        let examples: std::path::PathBuf =
            [env!("CARGO_MANIFEST_DIR"), "..", "examples"].iter().collect();
        let mut dirs: Vec<std::path::PathBuf> = std::fs::read_dir(&examples)
            .expect("examples directory")
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|dir| dir.join("functor.json").is_file())
            .collect();
        dirs.sort();
        assert!(
            dirs.len() >= 20,
            "expected the full example set, found {} project dirs — did the \
examples move?",
            dirs.len()
        );

        let mut failures = Vec::new();
        for dir in &dirs {
            let name = dir.file_name().unwrap_or_default().to_string_lossy().into_owned();
            let dir_str = dir.to_string_lossy().into_owned();
            let Some(config) = super::detect(&dir_str) else {
                failures.push(format!("{name}: functor.json did not parse as a project"));
                continue;
            };
            let projects = match config.all() {
                Ok(projects) => projects,
                Err(e) => {
                    failures.push(format!("{name}: {e}"));
                    continue;
                }
            };
            // A multi-entry project typechecks once PER entry: each entry is
            // its own program root over the same sibling modules.
            for project in &projects {
                let label = format!("{name}[{}]", project.entry);
                let entry = match project.entry_path(&dir_str) {
                    Ok(entry) => entry,
                    Err(e) => {
                        failures.push(format!("{label}: {e}"));
                        continue;
                    }
                };
                match functor_lang::project::load_with_prelude(
                    &entry,
                    &std::collections::HashMap::new(),
                    &functor_prelude::modules(),
                ) {
                    Ok(loaded) => {
                        for diag in loaded.check() {
                            let (file, line, col) = loaded.sources.resolve(diag.span.start);
                            failures.push(format!(
                                "{label}: {}:{line}:{col}: {}",
                                file.path.display(),
                                diag.message
                            ));
                        }
                    }
                    Err(e) => failures.push(format!(
                        "{label}: {}:{}:{}: {}",
                        e.path.display(),
                        e.line,
                        e.col,
                        e.message
                    )),
                }
            }
        }
        assert!(
            failures.is_empty(),
            "shipped examples no longer typecheck against the prelude:\n{}",
            failures.join("\n")
        );
    }

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

    #[cfg(feature = "web")]
    #[test]
    fn entries_inside_the_project_are_servable() {
        assert!(!entry_escapes_project("game.fun"));
        assert!(!entry_escapes_project("src/game.fun"));
    }

    #[cfg(feature = "web")]
    #[test]
    fn escaping_entries_are_rejected() {
        assert!(entry_escapes_project("../shared/game.fun"));
        assert!(entry_escapes_project("src/../../game.fun"));
        assert!(entry_escapes_project("/tmp/game.fun"));
    }
}
