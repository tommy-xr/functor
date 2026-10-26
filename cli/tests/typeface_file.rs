//! `Typeface.file` end to end through the real binary (the project root is
//! process-global, so this runs in its own process): a project's copy of a
//! built-in font exports byte-identical to the built-in; bad paths are
//! errors that name them.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A one-file project whose part is "Love" in `typeface`.
fn project(name: &str, typeface: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("functor-typeface-{name}-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("fonts")).unwrap();
    std::fs::write(
        dir.join("functor.json"),
        r#"{"language":"functor-lang","entry":"part.fun"}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("part.fun"),
        format!("let part = Sketch.text({typeface}, 6.0, \"Love\") |> Solid.extrude(0.6)\n"),
    )
    .unwrap();
    let sans = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../functor-solid/fonts/FunctorSans-Regular.ttf");
    std::fs::copy(sans, dir.join("fonts/Sans.ttf")).unwrap();
    dir
}

/// Export the project's part; `Ok(stl)` or `Err(output)`.
fn export(dir: &Path) -> Result<Vec<u8>, String> {
    let out = dir.join("part.stl");
    let run = Command::new(env!("CARGO_BIN_EXE_functor"))
        .arg("-d")
        .arg(dir)
        .args(["export", "-o"])
        .arg(&out)
        .output()
        .unwrap();
    let text =
        String::from_utf8_lossy(&run.stdout).into_owned() + &String::from_utf8_lossy(&run.stderr);
    if run.status.success() {
        Ok(std::fs::read(out).unwrap())
    } else {
        Err(text)
    }
}

#[test]
fn a_font_file_exports_like_the_built_in_it_copies() {
    let file = project("file", "Typeface.file(\"fonts/Sans.ttf\")");
    let built_in = project("builtin", "Typeface.sans()");
    assert_eq!(export(&file).unwrap(), export(&built_in).unwrap());
    for dir in [file, built_in] {
        std::fs::remove_dir_all(dir).ok();
    }
}

#[test]
fn missing_and_escaping_font_paths_are_errors() {
    let missing = project("missing", "Typeface.file(\"fonts/Missing.ttf\")");
    let err = export(&missing).unwrap_err();
    assert!(
        err.contains("fonts/Missing.ttf") && err.contains("part.fun:1"),
        "{err}"
    );
    let escaping = project("escaping", "Typeface.file(\"../Sans.ttf\")");
    let err = export(&escaping).unwrap_err();
    assert!(err.contains("inside the project"), "{err}");
    // A symlink inside the project can't reach a font outside it.
    let linked = project("linked", "Typeface.file(\"fonts/Linked.ttf\")");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            escaping.join("fonts/Sans.ttf"),
            linked.join("fonts/Linked.ttf"),
        )
        .unwrap();
        let err = export(&linked).unwrap_err();
        assert!(err.contains("inside the project"), "{err}");
    }
    for dir in [missing, escaping, linked] {
        std::fs::remove_dir_all(dir).ok();
    }
}
