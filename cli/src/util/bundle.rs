//! The shared bundle-staging machinery behind `build wasm` and
//! `build native`: both exports are "the project directory, copied under
//! `dist/`, plus a runtime" — this module owns the copy and its rules, the
//! pre-wipe validation, and the missing-asset lint, so the two targets can
//! never drift on what a bundle contains.
//!
//! The whole project directory is copied (minus hidden entries and the
//! reserved output names below) rather than a statically discovered file
//! set: asset paths are runtime strings (`Scene.model("shark.glb")`, or
//! computed), and a non-embedded `.gltf` references external `.bin`/texture
//! files from *inside* the glTF — no source scan can see either, and a
//! missing asset degrades to the invisible fallback, the worst failure mode
//! for a published game. Copy-everything is the only rule that keeps the
//! run ↔ export invariant. A best-effort lint (below) still catches the
//! common case of literal-referenced assets that were never fetched.

use std::collections::BTreeSet;
use std::io::Error;
use std::path::Path;

/// Names at the project root the WEB exporter owns in the bundle: the
/// output dir and the runtime files it writes. Project files with these
/// names are skipped (and reported via [`StagedBundle::shadowed`]) rather
/// than merged — merging a project `pkg/` with the runtime's would leave
/// stray files beside the wasm bundle. Reservations are per-target (the
/// native export reserves only `dist` + its own binary name), so a native
/// game with a root `pkg/` asset dir still ships it.
pub const WEB_RESERVED: &[&str] = &["dist", "index.html", "pkg"];

/// Extensions the runtime fetches at runtime: models (plus the external
/// buffers/images a non-embedded `.gltf` references), audio, textures.
const ASSET_EXTENSIONS: &[&str] = &[
    "glb", "gltf", "bin", "wav", "ogg", "mp3", "png", "jpg", "jpeg", "hdr",
];

/// The result of [`stage_bundle`]: the copied project plus everything the
/// caller should report to the user.
#[derive(Debug)]
pub struct StagedBundle {
    /// Project files copied into the bundle (excludes any runtime files the
    /// caller adds afterwards).
    pub file_count: usize,
    /// Total bytes of those project files.
    pub project_bytes: u64,
    /// String-literal asset references that will NOT be in the bundle
    /// (missing from the project dir, or absolute/`..`/hidden/reserved
    /// paths).
    pub missing_assets: Vec<String>,
    /// Root-level project entries skipped because their names are reserved.
    pub shadowed: Vec<String>,
    /// Symlinked directories (or broken links) skipped by the copy.
    pub skipped_symlinks: Vec<String>,
}

/// The project's file list as URLs relative to the project dir (entry
/// first, then sibling `.fun`/`.funi` files) — the set the runners link.
/// Falls back to just the entry if the directory can't be scanned.
pub fn project_file_urls(working_directory: &str, entry: &str) -> Vec<String> {
    let entry_path = Path::new(working_directory).join(entry);
    let paths = match functor_lang::project::project_files(&entry_path) {
        Ok(paths) => paths,
        Err(_) => return vec![entry.to_string()],
    };
    let root = Path::new(working_directory);
    paths
        .iter()
        .map(|p| {
            p.strip_prefix(root)
                .unwrap_or(p)
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect()
}

/// The project's name for artifacts (the bundle binary, the zip): its
/// directory's file name — canonicalized, so `-d .` resolves to a real name.
pub fn project_name(root: &Path) -> String {
    std::fs::canonicalize(root)
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "game".to_string())
}

/// Validate, wipe, and copy the project into `out` (which must live under
/// `<root>/dist`). The shared front half of every bundle export:
///
/// 1. every module in `files` must satisfy the copy rules — a
///    listed-but-uncopied module is a broken bundle, so this fails loud
///    BEFORE the destructive wipe;
/// 2. refuse to wipe through a symlink anywhere on `dist/…/out` —
///    `remove_dir_all` follows intermediate symlinks and would delete the
///    link TARGET's contents, potentially outside the project;
/// 3. wipe `out` so a file deleted from the project can't linger, then
///    copy the project in.
pub fn stage_bundle(
    root: &Path,
    out: &Path,
    entry: &str,
    files: &[String],
    reserved: &[&str],
) -> Result<StagedBundle, Error> {
    let unbundleable: Vec<&str> = files
        .iter()
        .filter(|f| !in_bundle(root, f, reserved))
        .map(|f| f.as_str())
        .collect();
    if !unbundleable.is_empty() {
        return Err(Error::other(format!(
            "module(s) that can't ship in the bundle (hidden path segment, or a reserved \
name {reserved:?} at the project root): {} — rename or move them",
            unbundleable.join(", ")
        )));
    }

    // Walk dist → out checking each component (dist and out included).
    // (`dist/.`-style joins would resolve THROUGH a symlink, hiding it, so
    // each path is checked as-is.)
    let dist = root.join("dist");
    let below_dist = out
        .strip_prefix(&dist)
        .map_err(|_| Error::other("bundle output must live under the project's dist/"))?
        .to_path_buf();
    let mut guard = dist.clone();
    let mut to_check = vec![dist];
    for component in below_dist.iter() {
        guard = guard.join(component);
        to_check.push(guard.clone());
    }
    for path in to_check {
        let is_symlink = path
            .symlink_metadata()
            .is_ok_and(|m| m.file_type().is_symlink());
        if is_symlink {
            return Err(Error::other(format!(
                "{} is a symlink — refusing to wipe and export through it",
                path.display()
            )));
        }
    }
    if out.exists() {
        std::fs::remove_dir_all(out)?;
    }

    let shadowed: Vec<String> = reserved
        .iter()
        .filter(|name| root.join(name).exists() && **name != "dist")
        .map(|name| name.to_string())
        .collect();

    let mut stats = CopyStats::default();
    copy_project(root, out, reserved, true, &mut stats)?;

    Ok(StagedBundle {
        file_count: stats.files,
        project_bytes: stats.bytes,
        missing_assets: missing_asset_references(root, entry, reserved),
        shadowed,
        skipped_symlinks: stats.skipped_symlinks,
    })
}

#[derive(Default)]
struct CopyStats {
    files: usize,
    bytes: u64,
    skipped_symlinks: Vec<String>,
}

/// Recursive project copy: skips hidden entries everywhere (`.git`,
/// `.DS_Store`) and the reserved names at the root only. Symlinked FILES
/// copy through (`fs::copy` reads the target — a linked shared asset is a
/// real workflow), but symlinked DIRECTORIES are skipped and reported:
/// following one can recurse forever (a link to an ancestor) or vacuum an
/// external tree into the bundle.
fn copy_project(
    src: &Path,
    dst: &Path,
    reserved: &[&str],
    is_root: bool,
    stats: &mut CopyStats,
) -> Result<(), Error> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with('.') {
            continue;
        }
        if is_root && reserved.contains(&name_str.as_ref()) {
            continue;
        }
        let from = entry.path();
        let to = dst.join(&name);
        // no-follow, so symlinks are decided here and never recursed into
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            match std::fs::metadata(&from) {
                Ok(m) if m.is_file() => {
                    stats.bytes += std::fs::copy(&from, &to)?;
                    stats.files += 1;
                }
                // A symlinked dir or a broken link: skip + report.
                _ => stats.skipped_symlinks.push(from.display().to_string()),
            }
        } else if file_type.is_dir() {
            copy_project(&from, &to, reserved, false, stats)?;
        } else {
            stats.bytes += std::fs::copy(&from, &to)?;
            stats.files += 1;
        }
    }
    Ok(())
}

/// Best-effort missing-asset lint: every string literal in the project's
/// `.fun`/`.funi` sources that looks like an asset path should resolve to a
/// file the bundle carries (both runners resolve asset paths relative to
/// the project dir). Absolute or `..` paths may work in a dev checkout but
/// can never be in the bundle, so they're flagged even when the file
/// exists. Computed paths (`"fish" ++ ".glb"`) are invisible to this scan —
/// hence warn-only, never a gate — but it catches the common case:
/// gitignored models that were never fetched, producing a bundle that
/// silently renders fallbacks.
pub fn missing_asset_references(root: &Path, entry: &str, reserved: &[&str]) -> Vec<String> {
    let Ok(files) = functor_lang::project::project_files(&root.join(entry)) else {
        return Vec::new();
    };
    let mut missing = BTreeSet::new();
    for path in files {
        let Ok(src) = std::fs::read_to_string(&path) else {
            continue;
        };
        // A lex failure just skips the file: the build's typecheck gate
        // already ran, so this only happens for sources it also rejected.
        let Ok(tokens) = functor_lang::lexer::lex(&src, 0) else {
            continue;
        };
        for token in tokens {
            if let functor_lang::lexer::TokenKind::Str(s) = token.kind {
                if is_asset_path(&s) && !in_bundle(root, &s, reserved) {
                    missing.insert(s);
                }
            }
        }
    }
    missing.into_iter().collect()
}

fn is_asset_path(s: &str) -> bool {
    // A URL ("https://cdn/x.png") is fetched remotely, not from the bundle.
    !s.contains("://")
        && Path::new(s)
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| ASSET_EXTENSIONS.iter().any(|a| ext.eq_ignore_ascii_case(a)))
}

/// Will the path `s` be present in the exported bundle at the path the
/// runtime resolves? Mirrors the copy rules: relative, inside the project,
/// no hidden segments, not under a reserved root name.
fn in_bundle(root: &Path, s: &str, reserved: &[&str]) -> bool {
    let path = Path::new(s);
    if path.is_absolute() {
        return false;
    }
    if s.split(['/', '\\'])
        .next()
        .is_some_and(|first| reserved.contains(&first))
    {
        return false;
    }
    // A `.`-prefixed segment is excluded by the copy's hidden-file rule,
    // and `..` (also caught here) escapes the project.
    if s.split(['/', '\\'])
        .any(|seg| seg != "." && seg.starts_with('.'))
    {
        return false;
    }
    root.join(path).is_file()
}
