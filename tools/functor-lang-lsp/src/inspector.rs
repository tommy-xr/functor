//! The pure half of the paused-scene inspector: a trace document (the wire
//! contract in `docs`/`visual-debugger-implementation.md`) plus a file's source
//! text turned into **live-value inlay hints** and a **CodeLens execution
//! picker**. Like [`functor_lang::inlay`] / [`functor_lang::codelens`], this
//! module decides *content* (offsets, spans, labels) and the editor server
//! (`main.rs`) speaks the protocol — so it is unit-testable without an editor,
//! window, or GPU.
//!
//! It lives in the LSP tool crate, not the `functor-lang` crate: the trace is
//! JSON and the source-hash gate needs SHA-256, and the language crate is
//! zero-dependency by design (its `Cargo.toml` says so). The only thing it
//! borrows from `functor-lang` is [`functor_lang::Span`], a plain byte range.
//!
//! **Source-hash gate.** Every live hint and picker lens is withheld unless the
//! trace's recorded SHA-256 for that file matches the SHA-256 of the source the
//! server currently holds (open buffer, else disk). This is the "never wrong
//! values on wrong lines" invariant — an edited buffer silently shows no live
//! data (type hints are unaffected; they come from a different path).
//!
//! **Offset convention (a contract note).** The wire contract's binding
//! `start`/`end` are byte offsets **into that one file's text** (per-file
//! local), whereas the LSP's project loader works in project-wide offsets
//! (`file.base + local`). Live-hint offsets returned here are therefore already
//! file-local and the server renders them with no `base` adjustment; picker
//! spans, which come from the parsed module, stay project-wide and the server
//! localizes them with its existing machinery.

use std::collections::HashSet;

use functor_lang::Span;
use serde_json::Value;

/// One trace document — the last real frame's recorded entry-point
/// invocations, plus the per-file source hashes that gate them.
pub struct TraceDoc {
    pub sources: Vec<SourceHash>,
    pub invocations: Vec<Invocation>,
}

/// The SHA-256 (hex) of one project file's text as the runtime loaded it.
pub struct SourceHash {
    pub file: String,
    pub hash: String,
}

/// One recorded call of an entry point (`update`, `tick`, …) within the frame.
pub struct Invocation {
    pub entry: String,
    /// 0-based order within the frame for this entry name.
    pub index: usize,
    /// Total invocations of this entry this frame.
    pub count: usize,
    /// Human display string of *why* it ran (opaque to us).
    pub provenance: String,
    /// True for `--ghost` forward-projection dry-runs; excluded from display.
    pub ghost: bool,
    pub bindings: Vec<Binding>,
}

/// One binding site's last recorded value.
pub struct Binding {
    pub name: String,
    /// Trace-relative file name (e.g. `game.fun`).
    pub file: String,
    /// Byte offsets **into that file's own text** (per-file local).
    pub start: usize,
    pub end: usize,
    /// `Display` text of the (last) bound value.
    pub value: String,
    /// Times this site bound during the invocation (loops > 1); `value` is last.
    pub count: usize,
}

impl TraceDoc {
    /// Parse a trace document from the wire-contract JSON. `None` if the shape
    /// is unrecognizable (missing `sources`/`invocations` arrays); individual
    /// malformed entries are skipped rather than failing the whole doc.
    pub fn from_json(v: &Value) -> Option<TraceDoc> {
        let sources = v["sources"]
            .as_array()?
            .iter()
            .filter_map(|s| {
                Some(SourceHash {
                    file: s["file"].as_str()?.to_string(),
                    hash: s["hash"].as_str()?.to_string(),
                })
            })
            .collect();
        let invocations = v["invocations"]
            .as_array()?
            .iter()
            .filter_map(Invocation::from_json)
            .collect();
        Some(TraceDoc {
            sources,
            invocations,
        })
    }
}

impl Invocation {
    fn from_json(v: &Value) -> Option<Invocation> {
        let bindings = v["bindings"]
            .as_array()
            .map(|arr| arr.iter().filter_map(Binding::from_json).collect())
            .unwrap_or_default();
        Some(Invocation {
            entry: v["entry"].as_str()?.to_string(),
            index: v["index"].as_u64().unwrap_or(0) as usize,
            count: v["count"].as_u64().unwrap_or(1).max(1) as usize,
            provenance: v["provenance"].as_str().unwrap_or("").to_string(),
            ghost: v["ghost"].as_bool().unwrap_or(false),
            bindings,
        })
    }
}

impl Binding {
    fn from_json(v: &Value) -> Option<Binding> {
        Some(Binding {
            name: v["name"].as_str()?.to_string(),
            file: v["file"].as_str()?.to_string(),
            start: v["start"].as_u64()? as usize,
            end: v["end"].as_u64()? as usize,
            value: v["value"].as_str().unwrap_or("").to_string(),
            count: v["count"].as_u64().unwrap_or(1).max(1) as usize,
        })
    }
}

/// One live-value hint: render `label` (a leading `= …` string) at byte
/// `offset`, which is **file-local** (see the module offset note).
pub struct LiveHint {
    pub offset: usize,
    pub label: String,
}

/// One execution-picker lens on an entry-point def. `span` is the def's span in
/// whatever coordinate space the caller passed (the server passes project-wide
/// module spans and localizes them itself). The cycle command carries
/// `[file, entry, current_index]`.
pub struct PickerLens {
    pub span: Span,
    pub title: String,
    pub entry: String,
    pub file: String,
    /// 0-based selected execution, already reduced mod `count`.
    pub current_index: usize,
}

/// Whether the server's `source` for `file_name` matches the trace's recorded
/// hash — the gate for showing any live data. A file with no recorded hash
/// fails closed (we can't verify, so we show nothing).
pub fn source_matches(trace: &TraceDoc, file_name: &str, source: &str) -> bool {
    match trace.sources.iter().find(|s| s.file == file_name) {
        Some(sh) => sha256_hex(source.as_bytes()) == sh.hash,
        None => false,
    }
}

/// The selected invocation for `entry`: `(invocation, selected_index, count)`.
/// `count` is the trace-reported total; the raw selection is reduced mod
/// `count`, and the invocation whose `index` equals that selection is chosen
/// (falling back to the first). Ghost invocations are excluded. `None` when the
/// entry has no non-ghost invocations.
fn selected_invocation<'a>(
    trace: &'a TraceDoc,
    entry: &str,
    selected: &dyn Fn(&str) -> usize,
) -> Option<(&'a Invocation, usize, usize)> {
    let group: Vec<&Invocation> = trace
        .invocations
        .iter()
        .filter(|i| i.entry == entry && !i.ghost)
        .collect();
    let first = *group.first()?;
    let count = group.iter().map(|i| i.count).max().unwrap_or(1).max(1);
    let sel = selected(entry) % count;
    let inv = group.iter().find(|i| i.index == sel).copied().unwrap_or(first);
    Some((inv, sel, count))
}

/// Live-value inlay hints for `file_name`'s `source`, from the selected
/// execution of each entry point. Empty (silently) when the source hash does
/// not match the trace. Hints are deduplicated by offset so a helper shared
/// across two selected invocations doesn't double up.
pub fn live_hints(
    trace: &TraceDoc,
    file_name: &str,
    source: &str,
    selected: &dyn Fn(&str) -> usize,
) -> Vec<LiveHint> {
    if !source_matches(trace, file_name, source) {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for entry in distinct_entries(trace) {
        let Some((inv, _, _)) = selected_invocation(trace, &entry, selected) else {
            continue;
        };
        for b in &inv.bindings {
            let offset = hint_offset(source, b);
            if b.file != file_name || !seen.insert(offset) {
                continue;
            }
            let label = if b.count > 1 {
                format!("= {} (×{})", b.value, b.count)
            } else {
                format!("= {}", b.value)
            };
            out.push(LiveHint { offset, label });
        }
    }
    out
}

/// Execution-picker lenses for the entry-point defs in `file_name`. `entry_defs`
/// is `(def_name, def_span)` for the file's top-level defs (the server pulls
/// these from the parsed module); a def gets a lens iff the trace has an
/// invocation for its name. Empty (silently) on a source-hash mismatch.
pub fn picker_lenses(
    trace: &TraceDoc,
    file_name: &str,
    source: &str,
    entry_defs: &[(String, Span)],
    selected: &dyn Fn(&str) -> usize,
) -> Vec<PickerLens> {
    if !source_matches(trace, file_name, source) {
        return Vec::new();
    }
    entry_defs
        .iter()
        .filter_map(|(name, span)| {
            let (inv, sel, count) = selected_invocation(trace, name, selected)?;
            let title = if count == 1 {
                format!("{name} — 1 execution")
            } else {
                format!("{name} — execution {}/{} ▸ [{}]", sel + 1, count, inv.provenance)
            };
            Some(PickerLens {
                span: *span,
                title,
                entry: name.clone(),
                file: file_name.to_string(),
                current_index: sel,
            })
        })
        .collect()
}

/// Where to render a binding's `= value` hint: **just after the binder name**.
///
/// Per the PR1 contract, a binding's span is name-precise only for lambda/match
/// binders; a `let` binder's span is the whole `let [mut] name =` **region**, so
/// its `end` sits after the `=`, not after the name. We therefore locate the
/// `name` text within the span (`rfind`, so the binder — not a `let`/`mut`
/// keyword — wins) and place the hint right after it, falling back to the span
/// end when the source slice is unusable or the name isn't found.
fn hint_offset(source: &str, b: &Binding) -> usize {
    match source.get(b.start..b.end) {
        Some(region) => match region.rfind(&b.name) {
            Some(i) => b.start + i + b.name.len(),
            None => b.end,
        },
        None => b.end,
    }
}

/// Distinct entry names in first-seen order.
fn distinct_entries(trace: &TraceDoc) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for inv in &trace.invocations {
        if !inv.ghost && seen.insert(inv.entry.clone()) {
            out.push(inv.entry.clone());
        }
    }
    out
}

/// The number of executions the trace holds for `entry` (its reported `count`),
/// or 0 if the entry is absent. Used by the server to cycle the picker index.
pub fn execution_count(trace: &TraceDoc, entry: &str) -> usize {
    trace
        .invocations
        .iter()
        .filter(|i| i.entry == entry && !i.ghost)
        .map(|i| i.count)
        .max()
        .unwrap_or(0)
}

/// SHA-256 of `bytes` as lowercase hex. Hand-rolled (FIPS 180-4) to keep the
/// LSP crate's dependency set at `serde_json` only — matching the crate's
/// "deliberately tiny" charter. Verified against the standard `"abc"` vector in
/// tests.
pub fn sha256_hex(bytes: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    // Pad: append 0x80, then zeros to 56 mod 64, then the 64-bit bit length.
    let mut msg = bytes.to_vec();
    let bit_len = (bytes.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, word) in w.iter_mut().enumerate().take(16) {
            let j = i * 4;
            *word = u32::from_be_bytes([chunk[j], chunk[j + 1], chunk[j + 2], chunk[j + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7]
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v[7] = v[6];
            v[6] = v[5];
            v[5] = v[4];
            v[4] = v[3].wrapping_add(t1);
            v[3] = v[2];
            v[2] = v[1];
            v[1] = v[0];
            v[0] = t1.wrapping_add(t2);
        }
        for (hi, vi) in h.iter_mut().zip(v.iter()) {
            *hi = hi.wrapping_add(*vi);
        }
    }

    let mut hex = String::with_capacity(64);
    for word in h {
        hex.push_str(&format!("{word:08x}"));
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // A trace whose one file's hash matches SHA-256("<source>").
    fn trace_for(source: &str, invocations: Value) -> TraceDoc {
        let doc = json!({
            "paused": true,
            "sources": [ { "file": "game.fun", "hash": sha256_hex(source.as_bytes()) } ],
            "invocations": invocations,
        });
        TraceDoc::from_json(&doc).expect("parse trace")
    }

    fn no_selection() -> impl Fn(&str) -> usize {
        |_| 0
    }

    #[test]
    fn sha256_matches_the_standard_abc_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn live_hints_render_value_at_binding_end() {
        let src = "let update = (model, msg) =>\n  let velocity = model in\n  velocity\n";
        let start = src.find("velocity").unwrap();
        let end = start + "velocity".len();
        let trace = trace_for(
            src,
            json!([{
                "entry": "update", "index": 0, "count": 1, "provenance": "tick dt=0.016",
                "ghost": false, "result": "0",
                "bindings": [{
                    "name": "velocity", "file": "game.fun",
                    "start": start, "end": end,
                    "value": "{ x = 0.0, y = -9.8 }", "count": 1
                }]
            }]),
        );
        let hints = live_hints(&trace, "game.fun", src, &no_selection());
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].offset, end);
        assert_eq!(hints[0].label, "= { x = 0.0, y = -9.8 }");
    }

    #[test]
    fn live_hints_place_after_name_for_a_region_shaped_span() {
        // PR1's `let` binder spans cover the `let name =` REGION, not just the
        // name. The hint must still land right after the binder name.
        let src = "let update = (model, msg) =>\n  let velocity = model in\n  velocity\n";
        let region_start = src.match_indices("let").nth(1).unwrap().0; // inner `let`
        let name_pos = region_start + src[region_start..].find("velocity").unwrap();
        let value_pos = name_pos + src[name_pos..].find("model").unwrap(); // `= model`
        let expected = name_pos + "velocity".len();
        let trace = trace_for(
            src,
            json!([{
                "entry": "update", "index": 0, "count": 1, "provenance": "p",
                "ghost": false, "result": "0",
                "bindings": [{
                    "name": "velocity", "file": "game.fun",
                    "start": region_start, "end": value_pos,
                    "value": "{ x = 0.0, y = -9.8 }", "count": 1
                }]
            }]),
        );
        let hints = live_hints(&trace, "game.fun", src, &no_selection());
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].offset, expected);
        assert_eq!(&src[..hints[0].offset].chars().rev().take(8).collect::<String>(), "yticolev");
    }

    #[test]
    fn live_hints_show_loop_count_when_greater_than_one() {
        let src = "let tick = (m, dt, tts) =>\n  let acc = m in\n  acc\n";
        let start = src.find("acc").unwrap();
        let end = start + "acc".len();
        let trace = trace_for(
            src,
            json!([{
                "entry": "tick", "index": 0, "count": 1, "provenance": "tick dt=0.016",
                "ghost": false, "result": "0",
                "bindings": [{
                    "name": "acc", "file": "game.fun", "start": start, "end": end,
                    "value": "42", "count": 8
                }]
            }]),
        );
        let hints = live_hints(&trace, "game.fun", src, &no_selection());
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].label, "= 42 (×8)");
    }

    #[test]
    fn source_hash_mismatch_yields_no_live_data() {
        let src = "let update = (model, msg) =>\n  let velocity = model in\n  velocity\n";
        let start = src.find("velocity").unwrap();
        let end = start + "velocity".len();
        let mut trace = trace_for(
            src,
            json!([{
                "entry": "update", "index": 0, "count": 1, "provenance": "p",
                "ghost": false, "result": "0",
                "bindings": [{ "name":"velocity","file":"game.fun","start":start,"end":end,"value":"1","count":1 }]
            }]),
        );
        // Corrupt the recorded hash so the gate closes.
        trace.sources[0].hash = "deadbeef".to_string();
        assert!(live_hints(&trace, "game.fun", src, &no_selection()).is_empty());
        let defs = vec![("update".to_string(), Span::new(4, 10))];
        assert!(picker_lenses(&trace, "game.fun", src, &defs, &no_selection()).is_empty());
    }

    #[test]
    fn picker_lens_multi_execution_shows_index_and_provenance() {
        let src = "let update = (model, msg) => model\n";
        let invs = json!([
            { "entry":"update","index":0,"count":3,"provenance":"subscription: Tick","ghost":false,"result":"0","bindings":[] },
            { "entry":"update","index":1,"count":3,"provenance":"effect result: Pong","ghost":false,"result":"0","bindings":[] },
            { "entry":"update","index":2,"count":3,"provenance":"input: Space down","ghost":false,"result":"0","bindings":[] }
        ]);
        let trace = trace_for(src, invs);
        let defs = vec![("update".to_string(), Span::new(0, 34))];
        // Select execution index 1.
        let selected = |e: &str| if e == "update" { 1 } else { 0 };
        let lenses = picker_lenses(&trace, "game.fun", src, &defs, &selected);
        assert_eq!(lenses.len(), 1);
        assert_eq!(lenses[0].title, "update — execution 2/3 ▸ [effect result: Pong]");
        assert_eq!(lenses[0].current_index, 1);
        assert_eq!(lenses[0].entry, "update");
    }

    #[test]
    fn picker_lens_single_execution_reads_one_execution() {
        let src = "let tick = (m, dt, tts) => m\n";
        let trace = trace_for(
            src,
            json!([{ "entry":"tick","index":0,"count":1,"provenance":"tick dt=0.016","ghost":false,"result":"0","bindings":[] }]),
        );
        let defs = vec![("tick".to_string(), Span::new(0, 28))];
        let lenses = picker_lenses(&trace, "game.fun", src, &defs, &no_selection());
        assert_eq!(lenses.len(), 1);
        assert_eq!(lenses[0].title, "tick — 1 execution");
    }

    #[test]
    fn selection_beyond_count_wraps_modulo() {
        let src = "let update = (model, msg) => model\n";
        let invs = json!([
            { "entry":"update","index":0,"count":2,"provenance":"a","ghost":false,"result":"0","bindings":[] },
            { "entry":"update","index":1,"count":2,"provenance":"b","ghost":false,"result":"0","bindings":[] }
        ]);
        let trace = trace_for(src, invs);
        let defs = vec![("update".to_string(), Span::new(0, 34))];
        // Raw index 5 % 2 == 1 → the "b" execution.
        let selected = |_: &str| 5usize;
        let lenses = picker_lenses(&trace, "game.fun", src, &defs, &selected);
        assert_eq!(lenses[0].title, "update — execution 2/2 ▸ [b]");
        assert_eq!(lenses[0].current_index, 1);
    }

    #[test]
    fn ghost_invocations_are_ignored() {
        let src = "let update = (model, msg) => model\n";
        let trace = trace_for(
            src,
            json!([{ "entry":"update","index":0,"count":1,"provenance":"ghost","ghost":true,"result":"0","bindings":[] }]),
        );
        let defs = vec![("update".to_string(), Span::new(0, 34))];
        assert!(picker_lenses(&trace, "game.fun", src, &defs, &no_selection()).is_empty());
        assert_eq!(execution_count(&trace, "update"), 0);
    }
}
