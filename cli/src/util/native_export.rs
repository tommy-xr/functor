//! `build native`: write the project as a runnable native bundle in
//! `dist/native/<os>-<arch>/` — a copy of the RUNNING `functor` binary
//! (the desktop runtime is built in), renamed after the project, next to
//! the same staged project file set the web export ships (see
//! `util::bundle`). Launching that binary bare boots the game: it finds
//! the adjacent `functor.json` (see `bundled_invocation` in main.rs).
//! Zip the folder to share a playable build for this platform.

use std::io::Error;
use std::path::{Path, PathBuf};

use super::bundle::{project_file_urls, project_name, stage_bundle, StagedBundle};

#[derive(Debug)]
pub struct NativeExport {
    /// The bundle directory: `<project>/dist/native/<os>-<arch>`.
    pub out_dir: PathBuf,
    /// The game binary inside it, named after the project.
    pub binary_path: PathBuf,
    /// Bytes of that binary.
    pub binary_bytes: u64,
    pub staged: StagedBundle,
}

/// Export the project as a native bundle for the platform this CLI runs
/// on. `exe` is the binary to ship — the caller passes
/// `std::env::current_exe()` — so bundling needs no download and no
/// compile; a release-built `functor` produces a release-grade bundle.
pub fn export_functor_lang_native(
    working_directory: &str,
    entry: &str,
    exe: &Path,
) -> Result<NativeExport, Error> {
    let root = Path::new(working_directory);
    let target = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
    let out = root.join("dist").join("native").join(&target);

    // Rebuilding THROUGH a bundled binary (`./dist/native/<t>/game build
    // native`) would wipe the running executable out from under the copy
    // below (and can't delete it at all on Windows). Refuse up front.
    let exe_canonical = std::fs::canonicalize(exe).unwrap_or_else(|_| exe.to_path_buf());
    if let Ok(root_canonical) = std::fs::canonicalize(root) {
        if exe_canonical.starts_with(root_canonical.join("dist")) {
            return Err(Error::other(format!(
                "{} lives inside the output directory — rebuild with a functor binary \
outside dist/ (the bundle would wipe the running executable)",
                exe.display()
            )));
        }
    }

    // Native reserves only the output dir and its own binary name — the
    // web-only names (index.html, pkg) ship normally in a native bundle.
    // Reserving the binary name keeps a same-named project file from being
    // silently clobbered by the copy below (it's skipped + reported).
    let name = format!("{}{}", project_name(root), std::env::consts::EXE_SUFFIX);
    let reserved = ["dist", name.as_str()];

    let files = project_file_urls(working_directory, entry);
    let staged = stage_bundle(root, &out, entry, &files, &reserved)?;

    // The binary goes in last (like the web runtime files) so nothing in
    // the project can shadow it. `fs::copy` preserves the exec bit.
    let binary_path = out.join(&name);
    let binary_bytes = std::fs::copy(exe, &binary_path)?;

    Ok(NativeExport {
        out_dir: out,
        binary_path,
        binary_bytes,
        staged,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU32, Ordering};

    static NEXT_DIR: AtomicU32 = AtomicU32::new(0);

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(name: &str) -> Self {
            let suffix = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "functor-native-{name}-{}-{suffix}",
                std::process::id()
            ));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
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
    fn exports_binary_plus_project_under_a_target_dir() {
        let dir = TestDir::new("layout");
        dir.write("functor.json", r#"{"language":"functor-lang","entry":"game.fun"}"#);
        dir.write("game.fun", "let init = 0");
        dir.write("laser.ogg", "ogg-bytes");
        dir.write("dist/native/stale-target/old", "wiped? no — sibling target");
        let fake_exe = dir.0.join(".fake-functor"); // hidden: not copied as project data
        fs::write(&fake_exe, "binary-bytes").unwrap();

        let wd = dir.0.to_string_lossy().to_string();
        let export = export_functor_lang_native(&wd, "game.fun", &fake_exe).unwrap();

        let target = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
        assert!(export.out_dir.ends_with(Path::new("dist/native").join(&target)));
        // The binary is named after the project dir and carries the exe bytes.
        let expected_name = format!(
            "{}{}",
            dir.0.file_name().unwrap().to_string_lossy(),
            std::env::consts::EXE_SUFFIX
        );
        assert_eq!(
            export.binary_path.file_name().unwrap().to_string_lossy(),
            expected_name
        );
        assert_eq!(fs::read(&export.binary_path).unwrap(), b"binary-bytes");
        // The game rides along — functor.json included, so the binary can
        // find its game at launch.
        assert!(export.out_dir.join("functor.json").is_file());
        assert!(export.out_dir.join("game.fun").is_file());
        assert!(export.out_dir.join("laser.ogg").is_file());
        // Only THIS target's dir is wiped; a sibling target's bundle stays.
        assert!(dir.0.join("dist/native/stale-target/old").is_file());
    }

    #[test]
    fn native_ships_web_reserved_names_but_not_its_own_binary_name() {
        let dir = TestDir::new("reserved");
        dir.write("functor.json", r#"{"language":"functor-lang","entry":"game.fun"}"#);
        dir.write("game.fun", "let init = 0");
        // Web-only reserved names are REAL project data for a native bundle.
        dir.write("index.html", "native games may ship one");
        dir.write("pkg/model.glb", "glb-bytes");
        // A project file named exactly like the generated binary would be
        // clobbered by the copy — reserved instead (skipped + reported).
        let binary_name = format!(
            "{}{}",
            dir.0.file_name().unwrap().to_string_lossy(),
            std::env::consts::EXE_SUFFIX
        );
        dir.write(&binary_name, "project data, not the binary");
        let fake_exe = dir.0.join(".fake-functor");
        fs::write(&fake_exe, "binary-bytes").unwrap();

        let wd = dir.0.to_string_lossy().to_string();
        let export = export_functor_lang_native(&wd, "game.fun", &fake_exe).unwrap();

        assert!(export.out_dir.join("index.html").is_file());
        assert!(export.out_dir.join("pkg/model.glb").is_file());
        assert_eq!(
            fs::read(&export.binary_path).unwrap(),
            b"binary-bytes",
            "the binary wins its name"
        );
        assert_eq!(export.staged.shadowed, vec![binary_name]);
    }

    #[test]
    fn refuses_to_rebuild_through_a_bundled_binary() {
        let dir = TestDir::new("self-wipe");
        dir.write("functor.json", r#"{"language":"functor-lang","entry":"game.fun"}"#);
        dir.write("game.fun", "let init = 0");
        // The running exe living inside dist/ is exactly the
        // `./dist/native/<t>/game build native` self-wipe scenario.
        dir.write("dist/native/some-target/game", "the running binary");
        let exe_in_dist = dir.0.join("dist/native/some-target/game");

        let wd = dir.0.to_string_lossy().to_string();
        let err = export_functor_lang_native(&wd, "game.fun", &exe_in_dist).unwrap_err();
        assert!(err.to_string().contains("output directory"), "{err}");
        assert!(exe_in_dist.is_file(), "nothing was wiped");
    }

    #[cfg(unix)]
    #[test]
    fn the_copied_binary_keeps_its_exec_bit() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TestDir::new("execbit");
        dir.write("functor.json", r#"{"language":"functor-lang","entry":"game.fun"}"#);
        dir.write("game.fun", "let init = 0");
        let fake_exe = dir.0.join(".fake-functor");
        fs::write(&fake_exe, "binary-bytes").unwrap();
        fs::set_permissions(&fake_exe, fs::Permissions::from_mode(0o755)).unwrap();

        let wd = dir.0.to_string_lossy().to_string();
        let export = export_functor_lang_native(&wd, "game.fun", &fake_exe).unwrap();
        let mode = fs::metadata(&export.binary_path).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111, "exec bits survive the copy");
    }
}
