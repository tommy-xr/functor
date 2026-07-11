//! In-browser Functor Lang language intelligence for the sandbox editor.
//!
//! The same front-end the LSP runs (`tools/functor-lang-lsp`) — diagnostics,
//! inlay hints, code lenses, and hover — compiled to a small wasm module the
//! CodeMirror editor imports directly. The editor lives in the sandbox's
//! PARENT page (the engine runs in an iframe), so this is its own tiny bundle
//! rather than a slice of the multi-MB engine wasm.
//!
//! Two exports, both returning JSON strings so the JS side needs no schema:
//!
//! - [`functor_lang_analyze`] runs ONE load/check pass and reports all three of
//!   diagnostics, inlay hints, and code lenses.
//! - [`functor_lang_hover`] answers a single hover at an offset.
//!
//! **Positions are UTF-16 code units**, matching CodeMirror (and JS string)
//! indexing: byte offsets from the parser are converted here in Rust, exactly
//! as the LSP converts them to LSP `character` counts.
//!
//! All logic lives in plain `pub fn`s (`analyze_json` / `hover_json`) with thin
//! `#[wasm_bindgen]` wrappers, so `cargo test -p functor-lang-wasm` exercises it
//! natively.

use std::path::{Path, PathBuf};

use functor_lang::project::{self, Project, SourceFile};
use serde_json::{json, Value};
use wasm_bindgen::prelude::*;

/// The single user file's name. The sandbox is a one-file program; the loader
/// puts it first, so its span base is 0 and its project-wide offsets ARE byte
/// offsets into `src`.
const USER_FILE: &str = "game.fun";

/// Analyze `src`: one load/check pass producing diagnostics, inlay hints, and
/// code lenses. Returns a JSON string:
///
/// ```json
/// {
///   "diagnostics": [{ "from": u16, "to": u16, "message": str, "severity": "error" }],
///   "inlays":      [{ "pos": u16, "label": str }],
///   "lenses":      [{ "line": u32, "from": u16, "text": str }]
/// }
/// ```
///
/// `from`/`to`/`pos` are whole-document UTF-16 offsets; `line` is 0-based.
/// A load-level failure (parse/link error) comes back as a single diagnostic —
/// never an exception. Prelude spans are filtered out.
#[wasm_bindgen]
pub fn functor_lang_analyze(src: &str) -> String {
    analyze_json(src)
}

/// Hover at `offset` (a UTF-16 code-unit position). Returns `{"from","to","text"}`
/// (both UTF-16 offsets) or `""` when there is nothing to show.
#[wasm_bindgen]
pub fn functor_lang_hover(src: &str, offset: f64) -> String {
    hover_json(src, offset.max(0.0) as usize)
}

/// See [`functor_lang_analyze`]. Pure — the tested seam.
pub fn analyze_json(src: &str) -> String {
    let project = match load(src) {
        Ok(project) => project,
        // A parse/link failure surfaces as one diagnostic at the reported
        // point, not an error.
        Err(err) => return load_error_json(src, err),
    };
    let Some(file) = project.sources.file_by_path(Path::new(USER_FILE)) else {
        return empty_analysis();
    };
    let (diags, types) = project.check_with_types();

    let diagnostics: Vec<Value> = diags
        .into_iter()
        .filter(|d| owns(file, d.span.start))
        .map(|d| {
            json!({
                "from": to_u16(file, d.span.start),
                "to": to_u16(file, d.span.end),
                "message": d.message,
                "severity": "error",
            })
        })
        .collect();

    let inlays: Vec<Value> = functor_lang::inlay::inlay_hints(&project.module, &types)
        .into_iter()
        .filter(|h| owns(file, h.offset))
        .map(|h| json!({ "pos": to_u16(file, h.offset), "label": h.label }))
        .collect();

    let lenses: Vec<Value> = functor_lang::codelens::signatures(&project.module, &types)
        .into_iter()
        .filter(|l| owns(file, l.span.start))
        .map(|l| {
            json!({
                "line": line_of(file, l.span.start),
                "from": to_u16(file, l.span.start),
                "text": l.title,
            })
        })
        .collect();

    json!({ "diagnostics": diagnostics, "inlays": inlays, "lenses": lenses }).to_string()
}

/// See [`functor_lang_hover`]. Pure — the tested seam. `offset` is UTF-16.
pub fn hover_json(src: &str, offset: usize) -> String {
    let Ok(project) = load(src) else {
        return String::new();
    };
    let Some(file) = project.sources.file_by_path(Path::new(USER_FILE)) else {
        return String::new();
    };
    let byte = file.base + from_u16(&file.src, offset);
    let (_, types) = project.check_with_types();
    let Some((span, text)) = functor_lang::hover::hover_text(&project.module, &types, byte) else {
        return String::new();
    };
    if !owns(file, span.start) {
        return String::new();
    }
    json!({
        "from": to_u16(file, span.start),
        "to": to_u16(file, span.end),
        "text": text,
    })
    .to_string()
}

/// Load `src` as a single-file project with the host prelude injected (so
/// `Scene.*` / `Camera.*` / … typecheck), mirroring the LSP.
fn load(src: &str) -> Result<Project, project::ProjectError> {
    project::load_sources_with_prelude(
        vec![(PathBuf::from(USER_FILE), src.to_string())],
        &functor_prelude::modules(),
    )
}

/// A load failure → one diagnostic at its reported point. The `ProjectError`
/// gives a 1-based (line, col) in the user file (base 0); convert it to a
/// zero-width UTF-16 offset.
fn load_error_json(src: &str, err: project::ProjectError) -> String {
    let byte = line_col_to_byte(src, err.line, err.col);
    let at = utf16_len(&src[..byte.min(src.len())]);
    json!({
        "diagnostics": [{ "from": at, "to": at, "message": err.message, "severity": "error" }],
        "inlays": [],
        "lenses": [],
    })
    .to_string()
}

fn empty_analysis() -> String {
    json!({ "diagnostics": [], "inlays": [], "lenses": [] }).to_string()
}

/// Whether a project-wide `offset` falls in `file` (its half-open base range) —
/// the LSP's `owns`, which keeps prelude/builtin spans from leaking.
fn owns(file: &SourceFile, offset: usize) -> bool {
    file.base <= offset && offset <= file.base + file.src.len()
}

/// A project-wide byte offset → a whole-document UTF-16 offset within `file`.
fn to_u16(file: &SourceFile, offset: usize) -> usize {
    let local = offset.saturating_sub(file.base).min(file.src.len());
    utf16_len(&file.src[..local])
}

/// The 0-based line a project-wide byte offset sits on within `file`.
fn line_of(file: &SourceFile, offset: usize) -> usize {
    let local = offset.saturating_sub(file.base).min(file.src.len());
    file.src[..local].matches('\n').count()
}

/// UTF-16 code-unit length of `s` (what CodeMirror/JS counts).
fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// A whole-document UTF-16 offset → a byte offset into `src`. Clamps past the
/// end. Inverse of [`utf16_len`] over a prefix.
fn from_u16(src: &str, units: usize) -> usize {
    let mut seen = 0;
    for (byte, ch) in src.char_indices() {
        if seen >= units {
            return byte;
        }
        seen += ch.len_utf16();
    }
    src.len()
}

/// A 1-based (line, col) — `col` counting characters from the line start, as
/// `functor_lang::line_col` reports — back to a byte offset into `src`.
fn line_col_to_byte(src: &str, line: usize, col: usize) -> usize {
    let mut cur_line = 1;
    let mut cur_col = 1;
    for (byte, ch) in src.char_indices() {
        if cur_line == line && cur_col == col {
            return byte;
        }
        if ch == '\n' {
            cur_line += 1;
            cur_col = 1;
        } else {
            cur_col += 1;
        }
    }
    src.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> Value {
        serde_json::from_str(json).unwrap()
    }

    // A valid program using prelude names typechecks: no diagnostics, and the
    // prelude's own files leak nothing into inlays/lenses (only the user file's
    // one def is reported).
    #[test]
    fn valid_program_with_prelude_is_clean() {
        let src = "let draw = (model, tts: Float) =>\n  \
            Frame.create(Camera.lookAt(0.0, 0.0, -6.0, 0.0, 0.0, 0.0), Scene.cube())\n";
        let out = parse(&analyze_json(src));
        assert_eq!(out["diagnostics"].as_array().unwrap().len(), 0, "{out}");
        // Inlays/lenses come only from the user file — never from the injected
        // prelude `.funi` modules.
        for lens in out["lenses"].as_array().unwrap() {
            let from = lens["from"].as_u64().unwrap() as usize;
            assert!(from <= utf16_len(src), "lens leaked past user file: {lens}");
        }
        for inlay in out["inlays"].as_array().unwrap() {
            let pos = inlay["pos"].as_u64().unwrap() as usize;
            assert!(pos <= utf16_len(src), "inlay leaked past user file: {inlay}");
        }
        // The one top-level def gets a signature lens on line 0.
        let lenses = out["lenses"].as_array().unwrap();
        assert!(
            lenses.iter().any(|l| l["line"] == 0),
            "expected a lens for `draw` on line 0: {out}"
        );
    }

    // A type error → exactly one diagnostic, with correct ASCII UTF-16 offsets
    // (here byte == UTF-16, since the source is ASCII) spanning the offending
    // sub-expression.
    #[test]
    fn type_error_reports_one_diagnostic() {
        // `1.0 + "x"`: adding a string to a float is a type error.
        let src = "let bad = 1.0 + \"x\"\n";
        let out = parse(&analyze_json(src));
        let diags = out["diagnostics"].as_array().unwrap();
        assert_eq!(diags.len(), 1, "{out}");
        let d = &diags[0];
        let from = d["from"].as_u64().unwrap() as usize;
        let to = d["to"].as_u64().unwrap() as usize;
        assert!(from < to && to <= utf16_len(src), "span {from}..{to}: {out}");
        assert_eq!(d["severity"], "error");
    }

    // A parse failure comes back as a single diagnostic (never an exception),
    // at the reported point.
    #[test]
    fn parse_error_is_a_single_diagnostic() {
        let out = parse(&analyze_json("let init = {\n"));
        let diags = out["diagnostics"].as_array().unwrap();
        assert_eq!(diags.len(), 1, "{out}");
        assert_eq!(diags[0]["severity"], "error");
    }

    // Non-ASCII before an error proves the UTF-8 byte offset is converted to a
    // UTF-16 code-unit offset: the multi-byte characters make the byte offset
    // strictly larger than the UTF-16 offset.
    #[test]
    fn non_ascii_offsets_are_utf16() {
        // "héllo→" — 'é' is 2 bytes, '→' is 3 bytes — sits before the error.
        let src = "let s = \"héllo→\"\nlet bad = 1.0 + \"x\"\n";
        let out = parse(&analyze_json(src));
        let diags = out["diagnostics"].as_array().unwrap();
        assert_eq!(diags.len(), 1, "{out}");
        let from = diags[0]["from"].as_u64().unwrap() as usize;
        let to = diags[0]["to"].as_u64().unwrap() as usize;
        // The multi-byte chars on line 1 push every byte offset on line 2 ahead
        // of its UTF-16 offset: the flagged token's byte position must be
        // STRICTLY greater than the reported UTF-16 offset — proving conversion
        // happened (a raw byte offset would be equal).
        let byte_from = from_u16(src, from);
        assert!(
            byte_from > from,
            "byte from ({byte_from}) should exceed UTF-16 from ({from}) — conversion applied: {out}"
        );
        // The flagged token is the `"x"` string literal (wrong type for `+`).
        assert!(src[byte_from..].starts_with("\"x\""), "flagged {:?}: {out}", &src[byte_from..]);
        assert!(to > from && to <= utf16_len(src));
    }

    // Hover round-trips on a non-ASCII line: the returned span's UTF-16 offsets
    // map back to the identifier's bytes.
    #[test]
    fn hover_round_trips_on_non_ascii_line() {
        // `spin` is used after a non-ASCII string literal; hover over it.
        let src = "let label = \"café→\"\nlet spin = 3.0\nlet twice = spin + spin\n";
        // Offset (UTF-16) of the first `spin` in `twice`'s body.
        let byte = src.rfind("spin + spin").unwrap();
        let offset = utf16_len(&src[..byte]);
        let out = hover_json(src, offset);
        assert!(!out.is_empty(), "expected a hover result");
        let v = parse(&out);
        let from = v["from"].as_u64().unwrap() as usize;
        let to = v["to"].as_u64().unwrap() as usize;
        assert_eq!(from_u16(src, from), byte, "hover from maps back to bytes");
        assert_eq!(&src[from_u16(src, from)..from_u16(src, to)], "spin");
        assert!(v["text"].as_str().unwrap().contains("float"), "{out}");
    }
}
