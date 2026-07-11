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
//! The staging rules (copy-everything, reserved names, symlink handling,
//! the missing-asset lint) live in `util::bundle`, shared with the native
//! export.

use std::io::Error;
use std::path::{Path, PathBuf};

use super::bundle::{project_file_urls, stage_bundle, StagedBundle, WEB_RESERVED};
use super::wasm_dev_server::{render_functor_lang_index, JS_FILE_1, WASM_FILE};

#[derive(Debug)]
pub struct WasmExport {
    /// The bundle directory: `<project>/dist/web`.
    pub out_dir: PathBuf,
    /// Bytes of the embedded runtime files written alongside the project.
    pub runtime_bytes: u64,
    pub staged: StagedBundle,
}

/// Export the project as a static web bundle (see `util::bundle` for the
/// staging rules).
pub fn export_functor_lang_wasm(working_directory: &str, entry: &str) -> Result<WasmExport, Error> {
    let root = Path::new(working_directory);
    let out = root.join("dist").join("web");

    let files = project_file_urls(working_directory, entry);
    let staged = stage_bundle(root, &out, entry, &files, WEB_RESERVED)?;

    // The runtime files go in last so nothing in the project can shadow them.
    std::fs::write(out.join("index.html"), render_functor_lang_index(entry, &files))?;
    let pkg = out.join("pkg");
    std::fs::create_dir_all(&pkg)?;
    std::fs::write(pkg.join("functor_runtime_web.js"), JS_FILE_1)?;
    std::fs::write(pkg.join("functor_runtime_web_bg.wasm"), WASM_FILE)?;

    Ok(WasmExport {
        out_dir: out,
        runtime_bytes: (JS_FILE_1.len() + WASM_FILE.len()) as u64,
        staged,
    })
}

/// Zip the exported bundle's CONTENTS into `zip_path` — `index.html` sits at
/// the zip ROOT, which is how itch.io wants an HTML5 game archive. Entries
/// are sorted so the archive is deterministic for identical inputs.
pub fn zip_bundle(bundle_dir: &Path, zip_path: &Path) -> Result<u64, Error> {
    let mut paths = Vec::new();
    collect_files(bundle_dir, &mut paths)?;
    paths.sort();

    if let Some(parent) = zip_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = std::fs::File::create(zip_path)?;
    let mut writer = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        // Pin the entry timestamp (the zip epoch, 1980-01-01): the default
        // becomes wall-clock time if zip's `time` feature is ever enabled
        // (feature unification could do it transitively), which would bake
        // the build time into every entry and break determinism.
        .last_modified_time(zip::DateTime::default());
    for path in &paths {
        let rel = path.strip_prefix(bundle_dir).map_err(Error::other)?;
        let name = rel.to_string_lossy().replace('\\', "/");
        writer.start_file(name, options).map_err(Error::other)?;
        let mut src = std::fs::File::open(path)?;
        std::io::copy(&mut src, &mut writer)?;
    }
    writer.finish().map_err(Error::other)?;
    Ok(std::fs::metadata(zip_path)?.len())
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), Error> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_files(&path, out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::bundle::{missing_asset_references, WEB_RESERVED};
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
        assert!(out.join("game.fun").is_file());
        assert!(out.join("pieces.fun").is_file());
        assert!(out.join("model.glb").is_file());
        assert!(out.join("tex/wall.png").is_file());
        assert!(!out.join(".DS_Store").exists(), "hidden files are excluded");
        assert!(!out.join("stale.txt").exists(), "stale output is wiped, not carried over");
        assert!(!out.join("dist").exists(), "the output dir is never copied into itself");
        // game.fun + pieces.fun + model.glb + tex/wall.png
        assert_eq!(export.staged.file_count, 4);
        assert!(export.staged.shadowed.is_empty());
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
        assert_eq!(export.staged.shadowed, vec!["index.html", "pkg"]);
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

        let missing = missing_asset_references(dir.path(), "game.fun", WEB_RESERVED);
        assert_eq!(
            missing,
            vec!["../outside.png", "/abs/path.jpg", "missing.glb", "pkg/tex.png"],
            "missing + unbundleable flagged; present / non-asset / comments / URLs not"
        );
    }

    #[test]
    fn zip_puts_the_bundle_contents_at_the_archive_root() {
        let dir = TestDir::new("zip");
        dir.write("game.fun", "let init = 0");
        dir.write("tex/wall.png", "png-bytes");

        let wd = dir.path().to_string_lossy().to_string();
        let export = export_functor_lang_wasm(&wd, "game.fun").unwrap();
        let zip_path = dir.path().join("dist/bundle.zip");
        let bytes = zip_bundle(&export.out_dir, &zip_path).unwrap();
        assert_eq!(bytes, fs::metadata(&zip_path).unwrap().len());

        let mut archive = zip::ZipArchive::new(fs::File::open(&zip_path).unwrap()).unwrap();
        let names: Vec<String> = (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().to_string())
            .collect();
        // Sorted, rooted at the bundle (index.html at the zip root — the
        // itch.io HTML5 layout), forward slashes only.
        assert_eq!(
            names,
            vec![
                "game.fun",
                "index.html",
                "pkg/functor_runtime_web.js",
                "pkg/functor_runtime_web_bg.wasm",
                "tex/wall.png",
            ]
        );
        // Round-trip: the archived source matches the exported file.
        let mut entry = archive.by_name("game.fun").unwrap();
        let mut content = String::new();
        std::io::Read::read_to_string(&mut entry, &mut content).unwrap();
        assert_eq!(content, "let init = 0");
        drop(entry);
        drop(archive);

        // Deterministic: re-zipping identical inputs is byte-identical (the
        // entry timestamp is pinned, not wall-clock).
        let again = dir.path().join("dist/bundle-again.zip");
        zip_bundle(&export.out_dir, &again).unwrap();
        assert_eq!(fs::read(&zip_path).unwrap(), fs::read(&again).unwrap());
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
        assert_eq!(export.staged.file_count, 1, "only game.fun ships");
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
        assert_eq!(export.staged.skipped_symlinks.len(), 1);
        assert!(export.staged.skipped_symlinks[0].ends_with("loop"));
    }
}
