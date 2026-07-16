//! In-browser Functor Lang language intelligence for the sandbox editor.
//!
//! The same front-end the LSP runs (`tools/functor-lang-lsp`) — diagnostics,
//! inlay hints, code lenses, and hover — compiled to a small wasm module the
//! CodeMirror editor imports directly. The editor lives in the sandbox's
//! PARENT page (the engine runs in an iframe), so this is its own tiny bundle
//! rather than a slice of the multi-MB engine wasm.
//!
//! The exports all return JSON strings so the JS side needs no schema:
//!
//! - [`functor_lang_analyze`] runs ONE load/check pass and reports all three of
//!   diagnostics, inlay hints, and code lenses.
//! - [`functor_lang_hover`] answers a single hover at an offset.
//! - [`functor_lang_complete`] answers completion candidates at an offset.
//! - The `*_project` variants ([`functor_lang_analyze_project`], …) take the
//!   WHOLE file set (`[{ "path", "source" }]`, entry first — the IDE's
//!   multi-file case) plus the active file's path, so cross-module references
//!   (`Palette.glow` from a sibling `palette.fun`) resolve instead of erroring.
//!   Results are for the active file only, in ITS local offsets.
//!
//! **Positions are UTF-16 code units**, matching CodeMirror (and JS string)
//! indexing: byte offsets from the parser are converted here in Rust, exactly
//! as the LSP converts them to LSP `character` counts.
//!
//! All logic lives in plain `pub fn`s (`analyze_json` / `hover_json`) with thin
//! `#[wasm_bindgen]` wrappers, so `cargo test -p functor-lang-wasm` exercises it
//! natively.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use functor_lang::complete::{CompletionItem, CompletionKind};
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
///   "lenses":      [{ "from": u16, "text": str }]
/// }
/// ```
///
/// `from`/`to`/`pos` are whole-document UTF-16 offsets.
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

/// Completion candidates at `offset` (a UTF-16 code-unit position). Returns a
/// JSON string `{"items": [{"label", "detail" (string|null), "kind"}]}` — the
/// `kind` a lowercase string (`"function"`, `"module"`, …). Never throws.
#[wasm_bindgen]
pub fn functor_lang_complete(src: &str, offset: f64) -> String {
    complete_json(src, offset.max(0.0) as usize)
}

/// Clear the completion last-good cache. The sandbox calls this whenever the
/// editor document is wholly replaced (example switch, inline `#src=` load,
/// reset) so candidates from the previous program can't leak into the new one.
#[wasm_bindgen]
pub fn functor_lang_reset() {
    reset_cache();
}

/// Project-aware [`functor_lang_analyze`]: `files_json` is the whole file set
/// (`[{ "path": str, "source": str }]`, entry first), `active` the path whose
/// diagnostics/inlays/lenses to report, in its local UTF-16 offsets.
///
/// The report is PER-FILE, matching the LSP's per-document model: a sibling's
/// parse/link error surfaces as a banner (it fails the whole pass), but a
/// sibling's TYPE error belongs to that file's own report — collect
/// project-wide problems by calling this once per file. An `active` not
/// present in `files_json` yields the empty result.
#[wasm_bindgen]
pub fn functor_lang_analyze_project(files_json: &str, active: &str) -> String {
    analyze_project_json(files_json, active)
}

/// Project-aware [`functor_lang_hover`]: `offset` is UTF-16, local to `active`.
#[wasm_bindgen]
pub fn functor_lang_hover_project(files_json: &str, active: &str, offset: f64) -> String {
    hover_project_json(files_json, active, offset.max(0.0) as usize)
}

/// Project-aware [`functor_lang_complete`]: `offset` is UTF-16, local to
/// `active` (whose source in `files_json` is the LIVE buffer).
#[wasm_bindgen]
pub fn functor_lang_complete_project(files_json: &str, active: &str, offset: f64) -> String {
    complete_project_json(files_json, active, offset.max(0.0) as usize)
}

/// See [`functor_lang_analyze`]. Pure — the tested seam.
pub fn analyze_json(src: &str) -> String {
    analyze_impl(single(src), Path::new(USER_FILE))
}

/// See [`functor_lang_analyze_project`]. Pure — the tested seam. A malformed
/// `files_json` yields an empty analysis, never an exception.
pub fn analyze_project_json(files_json: &str, active: &str) -> String {
    let Some(sources) = parse_files(files_json) else {
        return empty_analysis();
    };
    analyze_impl(sources, Path::new(active))
}

fn analyze_impl(sources: Vec<(PathBuf, String)>, active: &Path) -> String {
    // An `active` outside the set is a caller bug — the empty result, never a
    // report against some other file. The clone keeps the source for
    // load-error mapping (`load_sources` consumes the set).
    let Some(active_src) = sources
        .iter()
        .find(|(path, _)| path == active)
        .map(|(_, src)| src.clone())
    else {
        return empty_analysis();
    };
    let project = match load_sources(sources) {
        Ok(project) => project,
        // A parse/link failure surfaces as one diagnostic at the reported
        // point, not an error.
        Err(err) => return load_error_json(active, &active_src, err),
    };
    let Some(file) = project.sources.file_by_path(active) else {
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
                "from": to_u16(file, l.span.start),
                "text": l.title,
            })
        })
        .collect();

    json!({ "diagnostics": diagnostics, "inlays": inlays, "lenses": lenses }).to_string()
}

/// See [`functor_lang_hover`]. Pure — the tested seam. `offset` is UTF-16.
pub fn hover_json(src: &str, offset: usize) -> String {
    hover_impl(single(src), Path::new(USER_FILE), offset)
}

/// See [`functor_lang_hover_project`]. Pure — the tested seam.
pub fn hover_project_json(files_json: &str, active: &str, offset: usize) -> String {
    let Some(sources) = parse_files(files_json) else {
        return String::new();
    };
    hover_impl(sources, Path::new(active), offset)
}

fn hover_impl(sources: Vec<(PathBuf, String)>, active: &Path, offset: usize) -> String {
    let Ok(project) = load_sources(sources) else {
        return String::new();
    };
    let Some(file) = project.sources.file_by_path(active) else {
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

// The last project that loaded cleanly — dot-completion keeps answering off
// this while the live buffer is mid-edit or broken (the offset contract:
// context comes from the live `src`, candidates from a possibly-stale
// project). Refreshed on every clean load. Per-wasm-instance, single-threaded.
thread_local! {
    static LAST_GOOD: RefCell<Option<Project>> = const { RefCell::new(None) };
}

/// The most completion items to encode — bounds the JSON for a huge namespace.
const MAX_ITEMS: usize = 200;

/// See [`functor_lang_complete`]. Pure — the tested seam. `offset` is UTF-16.
///
/// Refreshes the last-good cache when `src` loads cleanly, then completes
/// against it (falling back to the previous cache when the live buffer is
/// broken; empty items when nothing has loaded yet). The byte offset the
/// language crate wants is LOCAL to `src`, so it converts straight from UTF-16
/// with no `file.base`.
pub fn complete_json(src: &str, offset: usize) -> String {
    complete_impl(single(src), Path::new(USER_FILE), offset)
}

/// See [`functor_lang_complete_project`]. Pure — the tested seam.
pub fn complete_project_json(files_json: &str, active: &str, offset: usize) -> String {
    let Some(sources) = parse_files(files_json) else {
        return empty_completion();
    };
    complete_impl(sources, Path::new(active), offset)
}

fn complete_impl(sources: Vec<(PathBuf, String)>, active: &Path, offset: usize) -> String {
    // The active file's source IS the live buffer (the offset contract:
    // context from the live text, candidates from a possibly-stale project).
    // An `active` outside the set is a caller bug — empty items, not
    // entry-scope candidates against an empty buffer.
    let Some(live) = sources
        .iter()
        .find(|(path, _)| path == active)
        .map(|(_, src)| src.clone())
    else {
        return empty_completion();
    };
    let byte = from_u16(&live, offset);
    if let Ok(project) = load_sources(sources) {
        LAST_GOOD.with(|cell| *cell.borrow_mut() = Some(project));
    }
    LAST_GOOD.with(|cell| {
        let borrow = cell.borrow();
        let Some(project) = borrow.as_ref() else {
            return empty_completion();
        };
        // Complete in the active file's module scope; a file the cached
        // project doesn't know (just created, buffer still broken) falls back
        // to the entry module.
        let module = project
            .sources
            .file_by_path(active)
            .map(|file| file.module.clone())
            .unwrap_or_else(|| project.entry.clone());
        let items = functor_lang::complete::complete(project, &module, &live, byte);
        completion_json(&items)
    })
}

/// See [`functor_lang_reset`]. Pure — the tested seam. Drops the last-good
/// project so the next completion starts from a blank cache (no candidates from
/// a previously-loaded, now-replaced document).
pub fn reset_cache() {
    LAST_GOOD.with(|cell| *cell.borrow_mut() = None);
}

/// Encode completion items as `{"items": [...]}`, capped at [`MAX_ITEMS`].
fn completion_json(items: &[CompletionItem]) -> String {
    let items: Vec<Value> = items
        .iter()
        .take(MAX_ITEMS)
        .map(|item| {
            json!({
                "label": item.label,
                "detail": item.detail,
                "kind": kind_str(item.kind),
            })
        })
        .collect();
    json!({ "items": items }).to_string()
}

fn empty_completion() -> String {
    json!({ "items": [] }).to_string()
}

/// A completion kind as a lowercase string, matching the CodeMirror `type` the
/// editor maps to a built-in icon.
fn kind_str(kind: CompletionKind) -> &'static str {
    match kind {
        CompletionKind::Function => "function",
        CompletionKind::Value => "value",
        CompletionKind::Module => "module",
        CompletionKind::Keyword => "keyword",
        CompletionKind::Constructor => "constructor",
        CompletionKind::Field => "field",
    }
}

/// The single-file wrappers' file set: `src` as the one `game.fun`.
fn single(src: &str) -> Vec<(PathBuf, String)> {
    vec![(PathBuf::from(USER_FILE), src.to_string())]
}

/// Load a file set as a project with the host prelude injected (so `Scene.*` /
/// `Camera.*` / … typecheck), mirroring the LSP.
fn load_sources(sources: Vec<(PathBuf, String)>) -> Result<Project, project::ProjectError> {
    project::load_sources_with_prelude(sources, &functor_prelude::modules())
}

/// Parse the `*_project` file-set payload: a JSON array of
/// `{ "path": str, "source": str }`, entry first. `None` on any malformation
/// (the callers degrade to their empty result — never an exception).
fn parse_files(files_json: &str) -> Option<Vec<(PathBuf, String)>> {
    let parsed: Value = serde_json::from_str(files_json).ok()?;
    let mut sources = Vec::new();
    for entry in parsed.as_array()? {
        sources.push((
            PathBuf::from(entry.get("path")?.as_str()?),
            entry.get("source")?.as_str()?.to_string(),
        ));
    }
    (!sources.is_empty()).then_some(sources)
}

/// A load failure → one diagnostic in the ACTIVE file. An error in the active
/// file lands at its reported point (the `ProjectError`'s 1-based (line, col)
/// converted to a zero-width UTF-16 offset); one in a sibling file can't be
/// underlined here, so it lands zero-width at the top with the file named in
/// the message.
fn load_error_json(active: &Path, active_src: &str, err: project::ProjectError) -> String {
    let (at, message) = if err.path == active {
        let byte = line_col_to_byte(active_src, err.line, err.col);
        (utf16_len(&active_src[..byte.min(active_src.len())]), err.message)
    } else {
        (0, format!("{}: {}", err.path.display(), err.message))
    };
    json!({
        "diagnostics": [{ "from": at, "to": at, "message": message, "severity": "error" }],
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
        // The one top-level def gets a signature lens titled `draw : …`.
        let lenses = out["lenses"].as_array().unwrap();
        assert!(
            lenses
                .iter()
                .any(|l| l["text"].as_str().unwrap().starts_with("draw ")),
            "expected a signature lens for `draw`: {out}"
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

    // Prime the last-good cache with a clean buffer, then complete `live` at its
    // end (UTF-16). Returns the parsed `{ items }` — the same one/two-step the
    // editor does (a valid doc parses, then the user types a `.`).
    fn complete_after(prime_src: &str, live: &str) -> Value {
        complete_json(prime_src, 0); // load cleanly → refresh the cache
        parse(&complete_json(live, utf16_len(live)))
    }

    fn labels(items: &Value) -> Vec<String> {
        items["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["label"].as_str().unwrap().to_string())
            .collect()
    }

    fn item<'a>(items: &'a Value, label: &str) -> &'a Value {
        items["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["label"] == label)
            .unwrap_or_else(|| panic!("no `{label}` in {:?}", labels(items)))
    }

    // (a) `Scene.` member completion offers prelude members with kind
    // "function". The buffer is a complete, valid program (`Scene.cube()`) with
    // the cursor right after the dot — the fresh path.
    #[test]
    fn scene_member_completion() {
        let src = "let d = () => Scene.cube()";
        let offset = utf16_len(&src[..src.find("Scene.").unwrap() + "Scene.".len()]);
        let out = parse(&complete_json(src, offset));
        let names = labels(&out);
        assert!(names.contains(&"cube".to_string()), "{names:?}");
        assert!(names.len() > 3, "expected many Scene members: {names:?}");
        assert_eq!(item(&out, "cube")["kind"], "function");
        assert!(item(&out, "cube")["detail"].is_string(), "{out}");
    }

    // (b) A BROKEN live buffer (trailing `.`) still completes via the last-good
    // cache — the cache was primed by a clean, unrelated buffer.
    #[test]
    fn broken_buffer_completes_from_cache() {
        let out = complete_after("let main = () => 1.0", "let x = 1.0\nlet s = Scene.");
        let names = labels(&out);
        assert!(names.contains(&"cube".to_string()), "{names:?}");
        assert_eq!(item(&out, "cube")["kind"], "function");
    }

    // (c) UTF-16 offset conversion: a non-ASCII prefix pushes byte offsets past
    // their UTF-16 counterparts, so a raw UTF-16-as-bytes offset would miss the
    // dot. The correct conversion lands member completion (`cube` present).
    #[test]
    fn completion_offset_is_utf16() {
        // 'é' (2 bytes) and '→' (3 bytes) before the completion point.
        let src = "let label = \"café→\"\nlet d = () => Scene.cube()";
        let dot = src.find("Scene.").unwrap() + "Scene.".len();
        let offset = utf16_len(&src[..dot]);
        // The byte offset must exceed the UTF-16 offset — proving conversion is
        // needed (a raw offset would land short of the dot).
        assert!(dot > offset, "expected multibyte prefix: byte {dot} vs utf16 {offset}");
        let out = parse(&complete_json(src, offset));
        assert!(labels(&out).contains(&"cube".to_string()), "{:?}", labels(&out));
    }

    // (d) A top-level partial `le` includes the keyword `let`.
    #[test]
    fn top_level_partial_offers_keyword() {
        let out = complete_after("let main = () => 1.0", "le");
        assert!(labels(&out).contains(&"let".to_string()), "{:?}", labels(&out));
        assert_eq!(item(&out, "let")["kind"], "keyword");
    }

    // An empty cache (nothing ever loaded) answers with empty items, never an
    // error, on a broken buffer.
    #[test]
    fn empty_cache_is_empty_items() {
        let out = parse(&complete_json("let s = Scene.", utf16_len("let s = Scene.")));
        assert!(out["items"].as_array().unwrap().is_empty(), "{out}");
    }

    // Resetting the cache clears the last-good project: a broken buffer for a
    // DIFFERENT program, completed after a reset, does NOT offer the previous
    // program's globals — the fix for switching sandbox examples and then
    // dot-completing a broken buffer. Without the reset the stale cache is
    // deliberately reused (the offset contract: candidates from a
    // possibly-stale project), which this test also documents.
    #[test]
    fn reset_clears_completion_cache() {
        // Program A defines a distinctive global `alpha`.
        let a = "let alpha = 1.0\nlet main = () => alpha";
        // A DIFFERENT, broken program (a bare top-level word `al` never parses),
        // so it can't refresh the cache — completion falls back to whatever the
        // cache holds. `al` is a top-level partial that matches A's `alpha`.
        let broken_b = "let beta = 2.0\nal";
        let at = utf16_len(broken_b);

        // Without reset: A's `alpha` leaks into the broken B buffer.
        complete_json(a, 0); // load A cleanly → cache A
        let leaked = parse(&complete_json(broken_b, at));
        assert!(
            labels(&leaked).contains(&"alpha".to_string()),
            "expected stale reuse of A's globals: {:?}",
            labels(&leaked)
        );

        // With reset: the cache is cleared, so A's globals are gone and the
        // broken buffer completes to nothing (empty cache → empty items).
        complete_json(a, 0); // re-cache A
        reset_cache();
        let cleared = parse(&complete_json(broken_b, at));
        assert!(
            !labels(&cleared).contains(&"alpha".to_string()),
            "cache not cleared — A's globals still offered: {:?}",
            labels(&cleared)
        );
        assert!(
            cleared["items"].as_array().unwrap().is_empty(),
            "empty cache → empty items: {cleared}"
        );
    }

    // --- Project-aware (multi-file) variants -----------------------------------

    // The IDE's two-file starter shape: game.fun references a sibling module.
    // Single-file analyze errors on the unknown `Palette`; the project variant
    // resolves it — the reason the `_project` API exists.
    const GAME: &str = "let draw = (model, tts: Float) =>\n  \
        Frame.create(Camera.lookAt(0.0, 0.0, -6.0, 0.0, 0.0, 0.0), \
        Scene.sphere() |> Scene.emissive(Color.rgb(0.15, 1.0, Palette.glow)))\n";
    const PALETTE: &str = "let glow = 0.85\nlet sky = 0.18\n";

    fn files_json(files: &[(&str, &str)]) -> String {
        Value::Array(
            files
                .iter()
                .map(|(path, source)| json!({ "path": path, "source": source }))
                .collect(),
        )
        .to_string()
    }

    #[test]
    fn project_analyze_resolves_sibling_modules() {
        // Project: the sibling links, the active file is clean, and every
        // lens/inlay is reported in ACTIVE-file-local offsets.
        let files = files_json(&[("game.fun", GAME), ("palette.fun", PALETTE)]);
        let out = parse(&analyze_project_json(&files, "game.fun"));
        assert_eq!(out["diagnostics"].as_array().unwrap().len(), 0, "{out}");
        for lens in out["lenses"].as_array().unwrap() {
            let from = lens["from"].as_u64().unwrap() as usize;
            assert!(from <= utf16_len(GAME), "lens outside the active file: {lens}");
        }

        // The observable difference from the single-file pass: the sibling
        // member RESOLVES. Single-file, `Palette` is tolerated but Unknown;
        // with the project, `Palette.glow` types as float.
        let member = GAME.find("Palette.glow").unwrap() + "Palette.".len();
        let offset = utf16_len(&GAME[..member]);
        let single = parse(&hover_json(GAME, offset));
        assert!(single["text"].as_str().unwrap().contains("Unknown"), "{single}");
        let project = parse(&hover_project_json(&files, "game.fun", offset));
        assert!(project["text"].as_str().unwrap().contains("float"), "{project}");
    }

    // Analyzing the SECOND file (base > 0) reports offsets local to it, and a
    // type error in it is flagged there — not filtered as foreign.
    #[test]
    fn project_analyze_reports_active_file_locally() {
        let bad_palette = "let glow = 1.0 + \"x\"\n";
        let files = files_json(&[("game.fun", "let unused = 1.0\n"), ("palette.fun", bad_palette)]);
        let out = parse(&analyze_project_json(&files, "palette.fun"));
        let diags = out["diagnostics"].as_array().unwrap();
        assert_eq!(diags.len(), 1, "{out}");
        let from = diags[0]["from"].as_u64().unwrap() as usize;
        let to = diags[0]["to"].as_u64().unwrap() as usize;
        assert!(from < to && to <= utf16_len(bad_palette), "span {from}..{to}: {out}");
    }

    // A PARSE failure in a sibling breaks the whole project pass; it surfaces
    // in the active file as a zero-width diagnostic at the top naming the file.
    #[test]
    fn project_sibling_parse_error_names_the_file() {
        let files = files_json(&[("game.fun", "let ok = 1.0\n"), ("palette.fun", "let broken = {\n")]);
        let out = parse(&analyze_project_json(&files, "game.fun"));
        let diags = out["diagnostics"].as_array().unwrap();
        assert_eq!(diags.len(), 1, "{out}");
        assert_eq!(diags[0]["from"], 0);
        assert_eq!(diags[0]["to"], 0);
        assert!(
            diags[0]["message"].as_str().unwrap().contains("palette.fun"),
            "{out}"
        );
    }

    // Hover in the second file (base > 0) answers in ITS local offsets: hover
    // a USE of `glow` and the returned span maps back onto that identifier.
    #[test]
    fn project_hover_in_second_file() {
        let palette = "let glow = 0.85\nlet bright = glow + 0.1\n";
        let files = files_json(&[("game.fun", "let unused = 1.0\n"), ("palette.fun", palette)]);
        let use_at = palette.rfind("glow").unwrap();
        let out = hover_project_json(&files, "palette.fun", utf16_len(&palette[..use_at]));
        assert!(!out.is_empty(), "expected a hover for `glow`");
        let v = parse(&out);
        let from = from_u16(palette, v["from"].as_u64().unwrap() as usize);
        let to = from_u16(palette, v["to"].as_u64().unwrap() as usize);
        assert_eq!(&palette[from..to], "glow");
        assert!(v["text"].as_str().unwrap().contains("float"), "{out}");
    }

    // Dot-completion on a SIBLING module offers its members.
    #[test]
    fn project_completion_offers_sibling_members() {
        reset_cache();
        let live = format!("{GAME}let more = Palette.");
        let files = files_json(&[("game.fun", GAME), ("palette.fun", PALETTE)]);
        // Prime the cache with the clean project…
        complete_project_json(&files, "game.fun", 0);
        // …then complete the live (broken: trailing dot) buffer.
        let live_files = files_json(&[("game.fun", &live), ("palette.fun", PALETTE)]);
        let out = parse(&complete_project_json(&live_files, "game.fun", utf16_len(&live)));
        let names = labels(&out);
        assert!(names.contains(&"glow".to_string()), "{names:?}");
        assert!(names.contains(&"sky".to_string()), "{names:?}");
    }

    // An `active` path not present in the file set is a caller bug: every
    // variant answers with its empty result — notably completion must NOT
    // fall back to entry-scope candidates against an empty buffer.
    #[test]
    fn project_unknown_active_is_empty() {
        reset_cache();
        let files = files_json(&[("game.fun", GAME), ("palette.fun", PALETTE)]);
        let out = parse(&analyze_project_json(&files, "typo.fun"));
        assert_eq!(out["diagnostics"].as_array().unwrap().len(), 0, "{out}");
        assert_eq!(out["lenses"].as_array().unwrap().len(), 0, "{out}");
        assert_eq!(hover_project_json(&files, "typo.fun", 4), "");
        assert_eq!(complete_project_json(&files, "typo.fun", 0), empty_completion());
    }

    // A malformed files payload degrades to the empty result, never a panic.
    #[test]
    fn project_malformed_payload_degrades() {
        let out = parse(&analyze_project_json("not json", "game.fun"));
        assert_eq!(out["diagnostics"].as_array().unwrap().len(), 0);
        assert_eq!(complete_project_json("[]", "game.fun", 0), empty_completion());
        assert_eq!(hover_project_json("[{\"path\": 1}]", "game.fun", 0), "");
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
