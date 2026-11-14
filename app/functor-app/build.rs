//! Embeds the built UI (`app/ui/dist`, from `npm run build:app-ui`) when it
//! exists, so a release binary carries it; without it the crate still builds
//! and serves a page saying how to build the UI.

use std::fmt::Write;
use std::path::{Path, PathBuf};

fn main() {
    // macOS shows this file's microphone purpose when the composer's mic
    // first records; a bare binary carries it in its `__info_plist` section.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        let plist = Path::new(env!("CARGO_MANIFEST_DIR")).join("Info.plist");
        println!("cargo:rerun-if-changed={}", plist.display());
        println!(
            "cargo:rustc-link-arg-bins=-Wl,-sectcreate,__TEXT,__info_plist,{}",
            plist.display()
        );
    }
    let ui = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ui");
    let dist = ui.join("dist");
    let mut files = Vec::new();
    // A `dist` older than the UI's own files would put an old page in a new
    // app (one whose window expects page changes that are not there).
    // Files it imports from elsewhere in the repo are not compared.
    let mut stale = false;
    if dist.join("index.html").is_file() {
        walk(&dist, &mut files);
        files.sort();
        println!("cargo:rerun-if-changed={}", dist.display());
        let src = ui.join("src");
        let mut sources = vec![ui.join("index.html"), ui.join("build.mjs")];
        if src.is_dir() {
            walk(&src, &mut sources);
        }
        for source in [&src, &sources[0], &sources[1]] {
            println!("cargo:rerun-if-changed={}", source.display());
        }
        let modified = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        let test = |p: &Path| p.file_name().is_some_and(|n| n.to_string_lossy().contains(".test."));
        let newest = sources
            .iter()
            .filter(|p| !test(p))
            .filter_map(|p| modified(p))
            .max();
        stale = newest > modified(&dist.join("app.js"));
        if stale {
            println!(
                "cargo:warning=app/ui/dist is older than its source: run `npm run build:app-ui`, then rebuild"
            );
        }
    } else {
        // Reruns once a build creates `dist`.
        println!("cargo:rerun-if-changed={}", ui.display());
    }
    let mut out = String::from("pub static UI: &[(&str, &[u8])] = &[\n");
    for file in &files {
        let name = file.strip_prefix(&dist).unwrap();
        let name: Vec<_> = name.iter().map(|c| c.to_string_lossy()).collect();
        let abs = std::fs::canonicalize(file).unwrap();
        writeln!(
            out,
            "    ({:?}, include_bytes!({:?})),",
            name.join("/"),
            abs
        )
        .unwrap();
    }
    out.push_str("];\n");
    writeln!(out, "pub static UI_STALE: bool = {stale};").unwrap();
    let path = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("ui_assets.rs");
    std::fs::write(path, out).unwrap();
}

fn walk(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, files);
        } else {
            files.push(path);
        }
    }
}
