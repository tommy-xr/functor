//! The paused-scene inspector's wire-contract trace builder (visual-debugger
//! PR2/2b) — shared by both shells' producers.
//!
//! During normal play a producer records nothing and renders no Display text
//! (the hard perf requirement); it only keeps a cheap replay journal of the
//! model-updating calls (`functor_lang_producer::JournalEntry`). On pause it
//! hands that journal, the loaded source hashes, and the live `Session` to
//! [`build_trace_doc`], which replays each call through
//! `Session::call_recorded` (the PR1 recorder) and assembles the wire JSON the
//! LSP consumes (`docs/visual-debugger`). Entry points are pure functions of
//! their args and effects are plain data (only the drain performs), so replay
//! is exact and side-effect-free.
//!
//! The desktop producer serves this over `GET /trace`; the web producer
//! publishes it through the `functor_lang_inspector_trace*` wasm exports for the
//! VS Code live-preview to relay. Keeping the assembly here means one tested
//! copy and a wire contract that cannot drift between the two shells.

use functor_lang::project::SourceMap;
use functor_lang::{Session, Span};

use crate::functor_lang_prelude::FunctorHost;
use crate::functor_lang_producer::JournalEntry;

/// One project source file's inspector metadata: its wire name, its base offset
/// in the project-wide span space, its length, and the sha256 of the text the
/// runtime loaded. Computed once per load (not per frame), so a binding span
/// maps to `(file, local offset)` and the wire `sources` gates on a content
/// hash without re-reading files.
pub struct InspectorSource {
    pub file: String,
    pub base: usize,
    pub len: usize,
    pub hash: String,
}

/// Build the per-file inspector metadata from a loaded [`SourceMap`]: the REAL
/// project `.fun` files only (skip the injected prelude `.funi` interfaces and
/// the `<builtin>/Net.fun` module — a game binding never lands in them, and they
/// aren't editor documents the LSP gates on).
pub fn inspector_sources(sources: &SourceMap) -> Vec<InspectorSource> {
    use sha2::{Digest, Sha256};
    sources
        .files()
        .iter()
        .filter(|f| !f.interface && !f.path.to_string_lossy().starts_with('<'))
        .map(|f| {
            let file = f
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| f.path.to_string_lossy().into_owned());
            let hash = format!("{:x}", Sha256::digest(f.src.as_bytes()));
            InspectorSource {
                file,
                base: f.base,
                len: f.src.len(),
                hash,
            }
        })
        .collect()
}

/// Map a project-wide binding span to `(file, local start, local end)` for the
/// wire contract, using the per-file base offsets. `None` if the span falls
/// outside a real project file (a prelude/builtin span — a game binding never
/// does).
fn span_to_file(sources: &[InspectorSource], span: Span) -> Option<(String, usize, usize)> {
    let src = sources
        .iter()
        .filter(|s| s.base <= span.start)
        .max_by_key(|s| s.base)?;
    if span.start > src.base + src.len {
        return None;
    }
    Some((
        src.file.clone(),
        span.start - src.base,
        span.end.saturating_sub(src.base),
    ))
}

/// The wire `sources` array — `{ file, hash }` per loaded project file.
fn sources_json(sources: &[InspectorSource]) -> Vec<serde_json::Value> {
    sources
        .iter()
        .map(|s| serde_json::json!({ "file": s.file, "hash": s.hash }))
        .collect()
}

/// Replay the last real frame's journal into the wire-contract `invocations`.
/// Each journaled call is re-run through `Session::call_recorded` — entry points
/// are pure functions of their args, so the record is exact, and effects are
/// plain data (nothing performs), so replay is side-effect-free. `index`/`count`
/// frame each call within its entry name; binding spans map to `(file, local
/// offsets)`.
fn build_invocations(
    journal: &[JournalEntry],
    sources: &[InspectorSource],
    session: &Session,
) -> Vec<serde_json::Value> {
    use std::collections::HashMap;
    // Replay FIRST, then frame: `index`/`count` are computed over the
    // invocations actually EMITTED, so the array is always consistent with its
    // own counts (the LSP picker mod-cycles on `count`). A call that succeeded
    // live, replayed pure, should not fail — skip one defensively rather than
    // abort the whole trace if it somehow does.
    let mut replayed = Vec::with_capacity(journal.len());
    for e in journal {
        if let Ok((_discard, inv)) = session.call_recorded(e.entry, e.args.clone(), &mut FunctorHost)
        {
            replayed.push((e, inv));
        }
    }
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for (e, _) in &replayed {
        *counts.entry(e.entry).or_default() += 1;
    }
    let mut seen: HashMap<&str, usize> = HashMap::new();
    let mut out = Vec::with_capacity(replayed.len());
    for (e, inv) in &replayed {
        let index = {
            let c = seen.entry(e.entry).or_default();
            let i = *c;
            *c += 1;
            i
        };
        let bindings: Vec<serde_json::Value> = inv
            .bindings
            .iter()
            .filter_map(|b| {
                span_to_file(sources, b.span).map(|(file, start, end)| {
                    serde_json::json!({
                        "name": b.name,
                        "file": file,
                        "start": start,
                        "end": end,
                        "value": b.value,
                        "count": b.count,
                    })
                })
            })
            .collect();
        out.push(serde_json::json!({
            "entry": e.entry,
            "index": index,
            "count": counts[e.entry],
            "provenance": e.provenance.render(&e.args),
            "ghost": false,
            "result": inv.result,
            "truncated": inv.truncated,
            "bindings": bindings,
        }));
    }
    out
}

/// Assemble the wire-contract trace document (`docs/visual-debugger`).
///
/// When NOT paused this is the byte-stable stub `{"paused":false,"sources":[…],
/// "invocations":[]}` — no `frame`/`tts` (they change every frame, and the LSP's
/// idle poll dedups on the doc bytes, so the unpaused doc must stay identical
/// while the sources are unchanged). When paused it carries `frame`/`tts` and
/// replays the journal into `invocations`. The caller owns any caching (building
/// only on a pause-state or paused-frame change) — this function is pure.
pub fn build_trace_doc(
    paused: bool,
    frame: u64,
    tts: f64,
    sources: &[InspectorSource],
    journal: &[JournalEntry],
    session: &Session,
) -> String {
    if !paused {
        return serde_json::json!({
            "paused": false,
            "sources": sources_json(sources),
            "invocations": [],
        })
        .to_string();
    }
    serde_json::json!({
        "frame": frame,
        "tts": tts,
        "paused": true,
        "sources": sources_json(sources),
        "invocations": build_invocations(journal, sources, session),
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::functor_lang_producer::Provenance;
    use functor_lang::project::load_single_source;
    use functor_lang::Value;
    use std::rc::Rc;

    // Two entry points with param binders + a `let` binder each — no engine
    // prelude needed (we never call `draw`), so this builds and replays under
    // the plain host. `input`'s primitive args let us construct the journal
    // directly (no ADT message value to synthesize).
    const SRC: &str = "\
        type Model = { n: Float }\n\
        let init = { n: 0.0 }\n\
        let tick = (m: Model, dt: Float, tts: Float) =>\n\
          let bumped = m.n + dt in\n\
          { n: bumped }\n\
        let input = (m: Model, key: Key.t, isDown: Bool) =>\n\
          let step = 1.0 in\n\
          { n: m.n + step }\n";

    fn session_and_sources() -> (Session, Value, Vec<InspectorSource>) {
        let project =
            load_single_source("game", SRC).unwrap_or_else(|e| panic!("load: {}", e.render()));
        let session = Session::load(&project.module, &mut FunctorHost)
            .unwrap_or_else(|f| panic!("session: {}", f.error.message));
        let model = session.global("init").expect("init");
        let sources = inspector_sources(&project.sources);
        (session, model, sources)
    }

    #[test]
    fn unpaused_doc_is_the_byte_stable_stub() {
        let (session, _model, sources) = session_and_sources();
        // Byte-identical regardless of the frame/tts args — the M1 contract: the
        // LSP dedups idle polls on the doc bytes, so the unpaused doc must not
        // carry per-frame fields.
        let a = build_trace_doc(false, 0, 0.0, &sources, &[], &session);
        let b = build_trace_doc(false, 7, 4.5, &sources, &[], &session);
        assert_eq!(a, b);
        let doc: serde_json::Value = serde_json::from_str(&a).unwrap();
        assert_eq!(doc["paused"], serde_json::json!(false));
        assert_eq!(doc["invocations"].as_array().unwrap().len(), 0);
        assert!(doc.get("frame").is_none(), "no frame while playing");
        assert!(doc.get("tts").is_none(), "no tts while playing");
        assert_eq!(doc["sources"][0]["file"], serde_json::json!("game.fun"));
        assert_eq!(doc["sources"][0]["hash"].as_str().unwrap().len(), 64);
    }

    #[test]
    fn paused_doc_replays_journal_into_wire_contract_invocations() {
        let (session, model, sources) = session_and_sources();
        // Two ticks + one input: proves index/count framing and both provenance
        // kinds, with real args that replay to real binding values.
        let journal = vec![
            JournalEntry {
                entry: "tick",
                args: vec![model.clone(), Value::Number(0.2), Value::Number(1.1)],
                provenance: Provenance::Tick,
            },
            JournalEntry {
                entry: "tick",
                args: vec![model.clone(), Value::Number(0.5), Value::Number(1.6)],
                provenance: Provenance::Tick,
            },
            JournalEntry {
                entry: "input",
                args: vec![
                    model.clone(),
                    Value::Variant {
                        ctor: Rc::from("Key.W"),
                        args: Rc::new(Vec::new()),
                    },
                    Value::Bool(true),
                ],
                provenance: Provenance::Input,
            },
        ];
        let doc: serde_json::Value =
            serde_json::from_str(&build_trace_doc(true, 3, 1.6, &sources, &journal, &session))
                .unwrap();

        assert_eq!(doc["paused"], serde_json::json!(true));
        assert_eq!(doc["frame"], serde_json::json!(3));
        assert_eq!(doc["tts"], serde_json::json!(1.6));
        assert_eq!(doc["sources"][0]["file"], serde_json::json!("game.fun"));
        assert_eq!(doc["sources"][0]["hash"].as_str().unwrap().len(), 64);

        let invs = doc["invocations"].as_array().unwrap();
        assert_eq!(invs.len(), 3);

        // The two ticks: 0-based index within the entry, shared count, dt-tagged
        // provenance rendered at f32 precision.
        let ticks: Vec<_> = invs.iter().filter(|i| i["entry"] == "tick").collect();
        assert_eq!(ticks.len(), 2);
        assert_eq!(ticks[0]["index"], serde_json::json!(0));
        assert_eq!(ticks[1]["index"], serde_json::json!(1));
        assert!(ticks.iter().all(|t| t["count"] == serde_json::json!(2)));
        assert_eq!(ticks[0]["provenance"], serde_json::json!("tick dt=0.2"));
        assert_eq!(ticks[0]["ghost"], serde_json::json!(false));

        // Bindings: the params + the `let bumped` site, in game.fun, at LOCAL
        // byte offsets into the file text, carrying the replayed values.
        let binds = ticks[0]["bindings"].as_array().unwrap();
        assert!(
            binds.iter().any(|b| b["name"] == "bumped"),
            "the let binder is recorded: {binds:#?}"
        );
        assert!(binds.iter().all(|b| b["file"] == "game.fun"));
        for b in binds {
            let start = b["start"].as_u64().unwrap() as usize;
            let end = b["end"].as_u64().unwrap() as usize;
            assert!(start <= end && end <= SRC.len(), "LOCAL offset into the file");
            assert!(b["value"].is_string());
        }
        // `dt` binds to the exact arg we passed — a real value.
        let dt = binds.iter().find(|b| b["name"] == "dt").expect("dt binding");
        assert_eq!(dt["value"], serde_json::json!("0.2"));

        // The input invocation: its own count, key/down provenance, real bindings.
        let input = invs.iter().find(|i| i["entry"] == "input").unwrap();
        assert_eq!(input["index"], serde_json::json!(0));
        assert_eq!(input["count"], serde_json::json!(1));
        assert_eq!(input["provenance"], serde_json::json!("input: Key.W down"));
        assert!(!input["bindings"].as_array().unwrap().is_empty());
    }
}
