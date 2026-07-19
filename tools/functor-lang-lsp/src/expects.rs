//! Live `expect` test status — the editor half of inline tests.
//!
//! On every buffer edit the server immediately pushes the project's expects
//! as `running` (the in-flight state); once the debounce settles, a WORKER
//! thread reloads the project from the live buffers and evaluates the
//! expects with [`functor_lang::run_expects_budgeted`], and the results
//! re-enter the server loop (as `$/functorExpects`, generation-tagged so a
//! stale run never paints) to be relayed to the client as the custom
//! notification [`STATUS`]:
//!
//! ```json
//! { "generation": 7, "files": { "file:///…/game.fun": [
//!     { "line": 12, "state": "pass",       "detail": null },
//!     { "line": 14, "state": "fail",       "detail": "left == right — left: 12, right: 12.5" },
//!     { "line": 15, "state": "error",      "detail": "game.fun:3:5: no pattern matched 1" },
//!     { "line": 16, "state": "unrunnable", "detail": "unknown external `Scene.cube` — engine calls need the runtime; run `functor test` or the game" }
//! ] } }
//! ```
//!
//! `line` is 0-based (LSP convention), the line of the `expect` keyword.
//! States: `running` | `pass` | `fail` | `error` | `unrunnable` (an expect
//! that calls an engine external — the plain evaluator has no host).
//!
//! The worker thread's stack follows `run_expects_budgeted`'s contract
//! (value depth ≤ budget ⇒ ~100 bytes/level): the default 10^6 budget gets
//! a 256MB reserved stack, scaled up if `FUNCTOR_LSP_EXPECT_BUDGET` raises
//! the budget. A budget of 0 disables the feature.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{json, Value};

/// The client-facing notification method.
pub const STATUS: &str = "functor/tests/status";

/// The default per-phase step budget (see `run_expects_budgeted`): well
/// under a second of interpreter work, far above any sane test's needs.
pub const DEFAULT_BUDGET: u64 = 1_000_000;

/// One expect's status row.
pub struct Row {
    pub uri: String,
    /// 0-based line of the `expect` keyword.
    pub line: u64,
    pub state: &'static str,
    pub detail: Option<String>,
}

/// The `functor/tests/status` params for a set of rows. `all_uris` seeds an
/// explicit (possibly empty) entry for EVERY project file, so a file whose
/// last expect was just deleted gets an authoritative `[]` and the client
/// clears its gutter — absent uris are left untouched client-side.
pub fn status_params(generation: u64, rows: &[Row], all_uris: &[String]) -> Value {
    // BTreeMap: deterministic file order in the payload (tests diff it).
    let mut files: BTreeMap<&str, Vec<Value>> = BTreeMap::new();
    for uri in all_uris {
        files.entry(uri).or_default();
    }
    for row in rows {
        files.entry(&row.uri).or_default().push(json!({
            "line": row.line,
            "state": row.state,
            "detail": row.detail,
        }));
    }
    json!({ "generation": generation, "files": files })
}

/// Every real (non-synthetic) file of a project as a uri — the authoritative
/// key set for [`status_params`].
pub fn project_uris(
    project: &functor_lang::project::Project,
    path_to_uri: impl Fn(&std::path::Path) -> String,
) -> Vec<String> {
    project
        .sources
        .files()
        .iter()
        .filter(|file| !file.path.to_str().is_some_and(|p| p.starts_with('<')))
        .map(|file| path_to_uri(&file.path))
        .collect()
}

/// Every expect in `project` as a `running` row — the immediate push on
/// each edit, so the gutter shows in-flight state while the debounce and
/// the evaluation worker do their work.
pub fn running_rows(
    project: &functor_lang::project::Project,
    path_to_uri: impl Fn(&std::path::Path) -> String,
) -> Vec<Row> {
    project
        .module
        .expects
        .iter()
        .map(|expect| {
            let file = project.sources.file_at(expect.span.start);
            let (_, line, _) = project.sources.resolve(expect.span.start);
            Row {
                uri: path_to_uri(&file.path),
                line: (line.max(1) - 1) as u64,
                state: "running",
                detail: None,
            }
        })
        .collect()
}

/// Evaluate a project's expects: the final rows plus the project's full uri
/// set (the authoritative keys — a loaded project with zero expects clears
/// gutters via empty lists). `None` means the buffer didn't load (nothing to
/// report; the previous states stand and diagnostics carry the why). Runs on
/// the CALLER's thread — callers are responsible for the stack contract (a
/// worker with `~100 * budget` bytes reserved). Loads the project fresh from
/// `entry` + `overrides` (live buffers), because IR/values are `Rc`-based
/// and cannot cross threads.
pub fn evaluate_rows(
    entry: Option<PathBuf>,
    single: Option<(PathBuf, String)>,
    overrides: std::collections::HashMap<PathBuf, String>,
    budget: u64,
    path_to_uri: impl Fn(&std::path::Path) -> String,
) -> Option<(Vec<Row>, Vec<String>)> {
    let prelude = functor_prelude::modules();
    let project = match entry {
        Some(entry) => functor_lang::project::load_with_prelude(&entry, &overrides, &prelude)
            .ok()
            // Mirror `load_project`'s membership rule: a nearest functor.json
            // whose entry project does NOT contain the requesting file must
            // fall back to the single-file view, or the worker would paint
            // (and authoritatively clear!) files of an unrelated project.
            .filter(|project| {
                single
                    .as_ref()
                    .is_none_or(|(path, _)| project.sources.file_by_path(path).is_some())
            }),
        None => None,
    }
    .or_else(|| {
        let (path, text) = single?;
        functor_lang::project::load_single_file(&path, &text, &prelude).ok()
    })?;
    let row = |span: functor_lang::Span, state: &'static str, detail: Option<String>| {
        let file = project.sources.file_at(span.start);
        let (_, line, _) = project.sources.resolve(span.start);
        Row {
            uri: path_to_uri(&file.path),
            line: (line.max(1) - 1) as u64,
            state,
            detail,
        }
    };
    let located = |error: &functor_lang::RunError| {
        let (file, line, col) = project.sources.resolve(error.span.start);
        format!(
            "{}:{line}:{col}: {}",
            file.path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            error.message
        )
    };
    let uris = project_uris(&project, &path_to_uri);
    let rows = match functor_lang::run_expects_budgeted(&project.module, &mut functor_lang::NoHost, Some(budget)) {
        Ok(reports) => reports
            .iter()
            .map(|report| {
                // The shared outcome mapping (ExpectOutcome::status — the
                // web IDE uses the same one); errors upgrade the bare
                // message to a located rendering.
                let (state, detail) = report.outcome.status();
                let detail = match &report.outcome {
                    functor_lang::ExpectOutcome::Error(error) if state == "error" => {
                        Some(located(error))
                    }
                    _ => detail,
                };
                row(report.span, state, detail)
            })
            .collect(),
        // The def load failed (or exceeded the budget): every expect gets
        // the load error — the tests can't run until the defs do.
        Err(failure) => {
            let detail = format!("defs failed to load: {}", located(&failure.error));
            project
                .module
                .expects
                .iter()
                .map(|expect| row(expect.span, "error", Some(detail.clone())))
                .collect()
        }
    };
    Some((rows, uris))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uri(path: &std::path::Path) -> String {
        format!("file://{}", path.display())
    }

    fn rows_for(src: &str, budget: u64) -> Vec<Row> {
        let dir = std::env::temp_dir().join(format!("functor-lsp-expects-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("game.fun");
        std::fs::write(&path, src).unwrap();
        let (rows, _uris) =
            evaluate_rows(None, Some((path, src.to_string())), Default::default(), budget, uri)
                .expect("loadable source");
        let _ = std::fs::remove_dir_all(&dir);
        rows
    }

    #[test]
    fn outcomes_map_to_states_with_details() {
        let src = "let area = (w, h) => w * h\n\
                   expect area(3.0, 4.0) == 12.0\n\
                   expect area(3.0, 4.0) == 12.5\n\
                   expect Scene.cube() == Scene.cube()\n";
        let rows = rows_for(src, DEFAULT_BUDGET);
        assert_eq!(
            rows.iter().map(|r| (r.line, r.state)).collect::<Vec<_>>(),
            vec![(1, "pass"), (2, "fail"), (3, "unrunnable")]
        );
        assert_eq!(
            rows[1].detail.as_deref(),
            Some("left == right — left: 12, right: 12.5")
        );
        assert!(rows[2].detail.as_deref().unwrap().contains("Scene.cube"));
    }

    #[test]
    fn budget_exhaustion_is_an_error_row() {
        let src = "let sum = (n) => List.range(n) |> List.fold((a, x) => a + x, 0.0)\n\
                   expect sum(10000.0) == 0.0\n";
        let rows = rows_for(src, 100);
        assert_eq!(rows[0].state, "error");
        assert!(rows[0].detail.as_deref().unwrap().contains("step budget"));
    }

    #[test]
    fn status_params_group_rows_by_file_and_seed_empty_files() {
        let rows = vec![
            Row { uri: "file:///a.fun".into(), line: 3, state: "pass", detail: None },
            Row { uri: "file:///b.fun".into(), line: 0, state: "running", detail: None },
            Row { uri: "file:///a.fun".into(), line: 5, state: "fail", detail: Some("x".into()) },
        ];
        let all = vec![
            "file:///a.fun".to_string(),
            "file:///b.fun".to_string(),
            "file:///empty.fun".to_string(),
        ];
        let params = status_params(9, &rows, &all);
        assert_eq!(params["generation"], 9);
        assert_eq!(params["files"]["file:///a.fun"].as_array().unwrap().len(), 2);
        assert_eq!(params["files"]["file:///b.fun"][0]["state"], "running");
        // The expect-less file gets an authoritative empty list (clears it).
        assert_eq!(params["files"]["file:///empty.fun"].as_array().unwrap().len(), 0);
    }
}
