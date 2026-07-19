//! `build wasm`: write the project as a self-contained static web bundle in
//! `dist/web/` — exactly what the wasm dev server serves (the rendered
//! Functor Lang index page, the embedded runtime `pkg/`, and the project's
//! files), written to disk instead of served. The invariant: anything that
//! runs under `run wasm` runs exported with the same file set — except
//! hidden entries and the reserved output names, which the exporter errors
//! on (modules) or reports (everything else) rather than shipping a
//! silently broken bundle. The folder works on any static host: zip it for
//! itch.io (HTML5), push it to GitHub Pages, `python -m http.server` it.
//!
//! The whole project directory is copied (minus hidden entries and the
//! reserved output names below) rather than a statically discovered file
//! set: asset locators are runtime strings inside Asset values
//! (`Scene.model(Asset.model("shark.glb"))`, `Texture.file(…)`, or
//! computed), and a non-embedded `.gltf` references external `.bin`/texture
//! files from *inside* the glTF — no source scan can see either, and a
//! missing asset degrades to the invisible fallback, the worst failure mode
//! for a published game. Copy-everything is the only rule that keeps the
//! run-wasm ↔ export invariant. A best-effort lint (below) still catches the
//! common case of literal-referenced assets that were never fetched.

use std::collections::BTreeSet;
use std::io::Error;
use std::path::{Path, PathBuf};

use super::wasm_dev_server::{
    project_file_urls, render_functor_lang_index, JS_FILE_1, SCRUBBER_JS, TIMELINE_MODEL_JS,
    WASM_FILE,
};

/// Names at the project root the exporter owns in the bundle: its output dir
/// and the runtime files it writes (the index page, the `pkg/` wasm bundle,
/// and the shared `scrubber.js` component). Project files with these names are
/// skipped (and reported via [`WasmExport::shadowed`]) rather than merged —
/// merging a project `pkg/` with the runtime's would leave stray files
/// beside the wasm bundle.
const RESERVED_ROOT: &[&str] = &[
    "dist",
    "index.html",
    "pkg",
    "scrubber.js",
    "timeline-model.js",
];

/// Extensions the runtime fetches at runtime: models (plus the external
/// buffers/images a non-embedded `.gltf` references), audio, textures.
const ASSET_EXTENSIONS: &[&str] = &[
    "glb", "gltf", "bin", "wav", "ogg", "mp3", "png", "jpg", "jpeg", "hdr",
];

#[derive(Debug)]
pub struct WasmExport {
    /// The bundle directory: `<project>/dist/web`.
    pub out_dir: PathBuf,
    /// Project files copied into the bundle (excludes the runtime files).
    pub file_count: usize,
    /// Total bytes of those project files.
    pub project_bytes: u64,
    /// Bytes of the embedded runtime files written alongside them.
    pub runtime_bytes: u64,
    /// String-literal asset references that will NOT be in the bundle
    /// (missing from the project dir, or absolute/`..`/hidden paths).
    pub missing_assets: Vec<String>,
    /// Root-level project entries skipped because their names are reserved.
    pub shadowed: Vec<String>,
    /// Symlinked directories (or broken links) skipped by the copy.
    pub skipped_symlinks: Vec<String>,
}

/// Export the project as a static web bundle. `dist/web` is wiped first so a
/// file deleted from the project can't linger in the bundle.
pub fn export_functor_lang_wasm(working_directory: &str, entry: &str) -> Result<WasmExport, Error> {
    let root = Path::new(working_directory);

    // Every module baked into the index's file list must be fetchable from
    // the bundle — a listed-but-uncopied module 404s at load time, a broken
    // bundle. Validate BEFORE the destructive wipe below, and fail loud.
    let files = project_file_urls(working_directory, entry);
    let unbundleable: Vec<&str> = files
        .iter()
        .filter(|f| !in_bundle(root, f))
        .map(|f| f.as_str())
        .collect();
    if !unbundleable.is_empty() {
        return Err(Error::other(format!(
            "module(s) that can't ship in the bundle (hidden path segment, or a reserved \
name {RESERVED_ROOT:?} at the project root): {} — rename or move them",
            unbundleable.join(", ")
        )));
    }

    let out = root.join("dist").join("web");
    // Refuse to wipe through a symlink: `remove_dir_all` follows an
    // intermediate `dist` symlink and would delete the link TARGET's
    // contents — potentially outside the project.
    for link in [root.join("dist"), out.clone()] {
        let is_symlink = link
            .symlink_metadata()
            .is_ok_and(|m| m.file_type().is_symlink());
        if is_symlink {
            return Err(Error::other(format!(
                "{} is a symlink — refusing to wipe and export through it",
                link.display()
            )));
        }
    }
    if out.exists() {
        std::fs::remove_dir_all(&out)?;
    }

    let shadowed: Vec<String> = RESERVED_ROOT
        .iter()
        .filter(|name| root.join(name).exists() && **name != "dist")
        .map(|name| name.to_string())
        .collect();

    let mut stats = CopyStats::default();
    copy_project(root, &out, true, &mut stats)?;

    // The runtime files go in last so nothing in the project can shadow them.
    std::fs::write(out.join("index.html"), render_functor_lang_index(entry, &files))?;
    std::fs::write(out.join("scrubber.js"), SCRUBBER_JS)?;
    std::fs::write(out.join("timeline-model.js"), TIMELINE_MODEL_JS)?;
    let pkg = out.join("pkg");
    std::fs::create_dir_all(&pkg)?;
    std::fs::write(pkg.join("functor_runtime_web.js"), JS_FILE_1)?;
    std::fs::write(pkg.join("functor_runtime_web_bg.wasm"), WASM_FILE)?;

    Ok(WasmExport {
        out_dir: out,
        file_count: stats.files,
        project_bytes: stats.bytes,
        runtime_bytes: (JS_FILE_1.len()
            + WASM_FILE.len()
            + SCRUBBER_JS.len()
            + TIMELINE_MODEL_JS.len()) as u64,
        missing_assets: missing_asset_references(root, entry),
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
fn copy_project(src: &Path, dst: &Path, is_root: bool, stats: &mut CopyStats) -> Result<(), Error> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with('.') {
            continue;
        }
        if is_root && RESERVED_ROOT.contains(&name_str.as_ref()) {
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
            copy_project(&from, &to, false, stats)?;
        } else {
            stats.bytes += std::fs::copy(&from, &to)?;
            stats.files += 1;
        }
    }
    Ok(())
}

/// Best-effort missing-asset lint: every string literal in the project's
/// `.fun`/`.funi` sources that looks like an asset path should resolve to a
/// file the bundle carries (wasm fetches asset paths as URLs relative to the
/// page, i.e. relative to the project dir). Absolute or `..` paths work
/// natively but can never be in the bundle, so they're flagged even when the
/// file exists. Computed paths (`"fish" ++ ".glb"`) are invisible to this
/// scan — hence warn-only, never a gate — but it catches the common case:
/// gitignored models that were never fetched, producing a bundle that
/// silently renders fallbacks.
fn missing_asset_references(root: &Path, entry: &str) -> Vec<String> {
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
                if is_asset_path(&s) && !in_bundle(root, &s) {
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

/// Will the path `s` be present in the exported bundle at the URL the
/// runtime fetches? Mirrors the copy rules: relative, inside the project,
/// no hidden segments, not under a reserved root name.
fn in_bundle(root: &Path, s: &str) -> bool {
    let path = Path::new(s);
    if path.is_absolute() {
        return false;
    }
    let mut segments = s.split(['/', '\\']);
    if segments
        .next()
        .is_some_and(|first| RESERVED_ROOT.contains(&first))
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU32, Ordering};

    static NEXT_DIR: AtomicU32 = AtomicU32::new(0);

    /// A unique temp project dir, removed on drop (same pattern as the
    /// `init` command's tests).
    struct TestDir(PathBuf);

    impl TestDir {
        fn new(name: &str) -> Self {
            let suffix = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "functor-export-{name}-{}-{suffix}",
                std::process::id()
            ));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn write(&self, rel: &str, content: &str) {
            let path = self.0.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, content).unwrap();
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn exports_the_dev_server_file_set() {
        let dir = TestDir::new("fileset");
        dir.write("game.fun", "let init = 0");
        dir.write("pieces.fun", "let helper = 1");
        dir.write("model.glb", "glb-bytes");
        dir.write("tex/wall.png", "png-bytes");
        dir.write(".DS_Store", "junk");
        dir.write("dist/web/stale.txt", "from a previous export");

        let wd = dir.path().to_string_lossy().to_string();
        let export = export_functor_lang_wasm(&wd, "game.fun").unwrap();

        let out = &export.out_dir;
        let index = fs::read_to_string(out.join("index.html")).unwrap();
        assert!(index.contains("window.__functorLangGamePath = \"game.fun\""));
        assert!(index.contains("\"pieces.fun\""), "sibling module in the file list");
        assert!(!fs::read(out.join("pkg/functor_runtime_web_bg.wasm")).unwrap().is_empty());
        assert!(!fs::read(out.join("pkg/functor_runtime_web.js")).unwrap().is_empty());
        assert!(
            fs::read_to_string(out.join("scrubber.js")).unwrap().contains("mountScrubber"),
            "the shared scrubber component ships with the bundle"
        );
        assert!(
            fs::read_to_string(out.join("timeline-model.js"))
                .unwrap()
                .contains("deriveTimelineView"),
            "the scrubber's functional core ships with the bundle"
        );
        assert!(out.join("game.fun").is_file());
        assert!(out.join("pieces.fun").is_file());
        assert!(out.join("model.glb").is_file());
        assert!(out.join("tex/wall.png").is_file());
        assert!(!out.join(".DS_Store").exists(), "hidden files are excluded");
        assert!(!out.join("stale.txt").exists(), "stale output is wiped, not carried over");
        assert!(!out.join("dist").exists(), "the output dir is never copied into itself");
        // game.fun + pieces.fun + model.glb + tex/wall.png
        assert_eq!(export.file_count, 4);
        assert!(export.shadowed.is_empty());
    }

    #[test]
    fn reserved_root_names_are_skipped_and_reported() {
        let dir = TestDir::new("reserved");
        dir.write("game.fun", "let init = 0");
        dir.write("index.html", "<html>the project's own page</html>");
        dir.write("pkg/junk.js", "not the runtime");

        let wd = dir.path().to_string_lossy().to_string();
        let export = export_functor_lang_wasm(&wd, "game.fun").unwrap();

        let out = &export.out_dir;
        let index = fs::read_to_string(out.join("index.html")).unwrap();
        assert!(index.contains("__functorLangGamePath"), "the bundle's index wins");
        assert!(!out.join("pkg/junk.js").exists(), "no merge into the runtime pkg/");
        assert_eq!(export.shadowed, vec!["index.html", "pkg"]);
    }

    #[test]
    fn lint_flags_missing_literal_assets_only() {
        let dir = TestDir::new("lint");
        dir.write("present.wav", "riff");
        dir.write("pkg/tex.png", "exists but reserved — the copy skips it");
        dir.write(
            "game.fun",
            r#"// a comment mentioning ghost.glb is not a reference
let init = 0
let a = "missing.glb"
let b = "present.wav"
let c = "../outside.png"
let d = "/abs/path.jpg"
let e = "notes.txt"
let f = "https://cdn.example/remote.png"
let g = "pkg/tex.png"
"#,
        );

        let missing = missing_asset_references(dir.path(), "game.fun");
        assert_eq!(
            missing,
            vec!["../outside.png", "/abs/path.jpg", "missing.glb", "pkg/tex.png"],
            "missing + unbundleable flagged; present / non-asset / comments / URLs not"
        );
    }

    #[test]
    fn hidden_sibling_modules_are_skipped_not_bundled() {
        let dir = TestDir::new("hidden-module");
        dir.write("game.fun", "let init = 0");
        // A dot-prefixed sibling (an editor temp file like `.#game.fun`) is
        // filtered by project loading (functor_lang::project skips non-loadable
        // stems), so it's neither listed in the index nor copied. The export
        // succeeds with only the entry — the hidden file never reaches it.
        dir.write(".hidden.fun", "let ghost = 1");

        let wd = dir.path().to_string_lossy().to_string();
        let export = export_functor_lang_wasm(&wd, "game.fun").unwrap();
        assert_eq!(export.file_count, 1, "only game.fun ships");
        assert!(!export.out_dir.join(".hidden.fun").exists(), "hidden sibling not bundled");
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_output_is_refused() {
        let dir = TestDir::new("symlink-out");
        dir.write("game.fun", "let init = 0");
        dir.write("elsewhere/precious.txt", "do not delete");
        std::os::unix::fs::symlink(dir.path().join("elsewhere"), dir.path().join("dist")).unwrap();

        let wd = dir.path().to_string_lossy().to_string();
        let err = export_functor_lang_wasm(&wd, "game.fun").unwrap_err();
        assert!(err.to_string().contains("symlink"), "{err}");
        assert!(
            dir.path().join("elsewhere/precious.txt").is_file(),
            "the link target was not wiped"
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_files_copy_but_symlinked_dirs_are_skipped() {
        let dir = TestDir::new("symlink-copy");
        dir.write("game.fun", "let init = 0");
        dir.write("shared/model.glb", "glb-bytes");
        std::os::unix::fs::symlink(
            dir.path().join("shared/model.glb"),
            dir.path().join("linked.glb"),
        )
        .unwrap();
        // A directory symlink to an ancestor — following it would recurse.
        std::os::unix::fs::symlink(dir.path(), dir.path().join("loop")).unwrap();

        let wd = dir.path().to_string_lossy().to_string();
        let export = export_functor_lang_wasm(&wd, "game.fun").unwrap();
        assert_eq!(
            fs::read(export.out_dir.join("linked.glb")).unwrap(),
            b"glb-bytes",
            "a symlinked file copies through"
        );
        assert!(!export.out_dir.join("loop").exists());
        assert_eq!(export.skipped_symlinks.len(), 1);
        assert!(export.skipped_symlinks[0].ends_with("loop"));
    }
}
