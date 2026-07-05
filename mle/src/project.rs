//! Multi-file MLE projects — B8 part 1 (docs/mle.md).
//!
//! **File = module.** Every `.mle` file in the entry file's directory IS a
//! module, named by its filename stem with the first letter capitalized
//! (`utils.mle` → `Utils`); the entry file is the program root. Loading is
//! **eager**: all files parse, lower, and (via [`crate::check`] on the
//! result) typecheck together — unreferenced modules still get diagnostics,
//! because whole-program checking is the point.
//!
//! [`load`] produces ONE merged [`Module`]: each file lowers with a
//! [`crate::lower`] project environment that canonicalizes its names (a
//! non-entry module `M`'s defs/types/constructor tags become `M.name`; the
//! entry stays bare — a single-file project is byte-identical to
//! single-file lowering), spans are offset per file into one project-wide
//! span space (rendered back by [`SourceMap`]), and ID counters thread
//! across files. Downstream — `Session`, the checker, `rebind` — consume
//! the merged module unchanged: cross-module calls are ordinary late-bound
//! globals ("Utils.clamp"), and stable rebind ids inherit the module prefix
//! from the def names.
//!
//! **Cycles are refused.** Any cross-file reference (a qualified use, an
//! `open`, a type annotation) is a dependency edge; the module graph must
//! be a DAG, and a cycle fails loud with its path (`Game → Utils → Game`).
//! Within a file, letrec-style mutual visibility is unchanged. The DAG
//! also gives the evaluation order: a module's top-level initializers run
//! after those of every module it depends on.
//!
//! **Protected names.** A file whose module name collides with a
//! builtin/prelude namespace (`List`, `Scene`, …) is refused — otherwise
//! `Scene.cube` would silently stop meaning the prelude.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::ast;
use crate::ir::Module;
use crate::lower::{exports_of, lower_in_project, Exports, IdBases, ProjectEnv};
use crate::parser::{capitalize, parse_with_base};
use crate::span::line_col;
use crate::types::RecordLiteralScopes;
use crate::CheckError;

/// Namespaces the language or the Functor prelude own; a module (file) name
/// colliding with one is a load-time error.
const PROTECTED_NAMESPACES: &[&str] = &[
    "Net",
    "List",
    "Text",
    "Math",
    "Scene",
    "Camera",
    "Frame",
    "Light",
    "Angle",
    "Time",
    "Sub",
    "Effect",
    "Physics",
    "RenderTarget",
];

/// A loaded multi-file program: the merged module plus the per-file source
/// map that renders its project-wide spans.
pub struct Project {
    /// The merged, name-canonicalized module (defs in dependency order).
    pub module: Module,
    pub sources: SourceMap,
    /// The entry file's module name (`game.mle` → `Game`).
    pub entry: String,
    /// Per-module record-literal visibility (own + `open`ed types) —
    /// [`Project::check`] hands it to the checker so a bare literal never
    /// resolves against an unrelated sibling's type.
    scopes: RecordLiteralScopes,
}

impl Project {
    /// Typecheck the whole program (every module, referenced or not) —
    /// [`crate::check`] with the project's record-literal scopes. Spans
    /// render through [`Project::sources`].
    pub fn check(&self) -> Vec<CheckError> {
        crate::types::check_with_scopes(&self.module, &self.scopes)
    }
}

/// One file of a project, with its base offset in the project-wide span
/// space (see [`crate::lexer::lex`]).
/// The built-in `Net` module (see the injection site in [`load_with_entry_source`]).
/// Connection ids are `Float` (small integers); text carries messages and
/// error strings. Mirrors F#'s `Functor.Net.NetEvent`.
const NET_MODULE_SRC: &str = "type NetEvent =\n\
     | Connected(id: Float)\n\
     | Message(id: Float, text: String)\n\
     | Disconnected(id: Float)\n\
     | Error(id: Float, text: String)\n";

pub struct SourceFile {
    pub path: PathBuf,
    /// The module name derived from the file name.
    pub module: String,
    pub src: String,
    pub base: usize,
}

/// Renders project-wide span offsets back to `file:line:col`.
pub struct SourceMap {
    /// Ascending by `base`; the entry file first (base 0), so span-less
    /// errors (`Span 0..0`) attribute to the entry, as they did before B8.
    files: Vec<SourceFile>,
}

impl SourceMap {
    pub fn files(&self) -> &[SourceFile] {
        &self.files
    }

    /// The file containing project-wide offset `offset` (the entry file for
    /// out-of-range offsets — fail soft, this is error rendering).
    pub fn file_at(&self, offset: usize) -> &SourceFile {
        self.files
            .iter()
            .rev()
            .find(|file| file.base <= offset)
            .unwrap_or(&self.files[0])
    }

    /// Render a project-wide offset as (path, line, col).
    pub fn resolve(&self, offset: usize) -> (&SourceFile, usize, usize) {
        let file = self.file_at(offset);
        let local = (offset - file.base).min(file.src.len());
        let (line, col) = line_col(&file.src, local);
        (file, line, col)
    }

    /// Render an error at a project-wide span offset: `path:line:col: message`.
    pub fn render(&self, offset: usize, message: &str) -> String {
        let (file, line, col) = self.resolve(offset);
        format!("{}:{line}:{col}: {message}", file.path.display())
    }
}

/// A project load failure, positioned in the offending file (line/col are
/// 1-based; project-level problems — a bad module name, a cycle — report at
/// 1:1 of the file they concern).
pub struct ProjectError {
    pub path: PathBuf,
    pub line: usize,
    pub col: usize,
    pub message: String,
}

impl ProjectError {
    /// `path:line:col: message` — the CLI/producer error shape.
    pub fn render(&self) -> String {
        format!(
            "{}:{}:{}: {}",
            self.path.display(),
            self.line,
            self.col,
            self.message
        )
    }
}

/// The `.mle` files of the project rooted at `entry_path`: the entry first,
/// then its sibling `.mle` files in name order. (Subdirectories are not
/// scanned — a directory is one flat module space.)
pub fn project_files(entry_path: &Path) -> std::io::Result<Vec<PathBuf>> {
    // A bare relative entry ("game.mle") has an EMPTY parent — that still
    // means the current directory, not "no siblings".
    let dir = match entry_path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let mut siblings: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().and_then(|e| e.to_str()) == Some("mle")
                && path.is_file()
                && path.file_name() != entry_path.file_name()
        })
        .collect();
    siblings.sort();
    let mut files = vec![entry_path.to_path_buf()];
    files.extend(siblings);
    Ok(files)
}

/// Load the multi-file project rooted at `entry_path` (see the module doc).
pub fn load(entry_path: &Path) -> Result<Project, ProjectError> {
    load_with_entry_source(entry_path, None)
}

/// [`load`], with the entry file's source overridden (the live-preview /
/// `reload-source` push path: the pushed buffer stands in for the entry
/// file on disk; siblings still load from disk).
pub fn load_with_entry_source(
    entry_path: &Path,
    entry_src: Option<String>,
) -> Result<Project, ProjectError> {
    let at = |path: &Path, message: String| ProjectError {
        path: path.to_path_buf(),
        line: 1,
        col: 1,
        message,
    };

    let paths = project_files(entry_path).map_err(|e| {
        at(
            entry_path,
            format!("cannot read the project directory: {e}"),
        )
    })?;

    // Derive and validate module names; read sources; assign span bases.
    let mut files: Vec<SourceFile> = Vec::new();
    let mut base = 0usize;
    for (index, path) in paths.iter().enumerate() {
        let module = module_name(path).map_err(|message| at(path, message))?;
        if PROTECTED_NAMESPACES.contains(&module.as_str()) {
            return Err(at(
                path,
                format!(
                    "module name `{module}` (from {}) collides with the builtin/prelude \
namespace `{module}` — rename the file",
                    file_name(path)
                ),
            ));
        }
        if let Some(previous) = files.iter().find(|f| f.module == module) {
            return Err(at(
                path,
                format!(
                    "module name `{module}` (from {}) is already taken by {} — module names \
come from file names, capitalized",
                    file_name(path),
                    file_name(&previous.path)
                ),
            ));
        }
        let src = match (index, &entry_src) {
            (0, Some(src)) => src.clone(),
            _ => std::fs::read_to_string(path)
                .map_err(|e| at(path, format!("cannot read {}: {e}", path.display())))?,
        };
        let len = src.len();
        files.push(SourceFile {
            path: path.clone(),
            module,
            src,
            base,
        });
        // +1 gap: a span at one file's very end never collides with the
        // next file's base.
        base += len + 1;
    }
    // The built-in `Net` module: a prelude-provided ADT always in scope, so
    // any game can `match ev with | Net.Connected(id) => …` without
    // declaring the type. It's an ordinary non-entry module (canonicalized
    // to `Net.NetEvent` / `Net.Connected` / …), loaded through the same
    // path — the host builds matching `Net.*` variant values (see
    // `mle_prelude`). Injected LAST so the entry stays index 0.
    files.push(SourceFile {
        path: PathBuf::from("<builtin>/Net.mle"),
        module: "Net".to_string(),
        src: NET_MODULE_SRC.to_string(),
        base,
    });
    let entry = files[0].module.clone();

    // Parse every file (spans land in the project-wide space).
    let render_span = |files: &[SourceFile], index: usize, span: crate::Span, message: &str| {
        let file = &files[index];
        let local = (span.start - file.base).min(file.src.len());
        let (line, col) = line_col(&file.src, local);
        ProjectError {
            path: file.path.clone(),
            line,
            col,
            message: message.to_string(),
        }
    };
    let mut programs: Vec<ast::Program> = Vec::new();
    for (index, file) in files.iter().enumerate() {
        let program = parse_with_base(&file.src, file.base)
            .map_err(|e| render_span(&files, index, e.span, &e.message))?;
        programs.push(program);
    }

    // Every module's exports, for cross-module resolution during lowering.
    let exports: HashMap<String, Exports> = files
        .iter()
        .zip(&programs)
        .map(|(file, program)| (file.module.clone(), exports_of(program)))
        .collect();

    // Lower each file with the project environment, threading ID bases so
    // the merged module is one ID space. Collect dependency edges.
    let mut bases = IdBases::default();
    let mut lowered: Vec<Module> = Vec::new();
    let mut deps: HashMap<String, HashSet<String>> = HashMap::new();
    let mut scopes = RecordLiteralScopes::default();
    for (index, (file, program)) in files.iter().zip(programs).enumerate() {
        // Record-literal visibility for this module: its own types plus its
        // `open`ed modules' (by canonical name — the entry's are bare).
        let canon = |module: &str, name: &str| {
            if module == entry {
                name.to_string()
            } else {
                format!("{module}.{name}")
            }
        };
        let mut visible: HashSet<String> = exports[&file.module]
            .types
            .iter()
            .map(|name| canon(&file.module, name))
            .collect();
        for item in &program.items {
            let ast::Item::Open(decl) = item else {
                continue;
            };
            if let Some(opened) = exports.get(&decl.module) {
                visible.extend(opened.types.iter().map(|name| canon(&decl.module, name)));
            }
        }
        let prefix = if file.module == entry {
            String::new()
        } else {
            file.module.clone()
        };
        scopes.by_module.insert(prefix, visible);

        let env = ProjectEnv {
            name: &file.module,
            entry: &entry,
            modules: &exports,
        };
        let (module, next, module_deps) = lower_in_project(program, &env, bases)
            .map_err(|e| render_span(&files, index, e.span, &e.message))?;
        bases = next;
        lowered.push(module);
        deps.insert(file.module.clone(), module_deps);
    }

    // Refuse dependency cycles (fail loud with the path), and derive the
    // evaluation order: dependencies before dependents. Iteration order is
    // file order (entry first), so the result is deterministic.
    let order = dependency_order(&files, &deps).map_err(|cycle| {
        let start = &cycle[0];
        let path = files
            .iter()
            .find(|f| &f.module == start)
            .map(|f| f.path.clone())
            .unwrap_or_else(|| entry_path.to_path_buf());
        ProjectError {
            path,
            line: 1,
            col: 1,
            message: format!(
                "modules depend on each other in a cycle: {} — cross-file cycles are not \
allowed (within one file, definitions may still be mutually recursive)",
                cycle.join(" → ")
            ),
        }
    })?;

    // Merge, in dependency order: a module's top-level initializers run
    // after those of every module it depends on.
    let mut merged = Module {
        types: Vec::new(),
        defs: Vec::new(),
    };
    let mut by_module: HashMap<String, Module> = files
        .iter()
        .map(|f| f.module.clone())
        .zip(lowered)
        .collect();
    for name in &order {
        let module = by_module.remove(name).expect("ordered once");
        merged.types.extend(module.types);
        merged.defs.extend(module.defs);
    }

    Ok(Project {
        module: merged,
        sources: SourceMap { files },
        entry,
        scopes,
    })
}

/// The module name a file provides: its stem, first letter capitalized
/// (`utils.mle` → `Utils`). The stem must be a valid identifier — module
/// names appear in source (`Utils.clamp`).
fn module_name(path: &Path) -> Result<String, String> {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let valid = !stem.is_empty()
        && stem.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && stem.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !valid {
        return Err(format!(
            "cannot derive a module name from `{}` — file stems must be identifiers \
(letters, digits, `_`; starting with a letter), like `utils.mle` → `Utils`",
            file_name(path)
        ));
    }
    Ok(capitalize(stem))
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Topologically order the modules (dependencies first), or return a cycle
/// path (`["Game", "Utils", "Game"]`) if the graph is not a DAG.
fn dependency_order(
    files: &[SourceFile],
    deps: &HashMap<String, HashSet<String>>,
) -> Result<Vec<String>, Vec<String>> {
    #[derive(Clone, Copy, PartialEq)]
    enum State {
        Unvisited,
        InProgress,
        Done,
    }
    let names: Vec<&String> = files.iter().map(|f| &f.module).collect();
    let mut state: HashMap<&str, State> = names
        .iter()
        .map(|n| (n.as_str(), State::Unvisited))
        .collect();
    let mut order: Vec<String> = Vec::new();

    // Iterative DFS with an explicit path for the cycle report. Sorted
    // dependency iteration keeps the order (and any cycle report)
    // deterministic.
    fn sorted_deps<'a>(deps: &'a HashMap<String, HashSet<String>>, name: &str) -> Vec<&'a String> {
        let mut list: Vec<&String> = deps
            .get(name)
            .map(|s| s.iter().collect())
            .unwrap_or_default();
        list.sort();
        list
    }
    for root in &names {
        if state[root.as_str()] != State::Unvisited {
            continue;
        }
        // (module, next dep index)
        let mut stack: Vec<(&String, usize)> = vec![(root, 0)];
        state.insert(root.as_str(), State::InProgress);
        while let Some(&mut (node, ref mut next)) = stack.last_mut() {
            let node_deps = sorted_deps(deps, node);
            if *next < node_deps.len() {
                let dep = node_deps[*next];
                *next += 1;
                // Unknown dep names can't occur (lowering validated them);
                // ignore defensively rather than panic.
                match state.get(dep.as_str()).copied() {
                    Some(State::Unvisited) => {
                        state.insert(dep.as_str(), State::InProgress);
                        stack.push((dep, 0));
                    }
                    Some(State::InProgress) => {
                        // Cycle: the path from the first occurrence of `dep`
                        // on the stack, closed with `dep` itself.
                        let from = stack
                            .iter()
                            .position(|(n, _)| *n == dep)
                            .unwrap_or_default();
                        let mut cycle: Vec<String> =
                            stack[from..].iter().map(|(n, _)| (*n).clone()).collect();
                        cycle.push(dep.clone());
                        return Err(cycle);
                    }
                    Some(State::Done) | None => {}
                }
            } else {
                state.insert(node.as_str(), State::Done);
                order.push(node.clone());
                stack.pop();
            }
        }
    }
    Ok(order)
}
