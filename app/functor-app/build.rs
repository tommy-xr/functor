//! Embeds the built UI (`app/ui/dist`, from `npm run build:app-ui`) when it
//! exists, so a release binary carries it; without it the crate still builds
//! and serves a page saying how to build the UI.

use std::fmt::Write;
use std::path::{Path, PathBuf};

fn main() {
    let ui = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ui");
    let dist = ui.join("dist");
    let mut files = Vec::new();
    if dist.join("index.html").is_file() {
        walk(&dist, &mut files);
        files.sort();
        println!("cargo:rerun-if-changed={}", dist.display());
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
