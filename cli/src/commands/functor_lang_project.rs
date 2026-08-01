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
use functor_runtime_common::debug_protocol::DEFAULT_DEVELOP_PORT;

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
    /// The role's entry-point binding prefix (same-file entries): every
    /// canonical entry binding resolves through it as camelCase (`"server"`
    /// → `serverInit`/`serverTick`/…). Empty = the classic unprefixed
    /// contract. Declared per role as `{ "file": "game.fun", "prefix":
    /// "server" }` so two roles can share one file.
    pub prefix: String,
    /// Whether physical relative mouse input is captured and routed to the game.
    pub mouse_capture: bool,
    /// Whether the shell exposes an absolute pointer to the game.
    pub cursor: CursorPolicy,
}

/// Shell pointer behavior declared by `functor.json`'s `cursor` field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorPolicy {
    /// Keep the system cursor in its ordinary shell/UI mode.
    #[default]
    Captured,
    /// Keep the system cursor visible and deliver absolute pointer input.
    Visible,
}

impl CursorPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Captured => "captured",
            Self::Visible => "visible",
        }
    }
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
    mouse_capture: Result<Option<bool>, String>,
    cursor: Option<serde_json::Value>,
}

fn manifest_mouse_capture(json: &serde_json::Value) -> Result<Option<bool>, String> {
    if json.get("viewer").is_some() {
        return Err(
            "functor.json `viewer.camera.control` was removed; remove `viewer` \
for the default captured game input, or use top-level `\"mouseCapture\": false` \
to keep the pointer free"
                .to_string(),
        );
    }
    match json.get("mouseCapture") {
        None => Ok(None),
        Some(serde_json::Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err("functor.json `mouseCapture` must be true or false".to_string()),
    }
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
        (None, Some(serde_json::Value::Object(map))) => {
            FunctorLangEntries::Named(map.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        }
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
    Some(FunctorLangConfig {
        entries,
        mouse_capture: manifest_mouse_capture(&json),
        cursor: json.get("cursor").cloned(),
    })
}

impl FunctorLangConfig {
    fn cursor_policy(&self) -> Result<CursorPolicy, Error> {
        match self.cursor.as_ref() {
            None => Ok(CursorPolicy::Captured),
            Some(value) if value.as_str() == Some("visible") => Ok(CursorPolicy::Visible),
            Some(value) if value.as_str() == Some("captured") => Err(Error::other(
                "functor.json `cursor: \"captured\"` was removed; remove `cursor` \
because capture is now the default",
            )),
            Some(_) => Err(Error::other(
                "functor.json `cursor` only supports \"visible\"",
            )),
        }
    }

    /// Resolve which entry this invocation runs. `requested` is the CLI's
    /// `--entry <name>`; a `Named` project with no request defaults to
    /// `client`, or the sole entry.
    pub fn select(&self, requested: Option<&str>) -> Result<FunctorLangProject, Error> {
        let requested_mouse_capture = *self
            .mouse_capture
            .as_ref()
            .map_err(|message| Error::other(message.clone()))?;
        let cursor = self.cursor_policy()?;
        if requested_mouse_capture == Some(true) && cursor == CursorPolicy::Visible {
            return Err(Error::other(
                "functor.json cannot combine `\"mouseCapture\": true` with \
`\"cursor\": \"visible\"` — choose captured or absolute mouse input",
            ));
        }
        // Games capture relative mouse input by default. An absolute-pointer
        // project opts into `cursor: "visible"`, which naturally disables
        // capture unless the manifest explicitly asks for the contradictory
        // combination above.
        let mouse_capture = requested_mouse_capture.unwrap_or(cursor != CursorPolicy::Visible);
        let project = |entry: String, prefix: String| FunctorLangProject {
            entry,
            prefix,
            mouse_capture,
            cursor,
        };
        match &self.entries {
            FunctorLangEntries::Conflicting => Err(Error::other(
                "functor.json declares both `entry` and `entries` — keep one",
            )),
            FunctorLangEntries::Malformed => Err(Error::other(
                "functor.json `entries` must be a map of name → .fun path (or \
{ \"file\": …, \"prefix\": … }) — e.g. {\"client\": \"client.fun\", \"server\": \"server.fun\"}",
            )),
            FunctorLangEntries::Single(entry) => match requested {
                None => Ok(project(entry.clone(), String::new())),
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
                let pick = |name: &str, value: &serde_json::Value| {
                    pick_entry(name, value).map(|(entry, prefix)| project(entry, prefix))
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

    /// Every declared entry, resolved — `build` validates each role's
    /// contract, and the example-sweep test typechecks each.
    pub fn all(&self) -> Result<Vec<FunctorLangProject>, Error> {
        match &self.entries {
            FunctorLangEntries::Named(map) => {
                map.iter().map(|(k, _)| self.select(Some(k))).collect()
            }
            _ => self.select(None).map(|p| vec![p]),
        }
    }
}

/// Resolve one `entries` value: the classic string form (`"client.fun"`) or
/// the object form (`{ "file": "game.fun", "prefix": "server" }` — same-file
/// entries, where the role's entry bindings resolve through the prefix as
/// camelCase: `serverInit`/`serverTick`/…). Malformed shapes get teaching
/// errors naming the exact fix. Returns the `(entry, prefix)` pair; the
/// caller folds in the project-wide settings.
fn pick_entry(name: &str, value: &serde_json::Value) -> Result<(String, String), Error> {
    match value {
        serde_json::Value::String(entry) if !entry.is_empty() => {
            Ok((entry.clone(), String::new()))
        }
        serde_json::Value::Object(map) => {
            if let Some(unknown) = map.keys().find(|k| k.as_str() != "file" && k.as_str() != "prefix")
            {
                return Err(Error::other(format!(
                    "functor.json entry `{name}`: unknown key \"{unknown}\" — the object form \
takes \"file\" and an optional \"prefix\""
                )));
            }
            let entry = match map.get("file") {
                Some(serde_json::Value::String(file)) if !file.is_empty() => file.clone(),
                _ => {
                    return Err(Error::other(format!(
                        "functor.json entry `{name}`: the object form needs a \"file\" — \
{{ \"file\": \"game.fun\", \"prefix\": \"{name}\" }}"
                    )))
                }
            };
            let prefix = match map.get("prefix") {
                None | Some(serde_json::Value::Null) => String::new(),
                Some(serde_json::Value::String(prefix)) => prefix.clone(),
                Some(_) => {
                    return Err(Error::other(format!(
                        "functor.json entry `{name}`: \"prefix\" must be a string — the \
camelCase binding prefix (e.g. \"server\" resolves serverInit/serverTick/…)"
                    )))
                }
            };
            // A prefix concatenates into binding NAMES, so it must itself be a
            // valid identifier — refuse `"my server"` here, not as a baffling
            // "has no top-level `let my serverInit`" later.
            let mut chars = prefix.chars();
            let valid = match chars.next() {
                None => true,
                Some(first) => {
                    (first.is_ascii_alphabetic() || first == '_')
                        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
                }
            };
            if !valid {
                return Err(Error::other(format!(
                    "functor.json entry `{name}`: \"prefix\" must be a valid identifier \
(it becomes the binding prefix: `{prefix}Init`, `{prefix}Tick`, …)"
                )));
            }
            Ok((entry, prefix))
        }
        _ => Err(Error::other(format!(
            "functor.json entry `{name}` must be a path to a .fun file, or \
{{ \"file\": \"game.fun\", \"prefix\": \"{name}\" }} for a same-file role"
        ))),
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
    pub fn build(
        &self,
        working_directory: &str,
        verify_assets: bool,
    ) -> Result<functor_lang::project::Project, Error> {
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
            return self.finish_build(project);
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
        self.finish_build(project)
    }

    /// The successful-build tail: report what loaded.
    fn finish_build(
        &self,
        project: functor_lang::project::Project,
    ) -> Result<functor_lang::project::Project, Error> {
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
        Ok(project)
    }

    /// `build`'s per-role contract gate (same-file entries): load a Session
    /// under the ENGINE prelude from the project [`Self::build`] already
    /// typechecked, and validate THIS role's entry-point contract — its
    /// (possibly prefixed) names at their required arities. A project
    /// declaring a server role whose `serverTick` is missing or misarityed
    /// fails here with an error naming `serverTick`, instead of at launch.
    /// The validation body is the exact one the runtimes use at load
    /// (`functor_runtime_common::functor_lang_producer::validate_contract`).
    pub fn check_contract(
        &self,
        project: &functor_lang::project::Project,
    ) -> Result<(), Error> {
        use functor_runtime_common::functor_lang_prelude::FunctorHost;
        use functor_runtime_common::functor_lang_producer::{validate_contract, EntryNames};
        let session = functor_lang::Session::load(&project.module, &mut FunctorHost)
            .map_err(|f| {
                Error::other(format!(
                    "cannot load the {} project: {}",
                    self.entry,
                    project.sources.render(f.error.span.start, &f.error.message)
                ))
            })?;
        let names = EntryNames::with_prefix(&self.prefix);
        validate_contract(&self.entry, &session, &names)
            .map(|_| ())
            .map_err(Error::other)
    }

    /// Run the project's inline `expect` tests headlessly under the ENGINE
    /// prelude (no GL context, no window, no game loop) — the thin CLI shell
    /// over [`functor_runtime_common::functor_lang_test::run_expects_in`].
    ///
    /// Takes the project [`Self::build`] already loaded and typechecked, so
    /// the bytes evaluated are exactly the bytes verified (re-loading would
    /// let an editor save land in between). A failure here is therefore a
    /// *runtime* one, rendered as a positioned diagnostic at the `expect`
    /// that produced it.
    pub fn test(&self, project: &functor_lang::project::Project) -> Result<(), Error> {
        let run = match functor_runtime_common::functor_lang_test::run_expects_in(project) {
            Ok(run) => run,
            Err(e) => {
                emit(Event::Diagnostic {
                    severity: Severity::Error,
                    file: Some(e.file.display().to_string()),
                    line: Some(e.line),
                    col: Some(e.col),
                    message: e.message,
                    source_line: std::fs::read_to_string(&e.file)
                        .ok()
                        .and_then(|src| nth_line(&src, e.line)),
                });
                return Err(Error::other(format!("cannot run the {} tests", self.entry)));
            }
        };

        for case in &run.cases {
            let Some(message) = &case.failure else {
                continue;
            };
            emit(Event::Diagnostic {
                severity: Severity::Error,
                file: Some(case.file.display().to_string()),
                line: Some(case.line),
                col: Some(case.col),
                message: message.clone(),
                source_line: case.source_line.clone(),
            });
        }

        if run.total() == 0 {
            emit(Event::Info {
                message: "no `expect` tests found".to_string(),
            });
            return Ok(());
        }
        let (passed, failed) = (run.passed(), run.failed());
        if failed > 0 {
            return Err(Error::other(format!(
                "{} expect(s): {passed} passed, {failed} failed",
                run.total()
            )));
        }
        emit(Event::Info {
            message: format!("{} expect(s): {passed} passed", run.total()),
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
        // A prefixed role can't run on VR yet: the device push path boots the
        // APK's embedded producer with the unprefixed contract, so running it
        // would silently play the wrong role. (Native passes --entry-prefix;
        // wasm bakes the prefix into the served page's boot config.)
        if !self.prefix.is_empty() && matches!(environment, Environment::Vr) {
            return Err(Error::other(format!(
                "entry prefix `{}` is not supported on vr yet — run this role with \
`run native` or `run wasm` (the vr shell loads the unprefixed contract)",
                self.prefix
            )));
        }
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
            "--cursor".to_string(),
            self.cursor.as_str().to_string(),
        ];
        if !self.prefix.is_empty() {
            // The role's entry-point prefix (same-file entries): the runtime
            // resolves `serverInit`/`serverTick`/… through it.
            argv.push("--entry-prefix".to_string());
            argv.push(self.prefix.clone());
        }
        if self.mouse_capture {
            argv.push("--mouse-capture".to_string());
        }
        let (runner_args, debug_warning) = resolve_debug_args(develop, runner_args);
        if let Some(message) = debug_warning {
            emit(Event::Warning { message });
        }
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
            let export = util::export_functor_lang_wasm(
                working_directory,
                &self.entry,
                self.mouse_capture,
                self.cursor.as_str(),
                &self.prefix,
            )?;
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

            let wasm_server_start = WasmDevServer::start_functor_lang(
                working_directory,
                &self.entry,
                self.mouse_capture,
                self.cursor.as_str(),
                &self.prefix,
            );
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

/// Resolve the debug-server args the desktop runtime is invoked with, and
/// strip the CLI-only `--no-debug` (the runtime's clap would reject it).
///
/// `develop` serves the debug runtime on the well-known localhost port
/// [`DEFAULT_DEVELOP_PORT`] by default, so an agent can attach to a human's
/// live session without being told a port. `run` stays opt-in — only a session
/// the developer explicitly called `develop` gets a listener for free. An
/// explicit `--debug-port` (or `--debug-port=P`) always wins, `--no-debug`
/// suppresses the default, and the added default carries
/// `--debug-port-optional` so a second concurrent session degrades to "no
/// debug server" instead of dying on the bind.
///
/// The default is LOCALHOST-ONLY by construction: `--debug-bind` (the flag that
/// widens the server to the LAN, where it is an unauthenticated remote-code
/// channel) suppresses it too, so a wide bind still takes an explicit
/// `--debug-port`. Nothing here ever widens a bind implicitly.
///
/// Returns the args to forward plus an optional warning to emit.
fn resolve_debug_args(develop: bool, runner_args: &[String]) -> (Vec<String>, Option<String>) {
    let has = |name: &str| {
        let prefix = format!("{name}=");
        runner_args
            .iter()
            .any(|arg| arg == name || arg.starts_with(&prefix))
    };
    let explicit = has("--debug-port");
    let bind = has("--debug-bind");
    let no_debug = runner_args.iter().any(|arg| arg == "--no-debug");
    // `--debug-port-optional` is internal to the injected develop default: an
    // EXPLICIT --debug-port that cannot bind must stay an error, so a
    // user-supplied copy is stripped rather than forwarded.
    let internal = runner_args.iter().any(|arg| arg == "--debug-port-optional");
    let mut args: Vec<String> = runner_args
        .iter()
        .filter(|arg| *arg != "--no-debug" && *arg != "--debug-port-optional")
        .cloned()
        .collect();
    let internal_warning = internal.then(|| {
        "--debug-port-optional is internal to the develop default and was ignored \
(an explicit --debug-port that cannot bind is an error)"
            .to_string()
    });

    if !develop || explicit || no_debug || bind {
        let warning = match (no_debug, explicit, bind) {
            (true, true, _) => {
                Some("--no-debug ignored: --debug-port was given explicitly".to_string())
            }
            (false, false, true) if develop => Some(
                "--debug-bind without --debug-port: no debug server started (a non-localhost \
bind is never implicit — pass --debug-port <PORT> to start one)"
                    .to_string(),
            ),
            _ => None,
        };
        return (args, warning.or(internal_warning));
    }

    args.push("--debug-port".to_string());
    args.push(DEFAULT_DEVELOP_PORT.to_string());
    args.push("--debug-port-optional".to_string());
    (args, internal_warning)
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
    let out = adb_output(
        Some(serial),
        &["shell", "pm", "list", "packages", VR_PACKAGE],
    )
    .await?;
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
        cmd.args([
            "-s", &serial, "logcat", "-T", "1", "-v", "brief", "-s", "functor",
        ])
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
    use super::{
        manifest_mouse_capture, nth_line, project_asset_files, resolve_debug_args, CursorPolicy,
        FunctorLangConfig, FunctorLangEntries,
    };

    fn resolve(develop: bool, args: &[&str]) -> (Vec<String>, Option<String>) {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        resolve_debug_args(develop, &args)
    }

    #[test]
    fn develop_defaults_to_the_well_known_debug_port() {
        let (args, warning) = resolve(true, &["--hidden"]);
        assert_eq!(
            args,
            ["--hidden", "--debug-port", "8077", "--debug-port-optional"]
        );
        assert_eq!(warning, None);
    }

    #[test]
    fn run_stays_opt_in() {
        assert_eq!(resolve(false, &["--hidden"]).0, ["--hidden"]);
    }

    #[test]
    fn an_explicit_debug_port_wins() {
        assert_eq!(
            resolve(true, &["--debug-port", "9001"]).0,
            ["--debug-port", "9001"]
        );
        assert_eq!(
            resolve(true, &["--debug-port=9001"]).0,
            ["--debug-port=9001"]
        );
    }

    #[test]
    fn no_debug_suppresses_the_default_and_never_reaches_the_runtime() {
        assert_eq!(resolve(true, &["--no-debug", "--hidden"]).0, ["--hidden"]);
        assert_eq!(resolve(false, &["--no-debug"]).0, Vec::<String>::new());
    }

    #[test]
    fn a_user_supplied_debug_port_optional_is_stripped() {
        // The flag is internal to the injected default: forwarded alongside an
        // explicit --debug-port it would silently degrade the fatal-bind
        // contract, so it never survives the CLI in any combination.
        let (args, warning) = resolve(true, &["--debug-port", "9001", "--debug-port-optional"]);
        assert_eq!(args, ["--debug-port", "9001"]);
        assert!(warning.is_some_and(|w| w.contains("--debug-port-optional")));
        let (args, warning) = resolve(true, &["--debug-port-optional"]);
        assert_eq!(
            args,
            ["--debug-port", "8077", "--debug-port-optional"],
            "the DEFAULT still carries the internal flag it injects itself"
        );
        assert!(warning.is_some_and(|w| w.contains("--debug-port-optional")));
    }

    #[test]
    fn a_bind_is_never_widened_implicitly() {
        // The default listener has no auth, so `--debug-bind` (which exists to
        // expose it) must not be handed a port it never asked for.
        for args in [
            vec!["--debug-bind", "0.0.0.0"],
            vec!["--debug-bind=0.0.0.0"],
        ] {
            let (resolved, warning) = resolve(true, &args);
            assert_eq!(resolved, args);
            assert!(warning.is_some_and(|w| w.contains("--debug-bind")));
        }
        // …but an explicit port with a wide bind is still exactly what was asked.
        assert_eq!(
            resolve(true, &["--debug-bind", "0.0.0.0", "--debug-port", "9001"]).0,
            ["--debug-bind", "0.0.0.0", "--debug-port", "9001"]
        );
    }

    #[test]
    fn no_debug_with_an_explicit_port_warns() {
        let (args, warning) = resolve(true, &["--no-debug", "--debug-port", "9001"]);
        assert_eq!(args, ["--debug-port", "9001"]);
        assert!(warning.is_some_and(|w| w.contains("--no-debug ignored")));
    }

    fn single(entry: &str) -> FunctorLangConfig {
        FunctorLangConfig {
            entries: FunctorLangEntries::Single(entry.to_string()),
            mouse_capture: Ok(None),
            cursor: None,
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
            mouse_capture: Ok(None),
            cursor: None,
        }
    }

    #[test]
    fn mouse_capture_is_a_top_level_boolean_that_defaults_on_for_games() {
        let defaults = manifest_mouse_capture(&serde_json::json!({
            "language": "functor-lang",
            "entry": "game.fun"
        }))
        .unwrap();
        assert_eq!(defaults, None);

        let enabled = manifest_mouse_capture(&serde_json::json!({
            "mouseCapture": true
        }))
        .unwrap();
        assert_eq!(enabled, Some(true));

        let disabled = manifest_mouse_capture(&serde_json::json!({
            "mouseCapture": false
        }))
        .unwrap();
        assert_eq!(disabled, Some(false));

        assert!(manifest_mouse_capture(&serde_json::json!({
            "mouseCapture": "yes"
        }))
        .unwrap_err()
        .contains("true or false"));
    }

    #[test]
    fn removed_viewer_camera_settings_point_to_mouse_capture() {
        for json in [
            serde_json::json!({ "viewer": { "camera": { "control": "game" } } }),
            serde_json::json!({ "viewer": { "camera": { "control": "orbit" } } }),
            serde_json::json!({ "viewer": { "camera": { "detached": "fps" } } }),
            serde_json::json!({ "viewer": { "debugCamera": "fps" } }),
            serde_json::json!({ "viewer": { "camera": { "controls": "game" } } }),
            serde_json::json!({ "viewer": { "camrea": { "control": "game" } } }),
            serde_json::json!({ "viewer": { "control": "game" } }),
            serde_json::json!({ "viewer": { "camera": "game" } }),
            serde_json::json!({ "viewer": "game" }),
        ] {
            let error = manifest_mouse_capture(&json).unwrap_err();
            assert!(error.contains("was removed"), "{error}");
            assert!(error.contains("mouseCapture"), "{error}");
        }
    }

    #[test]
    fn single_entry_selects_by_default_and_rejects_the_flag() {
        let project = single("game.fun").select(None).unwrap();
        assert_eq!(project.entry, "game.fun");
        assert!(project.mouse_capture);
        assert_eq!(project.cursor, CursorPolicy::Captured);
        let err = single("game.fun").select(Some("server")).unwrap_err();
        assert!(err.to_string().contains("single `entry`"), "{err}");
    }

    #[test]
    fn cursor_policy_is_explicit_and_validated() {
        let visible = FunctorLangConfig {
            entries: FunctorLangEntries::Single("game.fun".to_string()),
            mouse_capture: Ok(None),
            cursor: Some(serde_json::Value::from("visible")),
        };
        let visible = visible.select(None).unwrap();
        assert_eq!(visible.cursor, CursorPolicy::Visible);
        assert!(!visible.mouse_capture);

        let invalid = FunctorLangConfig {
            entries: FunctorLangEntries::Single("game.fun".to_string()),
            mouse_capture: Ok(None),
            cursor: Some(serde_json::Value::from("free")),
        };
        let err = invalid.select(None).unwrap_err();
        assert!(err.to_string().contains("visible"), "{err}");

        let non_string = FunctorLangConfig {
            entries: FunctorLangEntries::Single("game.fun".to_string()),
            mouse_capture: Ok(None),
            cursor: Some(serde_json::Value::Bool(true)),
        };
        assert!(non_string
            .select(None)
            .unwrap_err()
            .to_string()
            .contains("visible"));

        let captured = FunctorLangConfig {
            entries: FunctorLangEntries::Single("game.fun".to_string()),
            mouse_capture: Ok(None),
            cursor: Some(serde_json::Value::from("captured")),
        };
        let err = captured.select(None).unwrap_err();
        assert!(err.to_string().contains("was removed"), "{err}");
        assert!(err.to_string().contains("capture is now the default"), "{err}");
    }

    #[test]
    fn named_entries_select_by_name() {
        let config = named(&[("client", "client.fun"), ("server", "server.fun")]);
        assert_eq!(config.select(Some("server")).unwrap().entry, "server.fun");
        assert_eq!(config.select(Some("client")).unwrap().entry, "client.fun");
    }

    #[test]
    fn visible_pointer_and_explicit_mouse_capture_are_mutually_exclusive() {
        let config = FunctorLangConfig {
            entries: FunctorLangEntries::Single("game.fun".to_string()),
            mouse_capture: Ok(Some(true)),
            cursor: Some(serde_json::Value::from("visible")),
        };
        let err = config.select(None).unwrap_err();
        assert!(err.to_string().contains("cannot combine"), "{err}");
    }

    #[test]
    fn mouse_capture_false_is_the_free_pointer_exception() {
        let config = FunctorLangConfig {
            entries: FunctorLangEntries::Single("game.fun".to_string()),
            mouse_capture: Ok(Some(false)),
            cursor: None,
        };
        let project = config.select(None).unwrap();
        assert!(!project.mouse_capture);
        assert_eq!(project.cursor, CursorPolicy::Captured);
    }

    #[test]
    fn named_entries_default_to_client_or_the_sole_entry() {
        let config = named(&[("server", "server.fun"), ("client", "client.fun")]);
        assert_eq!(config.select(None).unwrap().entry, "client.fun");
        assert_eq!(
            named(&[("server", "server.fun")])
                .select(None)
                .unwrap()
                .entry,
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

    fn named_json(pairs: &[(&str, serde_json::Value)]) -> FunctorLangConfig {
        FunctorLangConfig {
            entries: FunctorLangEntries::Named(
                pairs
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.clone()))
                    .collect(),
            ),
            mouse_capture: Ok(None),
            cursor: None,
        }
    }

    #[test]
    fn an_object_entry_resolves_file_and_prefix() {
        let config = named_json(&[
            ("client", serde_json::json!("game.fun")),
            (
                "server",
                serde_json::json!({ "file": "game.fun", "prefix": "server" }),
            ),
        ]);
        let client = config.select(Some("client")).unwrap();
        assert_eq!((client.entry.as_str(), client.prefix.as_str()), ("game.fun", ""));
        let server = config.select(Some("server")).unwrap();
        assert_eq!(
            (server.entry.as_str(), server.prefix.as_str()),
            ("game.fun", "server")
        );
    }

    #[test]
    fn an_object_entry_without_prefix_is_unprefixed() {
        let config = named_json(&[("client", serde_json::json!({ "file": "client.fun" }))]);
        let client = config.select(None).unwrap();
        assert_eq!(
            (client.entry.as_str(), client.prefix.as_str()),
            ("client.fun", "")
        );
    }

    #[test]
    fn an_object_entry_without_file_is_refused() {
        let config = named_json(&[("server", serde_json::json!({ "prefix": "server" }))]);
        let err = config.select(Some("server")).unwrap_err();
        assert!(err.to_string().contains("needs a \"file\""), "{err}");
        assert!(err.to_string().contains("server"), "{err}");
    }

    #[test]
    fn a_non_string_prefix_is_refused() {
        let config = named_json(&[(
            "server",
            serde_json::json!({ "file": "game.fun", "prefix": 3 }),
        )]);
        let err = config.select(Some("server")).unwrap_err();
        assert!(err.to_string().contains("must be a string"), "{err}");
    }

    #[test]
    fn a_non_identifier_prefix_is_refused() {
        let config = named_json(&[(
            "server",
            serde_json::json!({ "file": "game.fun", "prefix": "my server" }),
        )]);
        let err = config.select(Some("server")).unwrap_err();
        assert!(err.to_string().contains("valid identifier"), "{err}");
    }

    #[test]
    fn an_unknown_object_key_is_refused() {
        let config = named_json(&[(
            "server",
            serde_json::json!({ "file": "game.fun", "prefx": "server" }),
        )]);
        let err = config.select(Some("server")).unwrap_err();
        assert!(err.to_string().contains("unknown key \"prefx\""), "{err}");
    }

    #[test]
    fn conflicting_entry_and_entries_are_refused() {
        let config = FunctorLangConfig {
            entries: FunctorLangEntries::Conflicting,
            mouse_capture: Ok(None),
            cursor: None,
        };
        let err = config.select(None).unwrap_err();
        assert!(
            err.to_string().contains("both `entry` and `entries`"),
            "{err}"
        );
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
            mouse_capture: Ok(None),
            cursor: None,
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
        let examples: std::path::PathBuf = [env!("CARGO_MANIFEST_DIR"), "..", "examples"]
            .iter()
            .collect();
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
            let name = dir
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
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
