//! `functor bake` end to end through the real binary: a project's `bakes`
//! become a lightmap PNG and a mesh, the same flags give the same bytes, and
//! bad input is an error that says what is wrong.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A 2 m floor quad under a lamp.
const FLOOR: &str = r#"let floorData = {
  positions: Bytes.fromF32le([-1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0, 0.0, -1.0, -1.0, 0.0, -1.0]),
  normals: Bytes.fromF32le([0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0]),
  uvs: Bytes.fromF32le([]),
  uvs2: Bytes.fromF32le([]),
  triangles: Bytes.fromU32le([0.0, 1.0, 2.0, 0.0, 2.0, 3.0]),
  groups: [],
  lightmap: Option.None
}
let lights = [Light.ambient(Color.rgb(0.2, 0.2, 0.2)), Light.point(Vec3.make(0.0, 1.0, 0.0), Color.rgb(1.0, 1.0, 1.0), 1.0, 3.0)]
"#;

/// A one-file project: [`FLOOR`] plus `bakes`.
fn project(name: &str, bakes: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("functor-bake-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("functor.json"), r#"{"language":"functor-lang","entry":"game.fun"}"#).unwrap();
    std::fs::write(dir.join("game.fun"), format!("{FLOOR}{bakes}\n")).unwrap();
    dir
}

/// Run `functor bake` with `args`; its output either way.
fn bake(dir: &Path, args: &[&str]) -> Result<String, String> {
    let run = Command::new(env!("CARGO_BIN_EXE_functor"))
        .arg("-d")
        .arg(dir)
        .arg("bake")
        .args(args)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&run.stdout).into_owned() + &String::from_utf8_lossy(&run.stderr);
    if run.status.success() {
        Ok(text)
    } else {
        Err(text)
    }
}

const ONE: &str = r#"let bakes = [{ name: "floor", mesh: floorData, lights: lights }]"#;

#[test]
fn bakes_write_a_lightmap_and_a_mesh() {
    let dir = project("ok", ONE);
    // Default: `baked/` in the project.
    let out = bake(&dir, &["--samples", "4", "--texels-per-meter", "8"]).unwrap();
    assert!(out.contains("baked `floor`: 2 triangles, 2 lights, 32×32 lightmap"), "{out}");
    let png = std::fs::read(dir.join("baked/floor.png")).unwrap();
    let mesh = std::fs::read(dir.join("baked/floor.mesh.bin")).unwrap();
    assert_eq!(&mesh[..4], b"FLMB");

    let reader = png::Decoder::new(std::io::Cursor::new(&png)).read_info().unwrap();
    let info = reader.info();
    assert_eq!((info.width, info.height, info.color_type), (32, 32, png::ColorType::Rgba));
    let text: Vec<_> = info.uncompressed_latin1_text.iter().map(|t| (t.keyword.as_str(), t.text.as_str())).collect();
    assert!(text.contains(&("functor-bake-version", "1")), "{text:?}");
    assert!(text.contains(&("functor-bake-settings", "texelsPerMeter=8 samples=4 aoDistance=1 seed=0 maxSize=2048")), "{text:?}");

    // The same flags, the same bytes.
    let again = dir.join("again");
    bake(&dir, &["--samples", "4", "--texels-per-meter", "8", "-o", again.to_str().unwrap()]).unwrap();
    assert_eq!(std::fs::read(again.join("floor.png")).unwrap(), png);
    assert_eq!(std::fs::read(again.join("floor.mesh.bin")).unwrap(), mesh);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn bad_bakes_and_flags_are_errors() {
    let cases: [(&str, &str, &[&str], &str); 7] = [
        ("none", "let x = 1.0", &[], "no top-level `let bakes = …` (a List<Lightmap.bakeJob>) to bake"),
        ("empty", "let bakes : List<Lightmap.bakeJob> = []", &[], "`bakes` is empty: nothing to bake"),
        (
            "twice",
            r#"let bakes = [{ name: "floor", mesh: floorData, lights: lights }, { name: "floor", mesh: floorData, lights: [] }]"#,
            &[],
            "`bakes`: bakes[1].name: \"floor\" is used twice",
        ),
        ("density", ONE, &["--texels-per-meter", "0"], "`floor`: texelsPerMeter must be a finite number above 0, got 0"),
        ("size", ONE, &["--max-size", "100"], "`floor`: maxSize must be a power of two, got 100"),
        ("small", ONE, &["--max-size", "8"], "`floor`: the chart from triangle 0 is 32×32 texels plus padding, over maxSize 8"),
        ("distance", ONE, &["--ao-distance", "0"], "`floor`: the occlusion distance must be a finite number above 0, got 0"),
    ];
    for (name, bakes, args, expected) in cases {
        let dir = project(name, bakes);
        let err = bake(&dir, args).unwrap_err();
        assert!(err.contains(expected), "{name}: {err}");
        // Nothing is written on an error.
        assert!(!dir.join("baked").exists(), "{name}");
        std::fs::remove_dir_all(dir).ok();
    }
}
