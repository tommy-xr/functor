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
        // Inject the engine's bundled `.fun` modules and `.funi` interfaces
        // so reusable modules such as `Animator` execute and host externals
        // such as `Scene.*` typecheck against their real types.
        let project = match functor_lang::project::load_with_bundled_modules(
            &path,
            &std::collections::HashMap::new(),
            &functor_prelude::bundled_modules(),
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
        if matches!(environment, Environment::Vr) {
            if !runner_args.is_empty() {
                emit(Event::Warning {
                    message: "runner args are ignored on vr (they configure the desktop runtime)"
                        .to_string(),
                });
            }
            return self.run_vr(working_directory).await;
        }
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

    /// `run vr` / `develop vr`: run the game on an adb-attached headset
    /// running the functor VR runtime (a tool APK built once — see
    /// runtime/functor-runtime-oculus/README.md). One command: launch the
    /// app, forward the push port, push the whole project, then keep
    /// watching and re-pushing on save (hot reload is built in, like
    /// native — `run` and `develop` are the same here too), streaming the
    /// headset's runtime log into this terminal.
    async fn run_vr(&self, working_directory: &str) -> Result<(), Error> {
        let entry_path = self.entry_path(working_directory)?;
        let project_root = Path::new(working_directory);
        let serial = adb_device().await?;
        adb_require_runtime(&serial).await?;
        // `am start` on the running singleTask activity is a no-op resume —
        // idempotent, so no need to check whether the app is already up.
        adb_run(&serial, &["shell", "am", "start", "-n", VR_COMPONENT]).await?;
        let forward = format!("tcp:{VR_PORT}");
        adb_run(&serial, &["forward", &forward, &forward]).await?;
        spawn_logcat(&serial);
        let addr = format!("127.0.0.1:{VR_PORT}");

        // Wait for the cold app to bind its endpoint, clearing any previous
        // project's upload manifest in the same round trip. Assets land
        // BEFORE the new game starts, so its first Sub.assets snapshot cannot
        // observe transient "missing on Android" failures.
        let mut ready = false;
        for _ in 0..20 {
            match post_asset_manifest(&addr, &[]) {
                Ok((200, _)) => {
                    ready = true;
                    break;
                }
                Ok((status, body)) => {
                    return Err(Error::other(format!(
                        "asset sync rejected ({status}): {body}"
                    )))
                }
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(500)).await,
            }
        }
        if !ready {
            return Err(Error::other(format!(
                "cannot reach the headset's debug endpoint (http://{addr} via adb forward) — \
is the functor VR runtime running? (`adb logcat -s functor` for its startup log)"
            )));
        }

        let mut attempted_assets = project_asset_files(project_root)?;
        let report = sync_project_assets(&addr, &attempted_assets, None)?;
        let mut observed_assets = attempted_assets.clone();
        emit(Event::Info {
            message: format!(
                "synced {} project asset(s) ({:.1} MB) to the headset",
                report.files,
                report.bytes as f64 / (1024.0 * 1024.0)
            ),
        });

        // Load only after every initial asset is resident. Keep retries for a
        // cable reconnect between the readiness probe and this request.
        let files = read_project_json(&entry_path)?;
        let mut attempted = None;
        for _ in 0..20 {
            match post_load_project(&addr, &files) {
                Ok((200, body)) => {
                    emit(Event::Info { message: body });
                    attempted = Some(files.clone());
                    break;
                }
                Ok((status, body)) => {
                    return Err(Error::other(format!("push rejected ({status}): {body}")))
                }
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(500)).await,
            }
        }
        let mut attempted = attempted.ok_or_else(|| {
            Error::other(format!(
                "cannot reach the headset's reload endpoint (http://{addr} via adb forward) — \
is the functor VR runtime running? (`adb logcat -s functor` for its startup log)"
            ))
        })?;
        emit(Event::Info {
            message: format!(
                "watching {} + siblings + project assets — edit and save to hot-reload on the headset \
(Ctrl-C to stop)",
                self.entry
            ),
        });

        // The watch loop, shaped like `push --watch`: poll contents (not
        // mtimes), track the last ATTEMPTED file set, and re-push the WHOLE
        // set on any change (file = module — a sibling edit must ship too).
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            // Atomic-save editors briefly unlink files mid-save; wait for
            // the next poll rather than failing the loop.
            let Ok(current) = read_project_json(&entry_path) else {
                continue;
            };
            let Ok(current_assets) = project_asset_files(project_root) else {
                continue;
            };
            let assets_changed = current_assets != observed_assets;
            if current == attempted && !assets_changed {
                continue;
            }
            // The same check gate `run native` uses — structured file:line
            // diagnostics beat the device's rendered 400 body, and a broken
            // edit never mutates either source OR assets on the headset.
            if self.build(working_directory, false).is_err() {
                emit(Event::Warning {
                    message: "check failed — the headset keeps the previous program".to_string(),
                });
                attempted = current;
                observed_assets = current_assets;
                continue;
            }
            // An asset add/rename can regenerate assets.fun during `build`.
            // Sync and push the exact post-build snapshots, in that order.
            let current = read_project_json(&entry_path)?;
            let current_assets = project_asset_files(project_root)?;
            if current_assets != attempted_assets {
                match sync_project_assets(&addr, &current_assets, Some(&attempted_assets)) {
                    Ok(report) => {
                        emit(Event::Info {
                            message: format!(
                                "synced {} changed asset(s) ({:.1} MB) to the headset",
                                report.files,
                                report.bytes as f64 / (1024.0 * 1024.0)
                            ),
                        });
                        attempted_assets = current_assets.clone();
                        observed_assets = current_assets;
                    }
                    // Leave the last synchronized inventory in place so the
                    // same bytes retry after a cable reconnect/runtime restart.
                    Err(e) => {
                        emit(Event::Warning {
                            message: format!("asset sync failed ({e}); retrying…"),
                        });
                        continue;
                    }
                }
            } else {
                observed_assets = current_assets;
            }
            if current == attempted {
                continue;
            }
            match post_reload_project(&addr, &current) {
                Ok((200, body)) => {
                    emit(Event::Info { message: body });
                    attempted = current;
                }
                Ok((status, body)) => {
                    emit(Event::Warning {
                        message: format!("push rejected ({status}): {body}"),
                    });
                    attempted = current;
                }
                // Transport failure (cable out, app restarting): leave
                // `attempted` alone so the same content retries next poll.
                Err(e) => emit(Event::Warning {
                    message: format!("push failed ({e}); retrying…"),
                }),
            }
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

// --- `run vr` plumbing -------------------------------------------------------

/// The functor VR runtime tool APK (runtime/functor-runtime-oculus).
const VR_PACKAGE: &str = "dev.functor.runner";
const VR_COMPONENT: &str = "dev.functor.runner/android.app.NativeActivity";
/// Its device-loopback push port (`adb forward` bridges it to this machine).
const VR_PORT: u16 = 8123;

/// Run one adb command to completion; stdout on success, a rendered error
/// (including the "adb isn't installed" case) otherwise.
async fn adb_output(serial: Option<&str>, args: &[&str]) -> Result<String, Error> {
    let mut cmd = tokio::process::Command::new("adb");
    if let Some(serial) = serial {
        cmd.args(["-s", serial]);
    }
    cmd.args(args);
    let out = cmd.output().await.map_err(|e| {
        Error::other(format!(
            "cannot run adb ({e}) — install Android platform-tools and ensure `adb` is on PATH"
        ))
    })?;
    if !out.status.success() {
        return Err(Error::other(format!(
            "adb {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// adb, success/failure only.
async fn adb_run(serial: &str, args: &[&str]) -> Result<(), Error> {
    adb_output(Some(serial), args).await.map(|_| ())
}

/// The attached device's serial: `ANDROID_SERIAL` when set (adb's own
/// convention), else the single `adb devices` entry — zero or several is an
/// error with the fix in the message.
async fn adb_device() -> Result<String, Error> {
    if let Ok(serial) = std::env::var("ANDROID_SERIAL") {
        return Ok(serial);
    }
    let out = adb_output(None, &["devices"]).await?;
    let devices: Vec<String> = out
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            match (fields.next(), fields.next()) {
                (Some(serial), Some("device")) => Some(serial.to_string()),
                _ => None,
            }
        })
        .collect();
    match devices.as_slice() {
        [one] => Ok(one.clone()),
        [] => {
            // The common first-connect states deserve their real diagnosis,
            // not "none": `unauthorized` = the USB-debugging prompt hasn't
            // been accepted; `offline` = a wedged connection.
            let stuck = out.lines().find_map(|line| {
                let mut fields = line.split_whitespace();
                match (fields.next(), fields.next()) {
                    (Some(serial), Some(state @ ("unauthorized" | "offline"))) => {
                        Some(format!("{serial} is {state}"))
                    }
                    _ => None,
                }
            });
            Err(Error::other(match stuck {
                Some(stuck) => format!(
                    "device attached but not ready ({stuck}) — put the headset on and \
accept the USB-debugging prompt (unauthorized), or reconnect the cable (offline)"
                ),
                None => "no device attached (`adb devices` lists none) — connect the \
headset over USB and accept its debugging prompt"
                    .to_string(),
            }))
        }
        _ => Err(Error::other(
            "multiple devices attached — set ANDROID_SERIAL to pick one",
        )),
    }
}

/// The tool APK ships separately from games (games are text, pushed live) —
/// require it up front with the install pointer, instead of a connection
/// error after launch.
async fn adb_require_runtime(serial: &str) -> Result<(), Error> {
    let out = adb_output(Some(serial), &["shell", "pm", "list", "packages", VR_PACKAGE]).await?;
    let installed = out
        .lines()
        .any(|line| line.trim() == format!("package:{VR_PACKAGE}"));
    if installed {
        Ok(())
    } else {
        Err(Error::other(format!(
            "the functor VR runtime isn't installed on {serial} — build + install the tool APK \
(see runtime/functor-runtime-oculus/README.md): npm run build:oculus:apk && \
adb install -r target-android/debug/apk/functor_runtime_oculus.apk"
        )))
    }
}

/// Stream the headset's runtime log (`adb logcat -s functor`) into the CLI's
/// event stream, so on-device `[functor-lang]` errors and `Debug.log` traces read
/// like `run native`'s console. `-T 1` starts at now (no history replay).
fn spawn_logcat(serial: &str) {
    let serial = serial.to_string();
    tokio::spawn(async move {
        use tokio::io::AsyncBufReadExt;
        let mut cmd = tokio::process::Command::new("adb");
        cmd.args(["-s", &serial, "logcat", "-T", "1", "-v", "brief", "-s", "functor"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        let Ok(mut child) = cmd.spawn() else {
            emit(Event::Warning {
                message: "cannot stream the headset log (adb logcat failed to start)".to_string(),
            });
            return;
        };
        let Some(stdout) = child.stdout.take() else {
            return;
        };
        let mut lines = tokio::io::BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            // brief format: `I/functor (12345): <message>` — keep the
            // message, drop the logcat framing (separator lines have no
            // "): " and are skipped).
            if let Some((_, message)) = line.split_once("): ") {
                emit(Event::Info {
                    message: format!("headset: {message}"),
                });
            }
        }
    });
}

/// One synchronizable project asset and the cheap metadata fingerprint used by
/// the 300ms watch loop. Bytes are read only for the initial push or when this
/// fingerprint changes, so large models are not re-read continuously.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ProjectAssetFile {
    locator: String,
    disk_path: PathBuf,
    len: u64,
    modified_ns: u128,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct AssetSyncReport {
    files: usize,
    bytes: u64,
}

/// Recursively collect self-contained GLB models, textures, and audio files.
/// Hidden directories and the generated `dist/` tree never ship.
fn project_asset_files(root: &Path) -> Result<Vec<ProjectAssetFile>, Error> {
    fn visit(
        root: &Path,
        directory: &Path,
        is_root: bool,
        out: &mut Vec<ProjectAssetFile>,
    ) -> Result<(), Error> {
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name
                .to_str()
                .ok_or_else(|| Error::other(format!("non-UTF8 file name: {:?}", name)))?;
            if name.starts_with('.') || (is_root && name == "dist") {
                continue;
            }
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                visit(root, &path, false, out)?;
                continue;
            }
            // Follow symlinked files (matching `build wasm`), but never recurse
            // through symlinked directories.
            let metadata = if file_type.is_symlink() {
                match std::fs::metadata(&path) {
                    Ok(metadata) if metadata.is_file() => metadata,
                    _ => continue,
                }
            } else if file_type.is_file() {
                entry.metadata()?
            } else {
                continue;
            };
            if !functor_runtime_common::asset::is_live_project_asset_file(&path) {
                continue;
            }
            let relative = path
                .strip_prefix(root)
                .map_err(|_| Error::other(format!("asset escaped project: {}", path.display())))?;
            let locator = relative
                .components()
                .map(|component| {
                    component.as_os_str().to_str().ok_or_else(|| {
                        Error::other(format!("non-UTF8 asset path: {}", relative.display()))
                    })
                })
                .collect::<Result<Vec<_>, _>>()?
                .join("/");
            functor_runtime_common::debug_protocol::validate_project_asset_path(&locator)
                .map_err(Error::other)?;
            let modified_ns = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |duration| duration.as_nanos());
            out.push(ProjectAssetFile {
                locator,
                disk_path: path,
                len: metadata.len(),
                modified_ns,
            });
        }
        Ok(())
    }

    let mut files = Vec::new();
    visit(root, root, true, &mut files)?;
    files.sort_by(|a, b| a.locator.cmp(&b.locator));
    Ok(files)
}

/// Push added/changed files individually, then finalize with the complete path
/// manifest so assets deleted on the host disappear from the runtime cache.
fn sync_project_assets(
    addr: &str,
    current: &[ProjectAssetFile],
    previous: Option<&[ProjectAssetFile]>,
) -> Result<AssetSyncReport, Error> {
    let mut report = AssetSyncReport::default();
    for asset in current {
        let unchanged = previous.is_some_and(|files| {
            files.iter().any(|old| {
                old.locator == asset.locator
                    && old.len == asset.len
                    && old.modified_ns == asset.modified_ns
            })
        });
        if unchanged {
            continue;
        }
        let bytes = std::fs::read(&asset.disk_path)?;
        let body =
            functor_runtime_common::debug_protocol::encode_project_asset(&asset.locator, &bytes)
                .map_err(Error::other)?;
        let (status, response) = http_post_bytes(
            addr,
            "/reload-asset",
            "application/octet-stream",
            &body,
            std::time::Duration::from_secs(30),
        )?;
        if status != 200 {
            return Err(Error::other(format!(
                "asset {} rejected ({status}): {response}",
                asset.locator
            )));
        }
        report.files += 1;
        report.bytes += bytes.len() as u64;
    }

    let paths: Vec<&str> = current.iter().map(|asset| asset.locator.as_str()).collect();
    let (status, response) = post_asset_manifest(addr, &paths)?;
    if status != 200 {
        return Err(Error::other(format!(
            "asset manifest rejected ({status}): {response}"
        )));
    }
    Ok(report)
}

fn post_asset_manifest(addr: &str, paths: &[&str]) -> Result<(u16, String), Error> {
    let manifest = serde_json::to_vec(paths).map_err(Error::other)?;
    http_post_bytes(
        addr,
        "/sync-assets",
        "application/json",
        &manifest,
        std::time::Duration::from_secs(5),
    )
}

/// The project's `.fun`/`.funi` files as the `/reload-project` wire body — a
/// JSON array of `[file name, source]` pairs, entry FIRST (`file = module`,
/// so names are enough). Serialized once: the watch loop's change-compare
/// and the POST body are the same string.
fn read_project_json(entry_path: &Path) -> Result<String, Error> {
    let mut files = Vec::new();
    for path in functor_lang::project::project_files(entry_path)? {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| Error::other(format!("non-UTF8 file name: {}", path.display())))?
            .to_string();
        files.push((name, std::fs::read_to_string(&path)?));
    }
    serde_json::to_string(&files).map_err(Error::other)
}

/// POST the whole project file set (the `read_project_json` body) to the
/// runtime's `/reload-project`.
fn post_reload_project(addr: &str, files_json: &str) -> Result<(u16, String), Error> {
    http_post(addr, "/reload-project", "application/json", files_json)
}

/// Load the first pushed project as a new game, taking its model from `init`.
/// Later watch-loop edits use `/reload-project` and preserve that model.
fn post_load_project(addr: &str, files_json: &str) -> Result<(u16, String), Error> {
    http_post(addr, "/load-project", "application/json", files_json)
}

/// Minimal HTTP POST over std::net — one dependency-free request to the
/// runner's shared debug HTTP server. Returns (status, body). `Connection: close`
/// keeps the read side trivial (read to EOF, split headers off).
fn post_reload_source(addr: &str, source: &str) -> Result<(u16, String), Error> {
    http_post(addr, "/reload-source", "text/plain", source)
}

fn http_post(
    addr: &str,
    path: &str,
    content_type: &str,
    body: &str,
) -> Result<(u16, String), Error> {
    http_post_bytes(
        addr,
        path,
        content_type,
        body.as_bytes(),
        std::time::Duration::from_secs(5),
    )
}

fn http_post_bytes(
    addr: &str,
    path: &str,
    content_type: &str,
    body: &[u8],
    timeout: std::time::Duration,
) -> Result<(u16, String), Error> {
    use std::io::{Read, Write};
    use std::net::ToSocketAddrs;
    // connect_timeout, not connect: a blackholed host must fail on our
    // request budget, not the OS's ~75s TCP give-up.
    let sockaddr = addr
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| Error::other(format!("cannot resolve {addr}")))?;
    let mut stream = std::net::TcpStream::connect_timeout(&sockaddr, timeout)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    write!(
        stream,
        "POST {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: {content_type}\r\n\
Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response)?;
    let response = String::from_utf8_lossy(&response);
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
    use super::{nth_line, project_asset_files, FunctorLangConfig, FunctorLangEntries};

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

    #[test]
    fn vr_asset_scan_is_recursive_and_skips_hidden_and_dist_files() {
        let root = std::env::temp_dir().join(format!("functor-vr-assets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for directory in ["models", "textures/walls", ".cache", "dist/web"] {
            std::fs::create_dir_all(root.join(directory)).unwrap();
        }
        for (path, bytes) in [
            ("models/ship.glb", b"model".as_slice()),
            ("models/ship.bin", b"external buffer"),
            ("models/ship.gltf", b"external model"),
            ("textures/walls/grid.PNG", b"texture"),
            ("theme.ogg", b"audio"),
            ("notes.txt", b"not an asset"),
            (".cache/hidden.png", b"hidden"),
            ("dist/web/stale.glb", b"generated"),
        ] {
            std::fs::write(root.join(path), bytes).unwrap();
        }

        let files = project_asset_files(&root).unwrap();
        let locators: Vec<&str> = files.iter().map(|asset| asset.locator.as_str()).collect();
        assert_eq!(
            locators,
            vec!["models/ship.glb", "textures/walls/grid.PNG", "theme.ogg",]
        );
        let _ = std::fs::remove_dir_all(root);
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
                match functor_lang::project::load_with_bundled_modules(
                    &entry,
                    &std::collections::HashMap::new(),
                    &functor_prelude::bundled_modules(),
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
