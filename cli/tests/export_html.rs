//! `functor export --html` through the real binary: one self-contained page
//! holding every part's mesh, and errors that say what is wrong. Headless.

use std::path::{Path, PathBuf};
use std::process::Command;

use base64::Engine;

/// A one-file project with `src` as its entry.
fn project(name: &str, src: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("functor-html-{name}-{}", std::process::id()));
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

/// The page's embedded parts: (name, triangle count from the index data).
fn parts(page: &str) -> Vec<(String, usize)> {
    let tag = r#"<script id="parts" type="application/json">"#;
    let body = &page[page.find(tag).unwrap() + tag.len()..];
    let json: serde_json::Value =
        serde_json::from_str(&body[..body.find("</script>").unwrap()]).unwrap();
    json["parts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            let indices = base64::engine::general_purpose::STANDARD
                .decode(p["indices"].as_str().unwrap())
                .unwrap();
            let per_triangle = if p["index32"].as_bool().unwrap() {
                12
            } else {
                6
            };
            assert_eq!(indices.len() % per_triangle, 0);
            let triangles = indices.len() / per_triangle;
            assert_eq!(p["triangles"].as_u64().unwrap() as usize, triangles);
            (p["name"].as_str().unwrap().to_string(), triangles)
        })
        .collect()
}

#[test]
fn html_embeds_every_part_and_loads_only_pinned_scripts() {
    let dir = project("page", PARTS);
    let html = dir.join("parts.html");
    let out = export(&dir, &["--html", html.to_str().unwrap()]).unwrap();
    assert!(out.contains("2 part(s)"), "{out}");
    let page = std::fs::read_to_string(&html).unwrap();
    assert!(page.len() < 100_000, "{} bytes", page.len());
    assert!(page.contains(&format!(
        "<title>{}</title>",
        dir.file_name().unwrap().to_str().unwrap()
    )));

    let found = parts(&page);
    let names: Vec<_> = found.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["part", "peg"]);
    assert_eq!(found[0].1, 12, "a box is 12 triangles");
    assert!(found[1].1 > 12);
    assert!(
        out.contains(&format!("{} triangles", found[0].1 + found[1].1)),
        "{out}"
    );

    // Every external URL is a pinned three.js build or Google Fonts.
    let allowed = [
        "https://cdnjs.cloudflare.com/ajax/libs/three.js/r128/three.min.js",
        "https://cdn.jsdelivr.net/npm/three@0.128.0/examples/js/controls/OrbitControls.js",
        "https://fonts.googleapis.com",
        "https://fonts.googleapis.com/css2?family=Archivo:wdth,wght@62..125,400..800&family=JetBrains+Mono:wght@400;600&display=swap",
    ];
    let urls: Vec<_> = page
        .match_indices("http")
        .map(|(i, _)| &page[i..i + page[i..].find(['"', '\'', ' ', ')']).unwrap()])
        .collect();
    assert_eq!(urls.len(), allowed.len(), "{urls:?}");
    for url in urls {
        assert!(allowed.contains(&url), "unexpected external URL {url}");
    }

    // `--part` picks and orders parts.
    export(&dir, &["--html", html.to_str().unwrap(), "--part", "peg"]).unwrap();
    let page = std::fs::read_to_string(&html).unwrap();
    assert_eq!(parts(&page).len(), 1);
    assert_eq!(parts(&page)[0].0, "peg");
}

#[test]
fn html_errors_name_the_problem() {
    let dir = project("errors", PARTS);
    let err = export(&dir, &["--html", "parts.htm"]).unwrap_err();
    assert!(err.contains("use a .html file"), "{err}");
    let err = export(&dir, &["--html", "parts.html", "--part", "lid"]).unwrap_err();
    assert!(
        err.contains("no part `lid`: the parts are `part`, `peg`"),
        "{err}"
    );
    let err = export(&dir, &["--html", "parts.html", "-o", "part.stl"]).unwrap_err();
    assert!(err.contains("cannot be used with"), "{err}");

    let empty = project("empty", "let size = 3.0\n");
    let err = export(&empty, &["--html", "parts.html"]).unwrap_err();
    assert!(err.contains("game.fun has no parts to view"), "{err}");
}
