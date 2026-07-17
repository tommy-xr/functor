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
use functor_lang::value::Value;
use functor_lang::{Session, Span};

use crate::functor_lang_prelude::FunctorHost;
use crate::functor_lang_producer::{JournalEntry, Provenance};

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

/// Replay the last real frame's journal into the wire-contract `invocations`,
/// plus one synthesized `draw` invocation against the frozen model when the
/// caller supplies its args (draw is pure and never journaled during play —
/// replaying it once at trace-build time is exact and free). Each call is
/// re-run through `Session::call_recorded` — entry points are pure functions
/// of their args, so the record is exact, and effects are plain data (nothing
/// performs), so replay is side-effect-free. `index`/`count` frame each call
/// within its entry name; binding spans map to `(file, local offsets)`.
fn build_invocations(
    journal: &[JournalEntry],
    draw_args: Option<&[Value]>,
    sources: &[InspectorSource],
    session: &Session,
) -> (Vec<serde_json::Value>, Vec<usize>) {
    use std::collections::HashMap;
    // Replay FIRST, then frame: `index`/`count` are computed over the
    // invocations actually EMITTED, so the array is always consistent with its
    // own counts (the LSP picker mod-cycles on `count`). A call that succeeded
    // live, replayed pure, should not fail — skip one defensively rather than
    // abort the whole trace if it somehow does. Draw comes last (the frame
    // order: update → tick → draw); a program without a `draw` def just skips.
    // Replay must be OBSERVATION-ONLY beyond effect inertness (effects are
    // plain data; only the drain performs): suppress `Debug.log` re-emission
    // (its lines already emitted live at frame time — re-emitting would
    // duplicate them on every trace build) and bracket the UI handler table
    // (a replayed `Ui.*` call would append handlers the next real `ui` pass
    // takes as its own, shifting every slot).
    let _mute = functor_lang::suppress_trace();
    let saved_handlers = crate::functor_lang_prelude::take_ui_handlers();
    let mut replayed = Vec::with_capacity(journal.len() + 1);
    for e in journal {
        if let Ok((_discard, inv)) = session.call_recorded(e.entry, e.args.clone(), &mut FunctorHost)
        {
            replayed.push((e.entry, Provenance::render(&e.provenance, &e.args), inv));
        }
    }
    if let Some(args) = draw_args {
        if let Ok((_discard, inv)) = session.call_recorded("draw", args.to_vec(), &mut FunctorHost) {
            replayed.push(("draw", Provenance::Draw.render(args), inv));
        }
    }
    let _replay_pushed = crate::functor_lang_prelude::take_ui_handlers();
    crate::functor_lang_prelude::restore_ui_handlers(saved_handlers);
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for (entry, _, _) in &replayed {
        *counts.entry(entry).or_default() += 1;
    }
    let mut seen: HashMap<&str, usize> = HashMap::new();
    let mut out = Vec::with_capacity(replayed.len());
    for (entry, provenance, inv) in &replayed {
        let index = {
            let c = seen.entry(entry).or_default();
            let i = *c;
            *c += 1;
            i
        };
        let bindings: Vec<serde_json::Value> = inv
            .bindings
            .iter()
            .filter_map(|b| {
                span_to_file(sources, b.span).map(|(file, start, end)| {
                    let mut o = serde_json::json!({
                        "name": b.name,
                        "file": file,
                        "start": start,
                        "end": end,
                        "value": b.value,
                        "preview": b.preview,
                        "kind": kind_str(b.kind),
                        "site": site_str(b.site),
                        "count": b.count,
                    });
                    // Numeric loop sites carry their observed range — the
                    // editors render `= min…max (×N)` for multi-hit sites.
                    if let (Some(min), Some(max)) = (b.min, b.max) {
                        o["min"] = serde_json::json!(min);
                        o["max"] = serde_json::json!(max);
                    }
                    o
                })
            })
            .collect();
        out.push(serde_json::json!({
            "entry": entry,
            "index": index,
            "count": counts[entry],
            "provenance": provenance,
            "ghost": false,
            "result": inv.result,
            "result_preview": inv.result_preview,
            "truncated": inv.truncated,
            "bindings": bindings,
        }));
    }
    // The paused frame's coverage rides along free (every values replay
    // collected it, draw included) — merged and deduped for coverage_json.
    let mut frame_coverage: Vec<usize> = replayed
        .iter()
        .flat_map(|(_, _, inv)| inv.coverage.iter().copied())
        .collect();
    frame_coverage.sort_unstable();
    frame_coverage.dedup();
    (out, frame_coverage)
}

/// The wire strings for the recorder's site/kind enums.
fn kind_str(kind: functor_lang::RecordedKind) -> &'static str {
    match kind {
        functor_lang::RecordedKind::Primitive => "primitive",
        functor_lang::RecordedKind::Composite => "composite",
    }
}

fn site_str(site: functor_lang::RecordedSite) -> &'static str {
    match site {
        functor_lang::RecordedSite::Binder => "binder",
        functor_lang::RecordedSite::Ref => "ref",
    }
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
    draw_args: Option<&[Value]>,
    session: &Session,
) -> String {
    build_trace_doc_with_coverage(
        paused, frame, tts, sources, journal, draw_args, &[], &[], session,
    )
}

/// [`build_trace_doc`] plus the recency-gutter data (visual-debugger v2 PR4):
///
/// - `ring` is a window of recent frames' journals `(frame, entries)` — the
///   coverage source. Each ringed frame replays COVERAGE-ONLY
///   (`Session::call_covered`: no Display rendering), yielding
///   `"coverage": { file: [ { "start": N, "frames": [-2, 0, 3] } ] }` —
///   per-file span starts with the sorted frame OFFSETS (ringFrame − frame)
///   they executed on. Offsets can be positive when the paused frame is
///   scrubbed behind the live head (the gutter's "ran in a frame after").
///   The paused frame's own coverage comes free from the values replay
///   (including draw); other frames' draw passes are not replayed (their
///   models aren't journaled).
/// - `runnable` is the static could-run set
///   (`functor_lang::coverage::runnable_offsets`, project-wide starts),
///   emitted per file as `"runnable": { file: [starts] }` — how a consumer
///   tells "runnable but never ran" (dark) from "not code at all".
#[allow(clippy::too_many_arguments)]
pub fn build_trace_doc_with_coverage(
    paused: bool,
    frame: u64,
    tts: f64,
    sources: &[InspectorSource],
    journal: &[JournalEntry],
    draw_args: Option<&[Value]>,
    ring: &[(u64, Vec<JournalEntry>)],
    runnable: &[usize],
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
    let (invocations, paused_coverage) = build_invocations(journal, draw_args, sources, session);
    serde_json::json!({
        "frame": frame,
        "tts": tts,
        "paused": true,
        "sources": sources_json(sources),
        "invocations": invocations,
        "coverage": coverage_json(frame, paused_coverage, ring, sources, session),
        "runnable": runnable_json(runnable, sources),
    })
    .to_string()
}

/// How many recent frames' journals the shells retain for coverage (the
/// recency gutter's ±window). Entries hold Rc-cloned args, so a frame costs
/// little; replay is lazy (pause-time only) and coverage-only per frame.
pub const COVERAGE_RING_FRAMES: usize = 120;

/// Merge the paused frame's coverage (already collected by the values
/// replay) with coverage-only replays of every OTHER ringed frame, grouped
/// per file with frame offsets relative to `frame`.
fn coverage_json(
    frame: u64,
    paused_coverage: Vec<usize>,
    ring: &[(u64, Vec<JournalEntry>)],
    sources: &[InspectorSource],
    session: &Session,
) -> serde_json::Value {
    use std::collections::{BTreeMap, BTreeSet};
    // The same observation-only bracket as the values replay: a windowful of
    // frames with a Debug.log in tick would otherwise re-emit ~120× per
    // trace build, and a Ui.* call would leak handlers into the next real
    // ui pass. (Sequential with build_invocations' own bracket, not nested.)
    let _mute = functor_lang::suppress_trace();
    let saved_handlers = crate::functor_lang_prelude::take_ui_handlers();
    // start (project-wide) → the frame offsets it executed on.
    let mut hits: BTreeMap<usize, BTreeSet<i64>> = BTreeMap::new();
    for start in paused_coverage {
        hits.entry(start).or_default().insert(0);
    }
    for (ring_frame, entries) in ring {
        let offset = *ring_frame as i64 - frame as i64;
        // offset == 0 is NOT skipped: when scrubbed to frame K, the values
        // replay saw an empty journal (seek clears it) and covered only draw
        // — the ring's (K, journal) entry is the sole source of the paused
        // frame's tick/update coverage. Unscrubbed, the union with
        // paused_coverage is identical (BTreeSet dedups), so always
        // processing is strictly safe for one cheap extra replay.
        for e in entries {
            if let Ok((_discard, cov)) = session.call_covered(e.entry, e.args.clone(), &mut FunctorHost)
            {
                for start in cov {
                    hits.entry(start).or_default().insert(offset);
                }
            }
        }
    }
    let _replay_pushed = crate::functor_lang_prelude::take_ui_handlers();
    crate::functor_lang_prelude::restore_ui_handlers(saved_handlers);
    // Group per file, in file-local offsets.
    let mut per_file: BTreeMap<String, Vec<serde_json::Value>> = BTreeMap::new();
    for (start, offsets) in hits {
        if let Some((file, local, _)) = span_to_file(sources, Span::new(start, start)) {
            per_file.entry(file).or_default().push(serde_json::json!({
                "start": local,
                "frames": offsets.iter().collect::<Vec<_>>(),
            }));
        }
    }
    serde_json::json!(per_file)
}

/// The static could-run set, per file in local offsets.
fn runnable_json(runnable: &[usize], sources: &[InspectorSource]) -> serde_json::Value {
    use std::collections::BTreeMap;
    let mut per_file: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for &start in runnable {
        if let Some((file, local, _)) = span_to_file(sources, Span::new(start, start)) {
            per_file.entry(file).or_default().push(local);
        }
    }
    serde_json::json!(per_file)
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
        let a = build_trace_doc(false, 0, 0.0, &sources, &[], None, &session);
        let b = build_trace_doc(false, 7, 4.5, &sources, &[], None, &session);
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
        let doc: serde_json::Value = serde_json::from_str(&build_trace_doc(
            true, 3, 1.6, &sources, &journal, None, &session,
        ))
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

    #[test]
    fn wire_carries_v2_fields_and_reference_sites() {
        let (session, model, sources) = session_and_sources();
        let journal = vec![JournalEntry {
            entry: "tick",
            args: vec![model, Value::Number(0.2), Value::Number(1.1)],
            provenance: Provenance::Tick,
        }];
        let doc: serde_json::Value = serde_json::from_str(&build_trace_doc(
            true, 1, 0.2, &sources, &journal, None, &session,
        ))
        .unwrap();
        let tick = &doc["invocations"][0];

        // The invocation result carries its preview alongside the full text.
        assert!(tick["result_preview"].is_string());

        let binds = tick["bindings"].as_array().unwrap();
        // Every binding carries preview/kind/site; the model param is a
        // composite (preview elides) and `dt` a primitive (preview == value).
        for b in binds {
            assert!(b["preview"].is_string(), "{b}");
            assert!(matches!(b["kind"].as_str(), Some("primitive" | "composite")), "{b}");
            assert!(matches!(b["site"].as_str(), Some("binder" | "ref")), "{b}");
        }
        let m = binds.iter().find(|b| b["name"] == "m" && b["site"] == "binder").unwrap();
        assert_eq!(m["kind"], serde_json::json!("composite"));
        let dt = binds.iter().find(|b| b["name"] == "dt" && b["site"] == "binder").unwrap();
        assert_eq!(dt["kind"], serde_json::json!("primitive"));
        assert_eq!(dt["preview"], dt["value"]);

        // Reference sites now ship: `m.n + dt` READS both names.
        assert!(
            binds.iter().any(|b| b["name"] == "m" && b["site"] == "ref"),
            "reference sites reach the wire: {binds:#?}"
        );
        assert!(binds.iter().any(|b| b["name"] == "dt" && b["site"] == "ref"));
    }

    #[test]
    fn replay_is_observation_only() {
        use crate::functor_lang_prelude::{push_ui_handler, take_ui_handlers, UiHandler};

        // A tick that BOTH traces (`Debug.log`) and registers a UI handler
        // (`Ui.button`) — the two ambient channels a replay could perturb.
        let src = "\
            let init = { n: 0.0 }\n\
            let tick = (m, dt: Float, tts: Float) =>\n\
              let b = Ui.button(\"hi\", 1.0) in\n\
              Debug.log(\"suppress-probe\", m)\n";
        let project =
            load_single_source("game", src).unwrap_or_else(|e| panic!("load: {}", e.render()));
        let session = Session::load(&project.module, &mut FunctorHost)
            .unwrap_or_else(|f| panic!("session: {}", f.error.message));
        let model = session.global("init").expect("init");
        let sources = inspector_sources(&project.sources);
        let journal = vec![JournalEntry {
            entry: "tick",
            args: vec![model, Value::Number(0.1), Value::Number(0.1)],
            provenance: Provenance::Tick,
        }];

        // A capturing sink (the process-wide test seam); the filter below
        // keys on this test's unique label so parallel tests can't pollute.
        let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let sink = captured.clone();
        functor_lang::set_trace_sink(Box::new(move |line| sink.lock().unwrap().push(line)));

        // A handler already registered mid-frame must survive the replay
        // untouched, with none of the replay's own pushes appended after it.
        let _ = take_ui_handlers();
        push_ui_handler(UiHandler::Msg(Value::Number(9.0)));

        let doc: serde_json::Value = serde_json::from_str(&build_trace_doc(
            true, 1, 0.1, &sources, &journal, None, &session,
        ))
        .unwrap();
        // The replay itself succeeded — the traced-and-widgeted tick recorded.
        let tick = &doc["invocations"][0];
        assert_eq!(tick["entry"], serde_json::json!("tick"));
        assert!(
            tick["bindings"].as_array().unwrap().iter().any(|b| b["name"] == "b"),
            "the Ui.button binding recorded"
        );

        let leaked: Vec<String> = captured
            .lock()
            .unwrap()
            .iter()
            .filter(|l| l.contains("suppress-probe"))
            .cloned()
            .collect();
        assert!(leaked.is_empty(), "replay re-emitted Debug.log: {leaked:?}");

        let handlers = take_ui_handlers();
        assert_eq!(handlers.len(), 1, "replay pushes must not linger in the UI table");
        assert!(matches!(handlers[0], UiHandler::Msg(Value::Number(n)) if n == 9.0));
    }

    #[test]
    fn coverage_window_and_runnable_pin_the_gutter_states() {
        // The recency gutter's exact scenario: a branch whose arm depends on
        // the model. Frame 5 (paused) and 6 take the true arm; frame 4 took
        // the false arm; a third arm never runs in the window but is
        // statically runnable.
        let src = "\
            let init = { n: 0.0 }\n\
            let tick = (m, dt: Float, tts: Float) =>\n\
              match m.n < 1.0 with\n\
              | true => { n: m.n + 1.0 }\n\
              | false => { n: 0.0 }\n";
        let project =
            load_single_source("game", src).unwrap_or_else(|e| panic!("load: {}", e.render()));
        let session = Session::load(&project.module, &mut FunctorHost)
            .unwrap_or_else(|f| panic!("session: {}", f.error.message));
        let sources = inspector_sources(&project.sources);
        let runnable = functor_lang::coverage::runnable_offsets(&project.module);

        let model = |n: f64| {
            Value::Record(Rc::new(vec![("n".to_string(), Value::Number(n))]))
        };
        let tick = |n: f64| JournalEntry {
            entry: "tick",
            args: vec![model(n), Value::Number(0.1), Value::Number(0.1)],
            provenance: Provenance::Tick,
        };
        // Paused frame 5 takes true (n=0); frame 4 took false (n=2); frame 6
        // takes true again (n=0.2).
        let journal = vec![tick(0.0)];
        let ring = vec![
            (4u64, vec![tick(2.0)]),
            (5u64, vec![tick(0.0)]),
            (6u64, vec![tick(0.2)]),
        ];
        let doc: serde_json::Value = serde_json::from_str(&build_trace_doc_with_coverage(
            true,
            5,
            0.5,
            &sources,
            &journal,
            None,
            &ring,
            &runnable,
            &session,
        ))
        .unwrap();

        let true_arm = src.find("{ n: m.n + 1.0 }").unwrap();
        // rfind: `init` is the SAME text as the false arm — the arm is last.
        let false_arm = src.rfind("{ n: 0.0 }").unwrap();
        let cov = doc["coverage"]["game.fun"].as_array().expect("coverage for game.fun");
        let frames_at = |start: usize| {
            cov.iter()
                .find(|c| c["start"] == serde_json::json!(start))
                .map(|c| c["frames"].as_array().unwrap().iter().map(|f| f.as_i64().unwrap()).collect::<Vec<_>>())
        };
        // The true arm ran on the paused frame (0) and the frame after (+1),
        // never the frame before.
        assert_eq!(frames_at(true_arm), Some(vec![0, 1]), "{cov:#?}");
        // The false arm ran ONLY the frame before (−1) — the cyan case.
        assert_eq!(frames_at(false_arm), Some(vec![-1]), "{cov:#?}");
        // Both arms are statically runnable (the dark state's baseline).
        let runnable_local: Vec<usize> = doc["runnable"]["game.fun"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as usize)
            .collect();
        assert!(runnable_local.contains(&true_arm) && runnable_local.contains(&false_arm));
    }

    #[test]
    fn draw_is_synthesized_against_the_frozen_model() {
        // A `draw` that needs no engine prelude: it returns a number derived
        // from the model, so the replay works under the plain host and the
        // recorded bindings carry real values.
        let src = "\
            let init = { n: 4.0 }\n\
            let tick = (m, dt: Float, tts: Float) => m\n\
            let draw = (m, tts: Float) =>\n\
              let shade = m.n + tts in\n\
              shade\n";
        let project =
            load_single_source("game", src).unwrap_or_else(|e| panic!("load: {}", e.render()));
        let session = Session::load(&project.module, &mut FunctorHost)
            .unwrap_or_else(|f| panic!("session: {}", f.error.message));
        let model = session.global("init").expect("init");
        let sources = inspector_sources(&project.sources);

        let draw_args = vec![model, Value::Number(1.5)];
        let doc: serde_json::Value = serde_json::from_str(&build_trace_doc(
            true,
            2,
            1.5,
            &sources,
            &[],
            Some(&draw_args),
            &session,
        ))
        .unwrap();

        let invs = doc["invocations"].as_array().unwrap();
        let draw = invs.iter().find(|i| i["entry"] == "draw").expect("draw invocation");
        assert_eq!(draw["provenance"], serde_json::json!("draw"));
        assert_eq!(draw["count"], serde_json::json!(1));
        let binds = draw["bindings"].as_array().unwrap();
        let shade = binds
            .iter()
            .find(|b| b["name"] == "shade" && b["site"] == "binder")
            .expect("shade binder");
        assert_eq!(shade["value"], serde_json::json!("5.5"));

        // A program WITHOUT draw skips gracefully (no invocation, no error).
        let (session2, model2, sources2) = session_and_sources();
        let args2 = vec![model2, Value::Number(0.5)];
        let doc2: serde_json::Value = serde_json::from_str(&build_trace_doc(
            true,
            2,
            0.5,
            &sources2,
            &[],
            Some(&args2),
            &session2,
        ))
        .unwrap();
        assert!(doc2["invocations"].as_array().unwrap().is_empty());
    }
}
