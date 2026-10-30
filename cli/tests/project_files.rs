//! Project files end to end through the real binary (the project root is
//! process-global, so this runs in its own process): `Typeface.file` exports
//! byte-identical to the built-in it copies, `File.bytes` reads data in
//! `functor test`, and bad paths are errors that name them.

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

#[test]
fn file_bytes_feed_expects() {
    let dir = project("bytes", "Typeface.sans()");
    std::fs::write(dir.join("data.bin"), [7u8, 0, 0, 0]).unwrap();
    let src = std::fs::read_to_string(dir.join("part.fun")).unwrap()
        + "expect Bytes.u32le(0.0, File.bytes(Path.of(\"data.bin\"))) == Result.Ok(7.0)\n";
    std::fs::write(dir.join("part.fun"), &src).unwrap();
    let test = |dir: &Path| {
        Command::new(env!("CARGO_BIN_EXE_functor"))
            .arg("-d")
            .arg(dir)
            .arg("test")
            .output()
            .unwrap()
    };
    assert!(test(&dir).status.success());
    std::fs::write(dir.join("data.bin"), [8u8, 0, 0, 0]).unwrap();
    let failed = test(&dir);
    let output = String::from_utf8_lossy(&failed.stdout).into_owned()
        + &String::from_utf8_lossy(&failed.stderr);
    assert!(
        !failed.status.success() && output.contains("part.fun:2"),
        "{output}"
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_decoded_mesh_exports_as_a_closed_part() {
    let dir = project("mesh", "Typeface.sans()");
    // A unit cube's corners (index = x + 2y + 4z); five faces, no top.
    let corners: Vec<u8> = (0..8u8)
        .flat_map(|i| [i & 1, (i >> 1) & 1, (i >> 2) & 1])
        .flat_map(|c| f32::from(c).to_le_bytes())
        .collect();
    std::fs::write(dir.join("cube.bin"), corners).unwrap();
    std::fs::write(
        dir.join("part.fun"),
        "let part = Mesh.fromPolygons(File.bytes(Path.of(\"cube.bin\")), [[0.0, 2.0, 3.0, 1.0], \
         [0.0, 1.0, 5.0, 4.0], [1.0, 3.0, 7.0, 5.0], [3.0, 2.0, 6.0, 7.0], [2.0, 0.0, 4.0, 6.0]]) \
         |> Solid.fromMesh({ height: 20.0, nozzle: 0.8 })\n",
    )
    .unwrap();
    let stl = export(&dir).unwrap();
    let count = u32::from_le_bytes(stl[80..84].try_into().unwrap()) as usize;
    assert!(
        count > 12 && stl.len() == 84 + 50 * count,
        "{count} triangles"
    );
    std::fs::remove_dir_all(dir).ok();
}
