//! Multi-file Functor Lang projects — B8 part 1 (docs/functor-lang.md).
//!
//! **File = module.** Every `.fun` file in the entry file's directory IS a
//! module, named by its filename stem with the first letter capitalized
//! (`utils.fun` → `Utils`); the entry file is the program root. Loading is
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
use crate::lower::{
    exports_of, lower_in_project, open_module_path, resolve_units, unit_decls, Exports, IdBases,
    ProjectEnv, UnitTarget,
};
use crate::parser::{capitalize, parse_interface_with_base, parse_with_base};
use crate::span::line_col;
use crate::types::RecordLiteralScopes;
use crate::CheckError;

/// Namespaces the language or the Functor prelude own; a module (file) name
/// colliding with one is a load-time error.
const PROTECTED_NAMESPACES: &[&str] = &[
    "Net",
    "Key",
    "List",
    "Map",
    "Text",
    "Math",
    "Random",
    "Scene",
    "Anim",
    "Terrain",
    "Asset",
    "Camera3D",
    "Camera2D",
    "Sprite",
    "Frame",
    "Light",
    "Fog",
    "Color",
    "Vec3",
    "Skybox",
    "Angle",
    "Texture",
    "Time",
    "Input",
    "Sub",
    "Effect",
    "Persistence",
    "Physics",
    "RenderTarget",
    "Ui",
    "Html",
    "Attr",
    "Style",
    "AudioSource",
    "AudioScene",
    "Debug",
];

/// Whether a module name is one the language or the Functor prelude owns.
///
/// Exposed so a host that SHIPS a namespace can assert its own modules are
/// reserved here — a new prelude module that nobody adds to the list above is
/// silently shadowable by a game's file or inline `module`.
pub fn is_protected_namespace(name: &str) -> bool {
    PROTECTED_NAMESPACES.contains(&name)
}

/// One source module bundled by the language or a host rather than read from
/// the user's project directory.
///
/// Implementation modules are ordinary `.fun` code with executable bodies;
/// interface modules are `.funi` declarations whose values remain
/// host-provided externals. Both travel through the same parser, lowerer,
/// checker, dependency graph, evaluator, source map, and reload-rebind path as
/// project modules.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BundledModuleKind {
    Implementation,
    Interface,
}

/// An in-memory module distributed with the language or host runtime.
#[derive(Clone, Debug)]
pub struct BundledModule {
    name: String,
    src: String,
    kind: BundledModuleKind,
    directory: &'static str,
}

impl BundledModule {
    /// Bundle a reusable `.fun` implementation module. Diagnostics and
    /// go-to-definition identify it as `<stdlib>/{name}.fun`.
    pub fn implementation(name: impl Into<String>, src: impl Into<String>) -> BundledModule {
        BundledModule::new(name, src, BundledModuleKind::Implementation, "<stdlib>")
    }

    /// Bundle a host `.funi` interface module. Diagnostics and
    /// go-to-definition keep the established `<prelude>/{name}.funi` path.
    pub fn interface(name: impl Into<String>, src: impl Into<String>) -> BundledModule {
        BundledModule::new(name, src, BundledModuleKind::Interface, "<prelude>")
    }

    /// Module name used by qualified access (`Animator`, `Scene`, …).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Embedded source text linked for this module.
    pub fn source(&self) -> &str {
        &self.src
    }

    /// Whether this descriptor carries executable `.fun` source or a
    /// host-backed `.funi` interface.
    pub fn kind(&self) -> BundledModuleKind {
        self.kind
    }

    fn builtin(
        name: impl Into<String>,
        src: impl Into<String>,
        kind: BundledModuleKind,
    ) -> BundledModule {
        BundledModule::new(name, src, kind, "<builtin>")
    }

    fn new(
        name: impl Into<String>,
        src: impl Into<String>,
        kind: BundledModuleKind,
        directory: &'static str,
    ) -> BundledModule {
        BundledModule {
            name: name.into(),
            src: src.into(),
            kind,
            directory,
        }
    }

    fn path(&self) -> PathBuf {
        let extension = match self.kind {
            BundledModuleKind::Implementation => "fun",
            BundledModuleKind::Interface => "funi",
        };
        PathBuf::from(format!("{}/{}.{}", self.directory, self.name, extension))
    }
}

/// A loaded multi-file program: the merged module plus the per-file source
/// map that renders its project-wide spans.
pub struct Project {
    /// The merged, name-canonicalized module (defs in dependency order).
    pub module: Module,
    pub sources: SourceMap,
    /// The entry file's module name (`game.fun` → `Game`).
    pub entry: String,
    /// Every inline `module` block of the project's files, in file order —
    /// the editor tooling's index (which module the cursor is in, folding).
    pub inline_modules: Vec<InlineModule>,
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

    /// [`Project::check`], also returning the per-expression type table — the
    /// project-wide input for `functor_lang::hover` / `functor_lang::inlay` / `functor_lang::codelens`.
    /// Spans (keys into the table's positions) are project-wide; render them
    /// through [`Project::sources`].
    pub fn check_with_types(&self) -> (Vec<CheckError>, crate::types::ExprTypes) {
        crate::types::check_with_scopes_and_types(&self.module, &self.scopes)
    }

    /// The inline `module` block containing project-wide `offset`, if any —
    /// the cursor's namespace for `functor_lang::complete` (blocks never
    /// nest, so at most one matches).
    pub fn inline_module_at(&self, offset: usize) -> Option<&InlineModule> {
        self.inline_modules
            .iter()
            .find(|module| module.span.start <= offset && offset < module.span.end)
    }
}

/// One `module Name { … }` block, indexed for editor tooling.
pub struct InlineModule {
    /// The declared name — how the block is referenced inside its own file
    /// (`Grid`).
    pub name: String,
    /// The declaring file's module (`Utils`).
    pub file: String,
    /// The canonical prefix its members carry: bare in the ENTRY file
    /// (`Server.step`), file-qualified elsewhere (`Utils.Grid.cell`).
    pub path: String,
    /// The whole block, `module` through `}`, in the project-wide span space.
    pub span: crate::Span,
}

/// One file of a project, with its base offset in the project-wide span
/// space (see [`crate::lexer::lex`]).
/// The built-in `Net` module (see the injection site in [`load_with_entry_source`]).
/// Connection ids are `Float` (small integers); text carries messages and
/// error strings. Mirrors F#'s `Functor.Net.NetEvent`. `HttpResponse` is the
/// value handed to an `Effect.httpGet`/`httpPost` tagger: `Response` for a
/// completed request (any HTTP status), `Failure` for a transport error.
///
/// `Data(id, value)` is an `Effect.sendMsg` payload decoded back into a
/// plain-data value. Its field is deliberately typed `unknown` — the EXPLICIT
/// gradual seam: the payload's real type is whatever ADT the two ends share
/// (the same shared-module declaration on both sides), so games match it
/// directly against their own constructors. (It used to say `NetData`, an
/// undeclared name that fell through to `Unknown` implicitly; that fall-through
/// is now a check error, so the seam has to be spelled.)
const NET_MODULE_SRC: &str = "type NetEvent =\n\
     | Connected(id: float)\n\
     | Message(id: float, text: string)\n\
     | Data(id: float, value: unknown)\n\
     | Disconnected(id: float)\n\
     | Error(id: float, text: string)\n\
     type HttpResponse =\n\
     | Response(status: float, body: string)\n\
     | Failure(error: string)\n";

/// The built-in `Random` interface module (injected beside `Net` in [`link`]).
/// `Seed` is an abstract type — the brand that keeps PRNG seeds out of
/// arithmetic — while at runtime a seed stays a plain number (plain data for
/// time-travel snapshots and hot-reload). The values are builtins
/// ([`crate::eval::Builtin`]); these signatures both type them (the checker
/// prefers a signature over a builtin scheme) and keep the qualified names
/// resolving now that a `Random` module exists.
const RANDOM_MODULE_SRC: &str = include_str!("../stdlib/random.funi");

/// The built-in `Key` module (injected beside `Net` in [`link`]): the variant
/// the `input` hook's `key` parameter carries — `Key.W`, `Key.Up`,
/// `Key.Num0` … — so a key typo is a check-time unknown-constructor error
/// instead of a silently dead string arm. The shells build matching
/// `Key.*` values (`functor_runtime_common::Key::ctor_tag`); `Unknown` is
/// filtered before dispatch and deliberately has no constructor here. Keep in
/// sync with the `Key` enum in `functor_runtime_common::input` (the digit row
/// is `Num0`..`Num9` — constructor names must be identifiers).
const KEY_MODULE_SRC: &str = include_str!("../stdlib/key.fun");

/// The built-in `Mouse` module (the mouse twin of `Key`): the variant the
/// `mouseButton` hook's `button` parameter carries — `Mouse.Left`,
/// `Mouse.Right`, `Mouse.Middle` — so a typo is a check-time
/// unknown-constructor error instead of a silently dead arm. The shells build
/// matching `Mouse.*` values (`functor_runtime_common::MouseButton::ctor_tag`);
/// `Unknown` is filtered before dispatch and deliberately has no constructor
/// here. Keep in sync with the `MouseButton` enum in
/// `functor_runtime_common::input`.
const MOUSE_MODULE_SRC: &str = include_str!("../stdlib/mouse.fun");

const OPTION_MODULE_SRC: &str = include_str!("../stdlib/option.fun");
const RESULT_MODULE_SRC: &str = include_str!("../stdlib/result.fun");

/// Documentation-only interfaces for the namespaces the INTERPRETER
/// implements in Rust ([`crate::eval::Builtin`]) rather than in Functor Lang
/// source. They are never linked into a project — the builtin registry
/// already resolves and types those calls — but they give the API-reference
/// generator the same `//!` / `///` prose surface every other module has.
///
/// Their signatures are pinned to [`crate::types::builtin_signature`] by a
/// drift test (`builtin_documentation_matches_the_registry`), so a builtin
/// added, removed, or retyped without updating these files fails the build.
const LIST_DOC_SRC: &str = include_str!("../stdlib/list.funi");
const MAP_DOC_SRC: &str = include_str!("../stdlib/map.funi");
const TEXT_DOC_SRC: &str = include_str!("../stdlib/text.funi");
const MATH_DOC_SRC: &str = include_str!("../stdlib/math.funi");
const DEBUG_DOC_SRC: &str = include_str!("../stdlib/debug.funi");

/// Language-owned modules available in every embedding, including the plain
/// `functor-lang` CLI. Keeping them in the same descriptor shape as host
/// modules is the distribution seam for reusable `.fun` stdlib code.
fn core_modules() -> Vec<BundledModule> {
    vec![
        BundledModule::builtin("Net", NET_MODULE_SRC, BundledModuleKind::Implementation),
        BundledModule::builtin("Random", RANDOM_MODULE_SRC, BundledModuleKind::Interface),
        BundledModule::builtin("Key", KEY_MODULE_SRC, BundledModuleKind::Implementation),
        BundledModule::builtin("Mouse", MOUSE_MODULE_SRC, BundledModuleKind::Implementation),
        BundledModule::implementation("Option", OPTION_MODULE_SRC),
        BundledModule::implementation("Result", RESULT_MODULE_SRC),
    ]
}

/// One standard-library module as the API-reference generator sees it.
///
/// Deliberately NOT a [`BundledModule`]: half of these are documentation-only
/// interfaces for Rust builtins, and linking one would shadow the builtin
/// registry with externals nothing implements. This type cannot be handed to a
/// loader at all.
pub struct DocumentationModule {
    name: &'static str,
    source: &'static str,
    /// Bodyless `.funi` signatures, versus executable `.fun` code whose
    /// signatures come from its own annotations.
    interface: bool,
}

impl DocumentationModule {
    /// Module name used by qualified access (`List`, `Option`, …).
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// The documented source text.
    pub fn source(&self) -> &'static str {
        self.source
    }

    /// Whether the source is a bodyless `.funi` interface.
    pub fn is_interface(&self) -> bool {
        self.interface
    }
}

/// The Functor Lang standard library as documentation sources, in reference
/// order — what `functor docs` renders beside the engine prelude.
///
/// Two kinds of module appear here, and the difference is invisible to a game:
///
/// - the namespaces the interpreter implements in Rust (`List`, `Map`,
///   `Text`, `Math`, `Random`, `Debug`), documented by the `.funi` sources
///   above — `Random`'s is the very module [`core_modules`] injects, and the
///   rest are documentation-only interfaces pinned to the builtin registry by
///   a drift test;
/// - the modules written in Functor Lang and linked into every project
///   (`Option`, `Result`, `Key`, `Mouse`), documented from the exact source
///   that runs, so those cannot drift at all.
pub fn stdlib_documentation_modules() -> Vec<DocumentationModule> {
    let interface = |name, source| DocumentationModule {
        name,
        source,
        interface: true,
    };
    let implementation = |name, source| DocumentationModule {
        name,
        source,
        interface: false,
    };
    vec![
        interface("List", LIST_DOC_SRC),
        interface("Map", MAP_DOC_SRC),
        interface("Text", TEXT_DOC_SRC),
        interface("Math", MATH_DOC_SRC),
        interface("Random", RANDOM_MODULE_SRC),
        interface("Debug", DEBUG_DOC_SRC),
        implementation("Option", OPTION_MODULE_SRC),
        implementation("Result", RESULT_MODULE_SRC),
        implementation("Key", KEY_MODULE_SRC),
        implementation("Mouse", MOUSE_MODULE_SRC),
    ]
}

fn prelude_modules(prelude: &[(String, String)]) -> Vec<BundledModule> {
    prelude
        .iter()
        .map(|(name, src)| BundledModule::interface(name.clone(), src.clone()))
        .collect()
}

pub struct SourceFile {
    pub path: PathBuf,
    /// The module name derived from the file name.
    pub module: String,
    pub src: String,
    pub base: usize,
    /// A `.funi` interface file (bodyless signatures), vs a `.fun`.
    pub interface: bool,
}

/// Whether a path is an interface file (`.funi`).
fn is_interface(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()) == Some("funi")
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

    /// The source file with this path, if the project has one — the LSP's
    /// seam for mapping an editor document (a file path) to its base in the
    /// project-wide span space. Matches on the exact path first, then on the
    /// canonicalized path (a URI-derived path and a `read_dir` path can
    /// differ in form: symlinks, `.`/`..`, absolute vs relative).
    pub fn file_by_path(&self, path: &Path) -> Option<&SourceFile> {
        if let Some(file) = self.files.iter().find(|f| f.path == path) {
            return Some(file);
        }
        let canon = std::fs::canonicalize(path).ok()?;
        self.files
            .iter()
            .find(|f| std::fs::canonicalize(&f.path).ok() == Some(canon.clone()))
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

/// The `.fun` files of the project rooted at `entry_path`: the entry first,
/// then its loadable sibling `.fun` files in name order. (Subdirectories are not
/// scanned — a directory is one flat module space.) Siblings whose stem isn't a
/// valid module identifier — editor temp files like `.#game.fun`, or `2d.fun` —
/// are skipped (see [`scan_project_dir`]); the watcher and dev server both go
/// through here, so they ignore those files too.
pub fn project_files(entry_path: &Path) -> std::io::Result<Vec<PathBuf>> {
    scan_project_dir(entry_path).map(|(kept, _skipped)| kept)
}

/// Scan the entry's directory, partitioning sibling `.fun`/`.funi` files into
/// **kept** (the entry first, then loadable siblings whose stem is a valid module
/// identifier — [`is_module_stem`]) and **skipped** (files that match the
/// extension but whose stem can't be a module: editor temp files like
/// `.#game.fun`, or non-identifier stems like `2d.fun`). Skipping them here — vs
/// letting [`module_name`] fail the whole load — is what keeps a stray editor
/// temp file next to `game.fun` from breaking hot reload. Both lists are sorted;
/// the entry is always kept regardless of its own stem.
fn scan_project_dir(entry_path: &Path) -> std::io::Result<(Vec<PathBuf>, Vec<PathBuf>)> {
    // A bare relative entry ("game.fun") has an EMPTY parent — that still
    // means the current directory, not "no siblings".
    let dir = match entry_path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let mut siblings: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            // `.fun` implementations and `.funi` interface files both load.
            matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("fun" | "funi")
            ) && path.is_file()
                && path.file_name() != entry_path.file_name()
        })
        .collect();
    siblings.sort();

    let (kept_siblings, skipped): (Vec<PathBuf>, Vec<PathBuf>) = siblings
        .into_iter()
        .partition(|path| is_loadable_stem(path));

    let mut kept = vec![entry_path.to_path_buf()];
    kept.extend(kept_siblings);
    Ok((kept, skipped))
}

/// Whether a path's file stem is a valid module identifier (so it can load as a
/// module — see [`is_module_stem`]). A dot-prefixed stem (`.#game`) or a
/// non-identifier stem (`2d`) returns false.
fn is_loadable_stem(path: &Path) -> bool {
    path.file_stem()
        .and_then(|s| s.to_str())
        .is_some_and(is_module_stem)
}

/// Whether `stem` is a valid module identifier: non-empty, starts with an ASCII
/// letter, and is otherwise ASCII alphanumeric or `_`. This is the same rule
/// [`module_name`] enforces — a file whose stem fails it cannot name a module.
fn is_module_stem(stem: &str) -> bool {
    !stem.is_empty()
        && stem.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && stem.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
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
    let overrides = match entry_src {
        Some(src) => HashMap::from([(entry_path.to_path_buf(), src)]),
        None => HashMap::new(),
    };
    load_with_overrides(entry_path, &overrides)
}

/// [`load`], with any project files' on-disk sources replaced by the given
/// in-memory buffers (keyed by path). The LSP's seam: an editor holds
/// unsaved edits for one or more open files (entry or sibling), and the rest
/// load from disk. A key matching no project file is ignored. Matching is by
/// exact path, then canonicalized path (see [`SourceMap::file_by_path`]).
pub fn load_with_overrides(
    entry_path: &Path,
    overrides: &HashMap<PathBuf, String>,
) -> Result<Project, ProjectError> {
    load_with_bundled_modules(entry_path, overrides, &[])
}

/// [`load_with_overrides`], plus a set of injected PRELUDE interface modules —
/// `(module name, .funi source)` pairs describing host-provided values
/// (`Scene.*`, `Camera3D.*`, …). Unlike project files they are not read from the
/// directory and are EXEMPT from the protected-namespace check: they are
/// precisely what defines those namespaces. This is the host's seam to give
/// the checker real types for its externals (see `docs/functor-lang-interfaces.md`); plain `functor_lang`
/// callers pass nothing, so the language stays host-agnostic.
pub fn load_with_prelude(
    entry_path: &Path,
    overrides: &HashMap<PathBuf, String>,
    prelude: &[(String, String)],
) -> Result<Project, ProjectError> {
    let bundled = prelude_modules(prelude);
    load_with_bundled_modules(entry_path, overrides, &bundled)
}

/// [`load_with_overrides`], plus implementation and interface modules bundled
/// by the embedding. Unlike project files they are supplied in memory and are
/// never watched for edits. Their names are reserved automatically, so a user
/// file cannot shadow a bundled module.
pub fn load_with_bundled_modules(
    entry_path: &Path,
    overrides: &HashMap<PathBuf, String>,
    bundled: &[BundledModule],
) -> Result<Project, ProjectError> {
    let at = |path: &Path, message: String| ProjectError {
        path: path.to_path_buf(),
        line: 1,
        col: 1,
        message,
    };

    let (paths, skipped) = scan_project_dir(entry_path).map_err(|e| {
        at(
            entry_path,
            format!("cannot read the project directory: {e}"),
        )
    })?;

    // Tell the user (once, at load — not per frame) about `.fun` siblings we
    // ignored, so a temp file or misnamed module isn't a silent no-load.
    for path in &skipped {
        eprintln!(
            "[functor-lang] ignoring {} — its file stem is not a valid module identifier \
(editor temp file?); rename it to load it as a module",
            file_name(path)
        );
    }

    // The ENTRY is always loaded, so its stem must name a valid module even
    // when it has no siblings — an on-disk one-file project must NOT take the
    // single-source `Main` fallback in `load_sources_with_bundled_modules`, which
    // exists for the inline wasm/docs single-entry case where the path is
    // only a label.
    module_name(entry_path).map_err(|message| at(entry_path, message))?;

    // Read every project file (entry first), honoring the in-memory overrides,
    // then hand the (path, source) pairs to the shared linker.
    let mut sources: Vec<(PathBuf, String)> = Vec::new();
    for path in paths.iter() {
        let src = match override_for(path, overrides) {
            Some(src) => src.clone(),
            None => std::fs::read_to_string(path)
                .map_err(|e| at(path, format!("cannot read {}: {e}", path.display())))?,
        };
        sources.push((path.clone(), src));
    }
    load_sources_with_bundled_modules(sources, bundled)
}

/// Link an already-read set of project sources (entry FIRST, then siblings —
/// each a `(path, source)` pair) plus the injected prelude modules. The
/// in-memory core shared by the on-disk [`load_with_prelude`] and the wasm
/// producer, which fetches each project file over HTTP rather than reading the
/// directory — so native and web run ONE link path (module-name derivation,
/// the protected-namespace guard, span bases, prelude injection). The paths are
/// only labels here (module names + error rendering); nothing is read from disk.
pub fn load_sources_with_prelude(
    sources: Vec<(PathBuf, String)>,
    prelude: &[(String, String)],
) -> Result<Project, ProjectError> {
    let bundled = prelude_modules(prelude);
    load_sources_with_bundled_modules(sources, &bundled)
}

/// Link already-read project sources plus implementation and interface
/// modules supplied by the embedding. This is the shared in-memory seam used
/// by native, wasm, editor tooling, and future stdlib modules.
pub fn load_sources_with_bundled_modules(
    sources: Vec<(PathBuf, String)>,
    bundled: &[BundledModule],
) -> Result<Project, ProjectError> {
    let core = core_modules();
    let at = |path: &Path, message: String| ProjectError {
        path: path.to_path_buf(),
        line: 1,
        col: 1,
        message,
    };

    // Derive and validate module names; assign span bases. A SINGLE-source
    // project (the inline / wasm single-entry case) has no siblings to refer
    // to it by name, so a non-identifier or protected stem falls back to
    // `Main` — same rule as `load_single_file`. On-disk multi-file projects
    // keep the loud errors: there, siblings reference modules by name.
    let single = sources.len() == 1;
    let mut files: Vec<SourceFile> = Vec::new();
    let mut base = 0usize;
    for (path, src) in sources.iter() {
        let module = if single {
            single_module_name(path, bundled, &core)
        } else {
            module_name(path).map_err(|message| at(path, message))?
        };
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
        // Include core dynamically so future stdlib modules are reserved even
        // before they are added to the host-oriented protected-name list.
        if bundled.iter().chain(&core).any(|item| item.name == module) {
            return Err(at(
                path,
                format!(
                    "module name `{module}` (from {}) collides with the bundled module \
namespace `{module}` — remove the file to use the bundled module, or rename it \
if it is a customized module",
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
        let len = src.len();
        files.push(SourceFile {
            interface: is_interface(path),
            path: path.clone(),
            module,
            src: src.clone(),
            base,
        });
        // +1 gap: a span at one file's very end never collides with the
        // next file's base.
        base += len + 1;
    }
    push_bundled(&mut files, bundled)?;
    push_bundled(&mut files, &core)?;
    link(files)
}

/// Append bundled modules to `files`, assigning span bases past the last one.
/// They are exempt from the project-file protected-namespace check because
/// they own their namespaces. Duplicate descriptors still fail loudly.
fn push_bundled(
    files: &mut Vec<SourceFile>,
    bundled: &[BundledModule],
) -> Result<(), ProjectError> {
    let mut base = files.last().map_or(0, |f| f.base + f.src.len() + 1);
    for item in bundled {
        let path = item.path();
        if let Some(previous) = files.iter().find(|file| file.module == item.name) {
            return Err(ProjectError {
                path,
                line: 1,
                col: 1,
                message: format!(
                    "bundled module `{}` is already supplied by {}",
                    item.name,
                    previous.path.display()
                ),
            });
        }
        files.push(SourceFile {
            interface: item.kind == BundledModuleKind::Interface,
            path,
            module: item.name.clone(),
            src: item.src.clone(),
            base,
        });
        base += item.src.len() + 1;
    }
    Ok(())
}

/// Parse, lower, and link a complete set of project and bundled source files
/// into one merged module.
fn link(files: Vec<SourceFile>) -> Result<Project, ProjectError> {
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
        let parse = if file.interface {
            parse_interface_with_base
        } else {
            parse_with_base
        };
        let program = parse(&file.src, file.base)
            .map_err(|e| render_span(&files, index, e.span, &e.message))?;
        programs.push(program);
    }

    // Every module's exports, for cross-module resolution during lowering.
    // Inline `module` blocks are keyed by their full path (`Utils.Grid`);
    // a file's own top-level exports never include them.
    let mut exports: HashMap<String, Exports> = HashMap::new();
    let mut inline_modules: Vec<InlineModule> = Vec::new();
    let file_modules: HashSet<&str> = files.iter().map(|f| f.module.as_str()).collect();
    for (index, (file, program)) in files.iter().zip(&programs).enumerate() {
        exports.insert(file.module.clone(), exports_of(&program.items));
        for decl in program.items.iter().filter_map(|item| match item {
            ast::Item::Module(decl) => Some(decl),
            _ => None,
        }) {
            let (name, span) = (decl.name.as_str(), decl.span);
            // An inline module is reachable UNQUALIFIED inside its own file
            // (`Server.step`), and resolves there before any sibling, so its
            // bare name must not shadow a builtin/bundled namespace or
            // another file's module — `module Scene` would silently steal
            // every `Scene.cube` in the declaring file. (Two different files
            // may each declare `module Grid`: those canonicalize apart, as
            // `Utils.Grid` / `Helpers.Grid`, and neither shadows the other.)
            if PROTECTED_NAMESPACES.contains(&name) {
                return Err(render_span(
                    &files,
                    index,
                    span,
                    &format!(
                        "`module {name}` collides with the built-in `{name}` namespace — \
rename it"
                    ),
                ));
            }
            if let Some(other) = file_modules.get(name) {
                let other = files
                    .iter()
                    .find(|f| f.module == *other)
                    .expect("module name came from files");
                return Err(render_span(
                    &files,
                    index,
                    span,
                    &format!(
                        "`module {name}` collides with the module of {} — rename one",
                        other.path.display()
                    ),
                ));
            }
            exports.insert(format!("{}.{name}", file.module), exports_of(&decl.items));
            inline_modules.push(InlineModule {
                name: decl.name.clone(),
                file: file.module.clone(),
                // The entry file's members canonicalize BARE (`lower`'s
                // rule), so its inline modules keep only their own segment.
                path: if file.module == entry {
                    decl.name.clone()
                } else {
                    format!("{}.{name}", file.module)
                },
                span: decl.block,
            });
        }
    }
    let exports = exports;

    // Units are PROJECT-WIDE (file = module makes them like constructors), so
    // resolve every module's `unit` declarations — each in its own scope —
    // before any file lowers: a suffixed literal may precede, or live in a
    // different file from, its declaration. A suffix may be declared once.
    // Duplicate suffixes are caught FIRST, on names alone: a redeclaration
    // should report as a duplicate even when the second declaration's target
    // is also broken.
    let mut declared_in: HashMap<&str, usize> = HashMap::new();
    for (index, program) in programs.iter().enumerate() {
        for decl in unit_decls(&program.items) {
            if let Some(&previous) = declared_in.get(decl.suffix.as_str()) {
                return Err(render_span(
                    &files,
                    index,
                    decl.span,
                    &format!(
                        "duplicate unit `{}` — it is already declared in {} (units are \
project-wide, like constructors)",
                        decl.suffix,
                        files[previous].path.display()
                    ),
                ));
            }
            declared_in.insert(&decl.suffix, index);
        }
    }
    let mut units: HashMap<String, UnitTarget> = HashMap::new();
    for (index, (file, program)) in files.iter().zip(&programs).enumerate() {
        let env = ProjectEnv {
            name: &file.module,
            entry: &entry,
            modules: &exports,
            units: &HashMap::new(),
        };
        let resolved = resolve_units(&program.items, Some(&env))
            .map_err(|e| render_span(&files, index, e.span, &e.message))?;
        for (suffix, target, _) in resolved {
            units.insert(suffix, target);
        }
    }

    // Lower each file with the project environment, threading ID bases so
    // the merged module is one ID space. Collect dependency edges.
    let mut bases = IdBases::default();
    let mut lowered: Vec<Module> = Vec::new();
    let mut deps: HashMap<String, HashSet<String>> = HashMap::new();
    let mut scopes = RecordLiteralScopes::default();
    for (index, (file, program)) in files.iter().zip(programs).enumerate() {
        // Record-literal visibility for this module: its own types plus its
        // `open`ed modules' (by canonical name — the entry's are bare).
        // Mirrors `lower::Lowerer::qualify`: the entry file's members stay
        // bare, so its inline modules keep only their own segment.
        let canon = |module: &str, name: &str| {
            if module == entry {
                name.to_string()
            } else if let Some(inline) = module
                .strip_prefix(entry.as_str())
                .and_then(|rest| rest.strip_prefix('.'))
            {
                format!("{inline}.{name}")
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
            // `open Server` may name one of this file's inline modules.
            let path = open_module_path(&file.module, &decl.module, &exports);
            if let Some(opened) = exports.get(&path) {
                visible.extend(opened.types.iter().map(|name| canon(&path, name)));
            }
        }
        let prefix = if file.module == entry {
            String::new()
        } else {
            file.module.clone()
        };
        // An inline module sees its own record types PLUS the file's (and
        // the file's `open`ed ones) — the lowering scope rule.
        for item in &program.items {
            let ast::Item::Module(decl) = item else {
                continue;
            };
            let path = format!("{}.{}", file.module, decl.name);
            let own = &exports[&path].types;
            // The module's own types SHADOW same-named enclosing ones (the
            // lowering rule), so drop the shadowed candidates instead of
            // leaving a bare literal ambiguous between `Cell` and
            // `Grid.Cell`.
            let mut inner: HashSet<String> = visible
                .iter()
                .filter(|name| {
                    let last = name.rsplit('.').next().unwrap_or(name);
                    !own.contains(last)
                })
                .cloned()
                .collect();
            inner.extend(own.iter().map(|name| canon(&path, name)));
            let inner_prefix = if prefix.is_empty() {
                decl.name.clone()
            } else {
                format!("{prefix}.{}", decl.name)
            };
            scopes.by_module.insert(inner_prefix, inner);
        }
        scopes.by_module.insert(prefix, visible);

        let env = ProjectEnv {
            name: &file.module,
            entry: &entry,
            modules: &exports,
            units: &units,
        };
        let (module, next, module_deps) = lower_in_project(program, &env, bases)
            .map_err(|e| render_span(&files, index, e.span, &e.message))?;
        bases = next;
        lowered.push(module);
        // An interface module has no runtime initializers, so its out-edges
        // are meaningless for evaluation order and cannot form a real cycle.
        // Dropping them lets a signature reference a consumer's type
        // (`render : (Game.Model) => …`) without a spurious cycle error.
        let module_deps = if file.interface {
            HashSet::new()
        } else {
            module_deps
        };
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
            .unwrap_or_else(|| files[0].path.clone());
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
        signatures: Vec::new(),
        expects: Vec::new(),
        units: Vec::new(),
        unit_ops: Vec::new(),
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
        merged.signatures.extend(module.signatures);
        merged.expects.extend(module.expects);
        merged.units.extend(module.units);
        merged.unit_ops.extend(module.unit_ops);
    }

    Ok(Project {
        module: merged,
        sources: SourceMap { files },
        entry,
        inline_modules,
        scopes,
    })
}

/// Load a project from ONE in-memory entry source (no filesystem) — the
/// wasm producer's path, where the game is fetched as a single text and
/// there are no sibling files. The core bundled modules are still injected.
pub fn load_single_source(module: &str, src: &str) -> Result<Project, ProjectError> {
    let mut files = vec![SourceFile {
        interface: false,
        path: PathBuf::from(format!("{module}.fun")),
        module: module.to_string(),
        src: src.to_string(),
        base: 0,
    }];
    push_bundled(&mut files, &core_modules())?;
    link(files)
}

/// Load ONE in-memory file as a single-file project, keeping its real path in
/// the [`SourceMap`] (so [`SourceMap::file_by_path`] resolves it) — the LSP's
/// path for a `.fun` with no `functor.json`: check just this buffer, never
/// scanning the directory for unrelated siblings. Module name is the file's
/// (`Main` if the stem isn't an identifier); a single file is its own bare
/// entry, so the name only labels it.
pub fn load_single_file(
    path: &Path,
    src: &str,
    prelude: &[(String, String)],
) -> Result<Project, ProjectError> {
    let bundled = prelude_modules(prelude);
    load_single_file_with_bundled_modules(path, src, &bundled)
}

/// Load one in-memory file plus implementation and interface modules supplied
/// by the embedding. The synthetic modules are available for checking,
/// completion, navigation, and evaluation but are not project files.
pub fn load_single_file_with_bundled_modules(
    path: &Path,
    src: &str,
    bundled: &[BundledModule],
) -> Result<Project, ProjectError> {
    let core = core_modules();
    let module = single_module_name(path, bundled, &core);
    let mut files = vec![SourceFile {
        interface: is_interface(path),
        path: path.to_path_buf(),
        module,
        src: src.to_string(),
        base: 0,
    }];
    push_bundled(&mut files, bundled)?;
    push_bundled(&mut files, &core)?;
    link(files)
}

/// The module name for a SINGLE-source project (one entry, no siblings): the
/// file's derived name, or `Main` when the stem isn't an identifier OR
/// capitalizes to a protected or bundled namespace (`net.fun` → `Net`), which
/// would otherwise collide with an injected module and fail the load. A single
/// file is its own bare entry, so the name only labels it — nothing references
/// it by name.
fn single_module_name(path: &Path, bundled: &[BundledModule], core: &[BundledModule]) -> String {
    match module_name(path) {
        Ok(name)
            if !PROTECTED_NAMESPACES.contains(&name.as_str())
                && !bundled.iter().chain(core).any(|item| item.name == name) =>
        {
            name
        }
        _ => "Main".to_string(),
    }
}

/// The module name a file provides: its stem, first letter capitalized
/// (`utils.fun` → `Utils`). The stem must be a valid identifier — module
/// names appear in source (`Utils.clamp`).
fn module_name(path: &Path) -> Result<String, String> {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    if !is_module_stem(stem) {
        return Err(format!(
            "cannot derive a module name from `{}` — file stems must be identifiers \
(letters, digits, `_`; starting with a letter), like `utils.fun` → `Utils`",
            file_name(path)
        ));
    }
    Ok(capitalize(stem))
}

/// The override source for `path`, if any: exact-path match first, then a
/// canonicalized match (the same fail-soft matching as
/// [`SourceMap::file_by_path`]).
fn override_for<'a>(path: &Path, overrides: &'a HashMap<PathBuf, String>) -> Option<&'a String> {
    if overrides.is_empty() {
        return None;
    }
    if let Some(src) = overrides.get(path) {
        return Some(src);
    }
    let canon = std::fs::canonicalize(path).ok()?;
    overrides
        .iter()
        .find(|(key, _)| std::fs::canonicalize(key).ok() == Some(canon.clone()))
        .map(|(_, src)| src)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_stem_accepts_identifiers_and_rejects_the_rest() {
        // Valid module identifiers.
        assert!(is_module_stem("game"));
        assert!(is_module_stem("Utils"));
        assert!(is_module_stem("enemy_ai"));
        assert!(is_module_stem("a1"));

        // Rejected: empty, non-letter start, and non-identifier characters —
        // the exact stems that would otherwise fail `module_name` (or, for a
        // dot-prefixed stem, name an editor temp file).
        assert!(!is_module_stem(""));
        assert!(!is_module_stem("2d")); // starts with a digit
        assert!(!is_module_stem(".#game")); // emacs lock file stem
        assert!(!is_module_stem("#game")); // starts with `#`
        assert!(!is_module_stem("my-game")); // contains `-`
        assert!(!is_module_stem("game.old")); // contains `.`
    }

    #[test]
    fn loadable_stem_reads_the_file_stem() {
        // `.fun` files whose stem is / isn't a valid module identifier.
        assert!(is_loadable_stem(Path::new("game.fun")));
        assert!(is_loadable_stem(Path::new("dir/utils.funi")));
        assert!(!is_loadable_stem(Path::new(".#game.fun"))); // stem `.#game`
        assert!(!is_loadable_stem(Path::new("2d.fun"))); // stem `2d`
    }
}
