//! Run a game project's inline `expect` tests under the ENGINE prelude,
//! headlessly — no GL context, no window, no game loop.
//!
//! `functor-lang test` (the language crate's CLI) evaluates expects under the
//! plain `NoHost` prelude, so it fails on any project whose modules mention
//! `Scene.*` / `Sprite.*` / `Camera3D.*` — and `file = module` means *every*
//! sibling loads, so one rendering module is enough to make a whole game
//! untestable. This module supplies the real [`FunctorHost`] instead, which is
//! the same host the shells run and is a unit struct: no external in the
//! registry touches GL, performs IO, or spawns a thread — they build protocol
//! values (a `Scene.cube(…)` builds a scene node) or *descriptors* (an
//! `Effect.*` describes an effect; performing it is the producer's effect
//! broker, which never runs here). So it is safe to instantiate off-GPU.
//!
//! Two externals are not quite pure: the `Ui.*` widget constructors push into
//! the prelude's `UI_HANDLERS` thread-local. The producer drains that after
//! every `ui(model)` evaluation; [`run_project_expects`] does the same at the
//! end, so a project whose expects construct widgets doesn't leak closures.
//!
//! This is the library core behind `functor test`; the CLI command is a thin
//! wrapper that renders [`ExpectRun`] as diagnostics.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::functor_lang_prelude::FunctorHost;

/// One evaluated `expect`, located in the file that wrote it.
#[derive(Debug, Clone)]
pub struct ExpectCase {
    pub file: PathBuf,
    pub line: usize,
    pub col: usize,
    /// The offending source line, carried from the already-in-memory
    /// `SourceFile` so the caller renders a caret without re-reading (and
    /// without risking a *different* snapshot than the one evaluated).
    pub source_line: Option<String>,
    /// `None` when the expect held; otherwise the human-facing reason (a
    /// rendered comparison, or a located runtime error).
    pub failure: Option<String>,
}

/// Every expect in the project, in source order.
#[derive(Debug, Clone)]
pub struct ExpectRun {
    pub cases: Vec<ExpectCase>,
}

impl ExpectRun {
    pub fn total(&self) -> usize {
        self.cases.len()
    }

    pub fn failed(&self) -> usize {
        self.cases.iter().filter(|c| c.failure.is_some()).count()
    }

    pub fn passed(&self) -> usize {
        self.total() - self.failed()
    }
}

/// The run never started: the project would not load, or evaluating its
/// top-level defs failed (both abort every expect, so they are one error
/// rather than N failures).
#[derive(Debug, Clone)]
pub struct ExpectRunError {
    pub file: PathBuf,
    pub line: usize,
    pub col: usize,
    pub message: String,
}

/// Load `entry` as a project (the entry plus every sibling `.fun`, plus the
/// engine's bundled modules and `.funi` interfaces — exactly what `build` and
/// the producers load) and evaluate its `expect` tests under the engine host.
///
/// Callers that already hold a loaded (and typechecked) project should use
/// [`run_expects_in`] instead — loading twice would let an edit land between
/// the check and the run, so the bytes evaluated would not be the bytes
/// verified.
pub fn run_project_expects(entry: &Path) -> Result<ExpectRun, ExpectRunError> {
    let project = functor_lang::project::load_with_bundled_modules(
        entry,
        &HashMap::new(),
        &functor_prelude::bundled_modules(),
    )
    .map_err(|e| ExpectRunError {
        file: e.path,
        line: e.line,
        col: e.col,
        message: e.message,
    })?;
    run_expects_in(&project)
}

/// Evaluate an ALREADY-LOADED project's `expect` tests under the engine host.
///
/// Like `functor-lang test`, this does NOT typecheck: `check` is the static
/// gate, and callers that want it (the `functor test` command does) run it on
/// this same project first. A non-bool expect therefore reports as its own
/// failure here.
///
/// Expects from the engine's own bundled modules are EXCLUDED: they are not
/// the user's tests, and their `<builtin>/…` paths are not files anyone can
/// open. (`build`'s module count filters the same marker.)
pub fn run_expects_in(
    project: &functor_lang::project::Project,
) -> Result<ExpectRun, ExpectRunError> {
    let reports = functor_lang::run_expects(&project.module, &mut FunctorHost).map_err(|failure| {
        let (file, line, col) = project.sources.resolve(failure.error.span.start);
        ExpectRunError {
            file: file.path.clone(),
            line,
            col,
            message: failure.error.message.clone(),
        }
    });
    // The `Ui.*` constructors register handlers in a thread-local the producer
    // drains per frame; nothing consumes them here, so drop whatever the
    // expects accumulated (also on the error path — the def load runs first).
    let _ = crate::functor_lang_prelude::take_ui_handlers();
    let reports = reports?;

    let cases = reports
        .iter()
        .filter(|report| {
            !project
                .sources
                .resolve(report.span.start)
                .0
                .path
                .starts_with("<builtin>")
        })
        .map(|report| {
            let (file, line, col) = project.sources.resolve(report.span.start);
            let failure = match &report.outcome {
                functor_lang::ExpectOutcome::Pass => None,
                functor_lang::ExpectOutcome::Fail(Some(cmp)) => Some(format!(
                    "expect failed: left {} right — left: {}, right: {}",
                    cmp.op, cmp.lhs, cmp.rhs
                )),
                functor_lang::ExpectOutcome::Fail(None) => {
                    Some("expect failed: expected true, got false".to_string())
                }
                // The error's own span can be inside a callee (a different
                // file), so carry its location in the text — the case stays
                // reported at the `expect` the developer wrote.
                functor_lang::ExpectOutcome::Error(error) => Some(format!(
                    "expect errored: {}",
                    project.sources.render(error.span.start, &error.message)
                )),
            };
            ExpectCase {
                file: file.path.clone(),
                line,
                col,
                source_line: nth_line(&file.src, line),
                failure,
            }
        })
        .collect();

    Ok(ExpectRun { cases })
}

/// The 1-based `line`th line of `src`, for the diagnostic caret.
fn nth_line(src: &str, line: usize) -> Option<String> {
    src.lines().nth(line.checked_sub(1)?).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write a throwaway project directory and return its entry path.
    fn project(name: &str, files: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        for (file, src) in files {
            std::fs::write(dir.path().join(file), src).expect("write");
        }
        let entry = dir.path().join(name);
        (dir, entry)
    }

    /// The whole point. `NoHost` cannot evaluate `Color.rgb`, and it is a
    /// TOP-LEVEL def, so under `functor-lang test` the def load aborts before
    /// any expect runs — the exact error the platformer jam entry hit
    /// (`game.fun: unknown external `Color.rgb``) that forced it to copy its
    /// pure modules to a scratch directory.
    #[test]
    fn expects_run_under_the_engine_prelude_which_nohost_cannot_load() {
        let (_dir, entry) = project(
            "game.fun",
            &[(
                "game.fun",
                r#"let sky = Color.rgb(0.1, 0.2, 0.3)
let ground = Scene.lit(sky, Scene.plane())

let double = (n) => n * 2.0

expect double(2.0) == 4.0
expect List.length([Scene.cube(), Scene.sphere()]) == 2
"#,
            )],
        );
        let run = run_project_expects(&entry).expect("project runs");
        assert_eq!(run.total(), 2);
        assert_eq!(run.failed(), 0, "{:?}", run.cases);
    }

    #[test]
    fn a_failing_expect_is_reported_with_its_location_and_does_not_stop_the_rest() {
        let (_dir, entry) = project(
            "game.fun",
            &[(
                "game.fun",
                "let double = (n) => n * 2.0\nexpect double(2.0) == 5.0\nexpect double(3.0) == 6.0\n",
            )],
        );
        let run = run_project_expects(&entry).expect("project runs");
        assert_eq!(run.total(), 2);
        assert_eq!(run.failed(), 1);
        let bad = &run.cases[0];
        assert_eq!(bad.line, 2);
        assert_eq!(bad.file.file_name().unwrap(), "game.fun");
        let message = bad.failure.as_deref().expect("a failure detail");
        assert!(message.contains('4'), "{message}");
        assert!(message.contains('5'), "{message}");
        assert!(run.cases[1].failure.is_none());
    }

    #[test]
    fn a_sibling_module_contributes_its_expects() {
        let (_dir, entry) = project(
            "game.fun",
            &[
                ("game.fun", "let sky = Color.rgb(0.1, 0.2, 0.3)\n"),
                (
                    "helpers.fun",
                    "let inc = (n) => n + 1.0\nexpect inc(1.0) == 2.0\n",
                ),
            ],
        );
        let run = run_project_expects(&entry).expect("project runs");
        assert_eq!(run.total(), 1);
        assert_eq!(run.cases[0].file.file_name().unwrap(), "helpers.fun");
        assert_eq!(run.failed(), 0);
    }

    #[test]
    fn a_project_that_will_not_load_is_one_positioned_error() {
        let (_dir, entry) = project("game.fun", &[("game.fun", "let broken = (\n")]);
        let err = run_project_expects(&entry).expect_err("a load error");
        assert_eq!(err.file.file_name().unwrap(), "game.fun");
        assert!(!err.message.is_empty());
    }

    /// Unit-suffix literals under the ENGINE prelude: the built-in suffixes
    /// evaluate to real branded values, and a project's own unit builds
    /// exactly what the handwritten constructor call does.
    #[test]
    fn unit_suffix_literals_evaluate_under_the_engine_prelude() {
        let (_dir, entry) = project(
            "game.fun",
            &[(
                "game.fun",
                r#"type Px = | Px(value: float)

unit px = Px

expect List.length([90deg, 1.5rad]) == 2
expect List.length([0.5s, 500ms, 250us, 2min, 1hr]) == 5
expect 16px == Px(16.0)
expect -2.5px == Px(-2.5)
expect List.length([Scene.cube() |> Scene.rotateY(90deg)]) == 1
"#,
            )],
        );
        let run = run_project_expects(&entry).expect("project runs");
        assert_eq!(run.total(), 5);
        assert_eq!(run.failed(), 0, "{:?}", run.cases);
    }

    #[test]
    fn a_project_with_no_expects_runs_clean() {
        let (_dir, entry) = project("game.fun", &[("game.fun", "let x = 1.0\n")]);
        let run = run_project_expects(&entry).expect("project runs");
        assert_eq!(run.total(), 0);
        assert_eq!(run.failed(), 0);
    }
}
