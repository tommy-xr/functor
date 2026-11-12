//! Reading a project file for the page's read-only code view.

use std::io::Read;
use std::path::Path;

use functor_app_core::projects::check_relative;

/// Larger files are refused rather than shown in part.
pub const MAX_BYTES: u64 = 1024 * 1024;

/// The UTF-8 text of `path` inside the project at `dir`, or why not. A path
/// that is not plain relative names, resolves (through any symlink) outside
/// the project, is not a regular file, is over [`MAX_BYTES`] or is not
/// UTF-8 is refused.
pub fn read_project_file(dir: &Path, path: &str) -> Result<String, String> {
    check_relative(path)?;
    let root = std::fs::canonicalize(dir).map_err(|e| format!("cannot open the project: {e}"))?;
    let file =
        std::fs::canonicalize(root.join(path)).map_err(|e| format!("cannot open {path}: {e}"))?;
    if !file.starts_with(&root) {
        return Err(format!("{path} leads outside the project"));
    }
    // Checked before opening, so a FIFO is never opened (it would block).
    let meta = std::fs::metadata(&file).map_err(|e| format!("cannot open {path}: {e}"))?;
    if !meta.is_file() {
        return Err(format!("{path} is not a file"));
    }
    let too_large = || format!("{path} is larger than {} KB", MAX_BYTES / 1024);
    if meta.len() > MAX_BYTES {
        return Err(too_large());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(&file)
        .and_then(|f| f.take(MAX_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|e| format!("cannot read {path}: {e}"))?;
    // It may have grown since the check.
    if bytes.len() as u64 > MAX_BYTES {
        return Err(too_large());
    }
    String::from_utf8(bytes).map_err(|_| format!("{path} is not UTF-8 text"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A project `p` with `game.fun` inside a temp dir, beside a `secret`.
    fn project() -> (tempfile::TempDir, std::path::PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("p");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("game.fun"), "let init = 0\n").unwrap();
        std::fs::write(dir.join("src/more.fun"), "// more\n").unwrap();
        std::fs::write(tmp.path().join("secret"), "password").unwrap();
        (tmp, dir)
    }

    #[test]
    fn reads_a_project_file() {
        let (_tmp, dir) = project();
        assert_eq!(
            read_project_file(&dir, "game.fun"),
            Ok("let init = 0\n".into())
        );
        assert_eq!(
            read_project_file(&dir, "src/more.fun"),
            Ok("// more\n".into())
        );
    }

    #[test]
    fn refuses_traversal_and_absolute_paths() {
        let (tmp, dir) = project();
        let secret = tmp.path().join("secret");
        for path in [
            "../secret",
            "src/../../secret",
            secret.to_str().unwrap(),
            "",
            ".",
        ] {
            let read = read_project_file(&dir, path);
            assert!(
                read.as_ref()
                    .is_err_and(|e| e.contains("not a path inside")),
                "{path}: {read:?}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn refuses_a_symlink_out_of_the_project() {
        let (tmp, dir) = project();
        std::os::unix::fs::symlink(tmp.path().join("secret"), dir.join("link.fun")).unwrap();
        std::os::unix::fs::symlink(tmp.path(), dir.join("up")).unwrap();
        for path in ["link.fun", "up/secret"] {
            let read = read_project_file(&dir, path);
            assert_eq!(read, Err(format!("{path} leads outside the project")));
        }
        // One that stays inside is followed.
        std::os::unix::fs::symlink(dir.join("game.fun"), dir.join("alias.fun")).unwrap();
        assert_eq!(
            read_project_file(&dir, "alias.fun"),
            Ok("let init = 0\n".into())
        );
    }

    #[test]
    fn refuses_large_binary_missing_and_non_files() {
        let (_tmp, dir) = project();
        std::fs::write(dir.join("big.fun"), vec![b'a'; MAX_BYTES as usize + 1]).unwrap();
        std::fs::write(dir.join("max.fun"), vec![b'a'; MAX_BYTES as usize]).unwrap();
        std::fs::write(dir.join("bin.fun"), [0x66, 0xff, 0xfe]).unwrap();
        assert_eq!(
            read_project_file(&dir, "big.fun"),
            Err("big.fun is larger than 1024 KB".into())
        );
        assert_eq!(
            read_project_file(&dir, "max.fun").map(|t| t.len()),
            Ok(MAX_BYTES as usize)
        );
        assert_eq!(
            read_project_file(&dir, "bin.fun"),
            Err("bin.fun is not UTF-8 text".into())
        );
        assert!(read_project_file(&dir, "none.fun")
            .is_err_and(|e| e.starts_with("cannot open none.fun")));
        assert_eq!(
            read_project_file(&dir, "src"),
            Err("src is not a file".into())
        );
    }
}
