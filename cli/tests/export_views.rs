//! `functor export --views` through the real binary: the parts' 8-view sheet
//! as a PNG, and errors that say what is wrong. The render needs a GL
//! display, so that test is ignored by default (like the goldens).

use std::path::{Path, PathBuf};
use std::process::Command;

/// A one-file project with `src` as its entry.
fn project(name: &str, src: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("functor-views-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("functor.json"),
        r#"{"language":"functor-lang","entry":"game.fun"}"#,
    )
    .unwrap();
    std::fs::write(dir.join("game.fun"), src).unwrap();
    dir
}

const PARTS: &str =
    "let part = Solid.box(30.0, 20.0, 10.0)\nlet peg = Solid.cylinder(4.0, 25.0)\nlet size = 3.0\n";

/// Run `functor export` with `args`; its output either way.
fn export(dir: &Path, args: &[&str]) -> Result<String, String> {
    let run = Command::new(env!("CARGO_BIN_EXE_functor"))
        .arg("-d")
        .arg(dir)
        .arg("export")
        .args(args)
        .output()
        .unwrap();
    let text =
        String::from_utf8_lossy(&run.stdout).into_owned() + &String::from_utf8_lossy(&run.stderr);
    if run.status.success() {
        Ok(text)
    } else {
        Err(text)
    }
}

#[test]
#[ignore = "needs a GL display; run with --ignored"]
fn views_writes_a_sheet_with_every_view_drawn() {
    let dir = project("sheet", PARTS);
    let png = dir.join("sheet.png");
    let out = export(&dir, &["--views", png.to_str().unwrap()]).unwrap();
    assert!(out.contains("views: `part` orange, `peg` blue"), "{out}");

    let decoder = png::Decoder::new(std::fs::File::open(&png).unwrap());
    let mut reader = decoder.read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut pixels).unwrap();
    assert_eq!((info.width, info.height), (2304, 2304));
    let channels = info.line_size / info.width as usize;
    let at = |x: usize, y: usize| {
        let i = y * info.line_size + x * channels;
        [pixels[i], pixels[i + 1], pixels[i + 2]]
    };
    // Tiles are 720 px, centred at (col − 1, 1 − row) · 1.04 tiles from the
    // middle; the bottom-left cell is empty.
    let cells = [
        (0, 0),
        (1, 0),
        (2, 0),
        (0, 1),
        (1, 1),
        (2, 1),
        (1, 2),
        (2, 2),
    ];
    for (col, row) in cells {
        let cx = 1152.0 + (col as f64 - 1.0) * 1.04 * 720.0;
        let cy = 1152.0 - (1.0 - row as f64) * 1.04 * 720.0;
        let (x0, y0) = ((cx - 300.0) as usize, (cy - 300.0) as usize);
        let background = at(x0, y0);
        let drawn = (y0..y0 + 600)
            .step_by(4)
            .flat_map(|y| (x0..x0 + 600).step_by(4).map(move |x| (x, y)))
            .filter(|&(x, y)| {
                at(x, y)
                    .iter()
                    .zip(background)
                    .any(|(a, b)| a.abs_diff(b) > 24)
            })
            .count();
        assert!(drawn > 900, "view ({col}, {row}) is blank: {drawn} px");
    }
}

#[test]
fn views_needs_a_png() {
    let dir = project("ext", PARTS);
    let err = export(&dir, &["--views", "sheet.jpg"]).unwrap_err();
    assert!(err.contains("use a .png file"), "{err}");
}

#[test]
fn views_lists_the_parts_for_an_unknown_one() {
    let dir = project("unknown", PARTS);
    let err = export(&dir, &["--views", "sheet.png", "--part", "lid"]).unwrap_err();
    assert!(
        err.contains("no part `lid`: the parts are `part`, `peg`"),
        "{err}"
    );
}

#[test]
fn views_refuses_a_repeated_part() {
    let dir = project("repeat", PARTS);
    let err = export(
        &dir,
        &["--views", "sheet.png", "--part", "peg", "--part", "peg"],
    )
    .unwrap_err();
    assert!(err.contains("`--part peg` is given twice"), "{err}");
}

#[test]
fn views_needs_a_part() {
    let dir = project("none", "let size = 3.0\nlet init = 0.0\n");
    let err = export(&dir, &["--views", "sheet.png"]).unwrap_err();
    assert!(err.contains("game.fun has no parts to view"), "{err}");
}

#[test]
#[ignore = "needs a GL display; run with --ignored"]
fn views_fails_on_a_part_that_cannot_mesh() {
    let dir = project(
        "empty",
        "let part = Solid.intersect(Solid.box(1.0, 1.0, 1.0), Solid.translate(Vec3.make(5.0, 0.0, 0.0), Solid.box(1.0, 1.0, 1.0)))\n",
    );
    let png = dir.join("sheet.png");
    let err = export(&dir, &["--views", png.to_str().unwrap()]).unwrap_err();
    assert!(!png.exists(), "{err}");
}
