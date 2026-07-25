//! The deterministic multiplayer netsim, hosted in the browser.
//!
//! Natively the netsim is a test harness; on the web it is the substance of
//! **simulating a whole multiplayer session inside the IDE** — several clients
//! and a server running side by side, over a virtual network, scrubbable as one
//! environment. This module is the first half of that: it builds N producers
//! from the project's own entries, steps them through a [`NetSim`], and exposes
//! the sim's state and its whole-environment seek to JavaScript. Rendering the
//! instances into panes comes next and builds on exactly these exports.
//!
//! Two properties make this cheap rather than a port:
//!
//! - `functor-netsim` is platform-free — it drives `Box<dyn GameProducer>` over
//!   the shared `VirtualNet`, so it compiles to wasm32 untouched (its desktop
//!   dependency is dev-only).
//! - the web runtime already runs the SHARED `FunctorLangEmbeddedGame` behind a
//!   `WebPlatform` seam, so an instance here is the same producer the single-game
//!   path uses — built once per role, with that role's entry hoisted to the front
//!   of the shared file set (`file = module`: every sibling loads with it, which
//!   is exactly how a multi-entry project shares its `protocol.fun`).
//!
//! **No real sockets are opened.** A producer's `Sub.connect`/`Sub.listen`/
//! `Effect.send` become commands on the shared queue; the single-game frame loop
//! dispatches those to `web_sys::WebSocket`, but here the sim drains them itself
//! and routes them through the virtual network instead. That is the same seam
//! the native harness uses, and it is why the sim is deterministic.

use std::cell::RefCell;

use functor_netsim::NetSim;
use functor_runtime_common::functor_lang_game_embedded::FunctorLangEmbeddedGame;
use functor_runtime_common::Frame;
use wasm_bindgen::prelude::*;

use crate::functor_lang_game::WebPlatform;

thread_local! {
    /// The live sim, if one has been started. Single-threaded by construction on
    /// wasm, which is also what makes the producers' shared net-command queue
    /// safe to drain per instance here (the native harness has to serialize its
    /// tests for exactly this reason).
    static SIM: RefCell<Option<NetSim>> = const { RefCell::new(None) };

    /// The entry each instance was built from, positionally by id. The sim
    /// knows an instance's role (it derives server/client from the live routing
    /// tables) but not which FILE it came from, and a pane labelled only
    /// "client" is ambiguous the moment two roles share a project.
    static ENTRIES: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };

    /// A `start_sim` is between its first await and installing its sim. Guards
    /// against two overlapping starts clobbering each other.
    static STARTING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Upper bound on frames per [`sim_step`] call. A simulation step runs real game
/// code with no yield point, so an unbounded count would wedge the tab with no
/// way back — the same reason the effect broker caps its drain.
const MAX_STEP_FRAMES: u32 = 10_000;

/// Build every instance. Fallible and side-effect-free with respect to the
/// shared queues, so a failure leaves the page exactly as it was.
async fn build_sim(entries: &[String], seed: u64) -> Result<NetSim, JsValue> {
    // Fetch the project's sources ONCE and reuse them for every instance: the
    // roles differ only in which file leads the list.
    let project = crate::load_project_sources(&entries[0])
        .await
        .map_err(|e| JsValue::from_str(&e))?;

    let mut sim = NetSim::new(seed);
    for entry in entries {
        let sources = functor_netsim::entry_sources(entry, &project)
            .map_err(|e| JsValue::from_str(&format!("sim: {e}")))?;
        let producer = FunctorLangEmbeddedGame::create(sources, Box::new(WebPlatform::new()))
            .map_err(|e| JsValue::from_str(&format!("sim: cannot load '{entry}': {e}")))?;
        sim.add_producer(Box::new(producer));
    }
    Ok(sim)
}

/// Is a simulation running? The single-game frame loop asks each frame: while a
/// sim owns the page the single game is suspended, because producers share this
/// thread's command queues and each drain empties them for everyone (see the
/// call site in `lib.rs`).
pub(crate) fn is_running() -> bool {
    SIM.with(|sim| sim.borrow().is_some())
}

/// Run `f` against the live sim, or return a uniform "no sim" error.
///
/// `try_borrow_mut`, not `borrow_mut`: the borrow is held across arbitrary
/// producer execution, and a page that wraps `console` could in principle
/// re-enter a `__sim` export from a producer's log call. A panic in wasm
/// aborts with no unwind, which would leave the borrow flag set and the
/// runtime permanently wedged — a plain error is strictly better.
fn with_sim<T>(op: &str, f: impl FnOnce(&mut NetSim) -> Result<T, String>) -> Result<T, String> {
    SIM.with(|sim| {
        let Ok(mut slot) = sim.try_borrow_mut() else {
            return Err(format!("{op}: the simulation is busy (re-entered)"));
        };
        match slot.as_mut() {
            Some(sim) => f(sim),
            None => Err(format!(
                "{op}: no simulation is running — call functorLangStartSim first"
            )),
        }
    })
}

/// Clear EVERY shared command queue this thread's producers write to.
///
/// The sim routes conn commands itself, but a game can also queue HTTP
/// (`Effect.httpGet`), audio (`Effect.play`), and preload commands — and
/// `NetSim::step` drains none of those. Left behind, they are performed FOR REAL
/// by whichever frame loop drains next: after `stop_sim` the resumed single game
/// would fire the sim instances' HTTP requests and play their sounds, then push
/// the completions into its own `update` where those taggers are meaningless.
///
/// So both the sim's start and its stop clear all four. Note the consequence,
/// which is deliberate for now: a simulated game's non-network effects are
/// DROPPED rather than performed — the sim models the network, not the browser.
fn clear_shared_command_queues() {
    let _ = functor_runtime_common::net::drain_conn_commands();
    let _ = functor_runtime_common::net::drain_commands();
    let _ = functor_runtime_common::audio::drain_commands();
    let _ = functor_runtime_common::asset::preload::drain_commands();
}

/// Start a simulation: one instance per entry in `entries`, all sharing the
/// project's files, wired through a seeded virtual network.
///
/// `entries` is an array of project-relative `.fun` paths — one per instance,
/// repeated for repeated roles (`["server.fun", "client.fun", "client.fun"]` is
/// a server and two clients). Every instance must be created before the sim
/// steps, so this builds them all up front.
///
/// Replaces any previous sim on success; on failure (an unknown entry, a load
/// error) the previous one is left running and nothing is torn down.
#[wasm_bindgen(js_name = functorLangStartSim)]
pub async fn start_sim(entries: JsValue, seed: f64) -> Result<usize, JsValue> {
    let array = js_sys::Array::from(&entries);
    if !js_sys::Array::is_array(&entries) {
        return Err(JsValue::from_str(
            "sim: entries must be an ARRAY of project .fun paths — a bare string \
             would be iterated character by character",
        ));
    }
    let mut parsed: Vec<String> = Vec::with_capacity(array.length() as usize);
    for (i, value) in array.iter().enumerate() {
        let Some(entry) = value.as_string() else {
            return Err(JsValue::from_str(&format!(
                "sim: entry {i} is not a string — every instance needs a project \
                 .fun path"
            )));
        };
        parsed.push(entry);
    }
    let entries = parsed;
    if entries.is_empty() {
        return Err(JsValue::from_str(
            "sim: no entries given — pass one project .fun path per instance, \
             e.g. [\"server.fun\", \"client.fun\", \"client.fun\"]",
        ));
    }

    // Reject a second concurrent start: both would await the fetch, both would
    // build producers, and the loser's instances would be dropped after it had
    // already cleared the shared queues.
    if STARTING.with(|s| s.replace(true)) {
        return Err(JsValue::from_str(
            "sim: a simulation is already starting — await that call first",
        ));
    }
    let result = build_sim(&entries, seed as u64).await;
    STARTING.with(|s| s.set(false));
    let sim = result?;

    // Everything is built and can no longer fail, so it is now safe to disturb
    // shared state: clear the queues (including anything the producers'
    // construction queued, which would otherwise be drained by instance 0 on the
    // first step) and install the sim.
    let count = sim.len();
    clear_shared_command_queues();
    SIM.with(|slot| *slot.borrow_mut() = Some(sim));
    ENTRIES.with(|e| *e.borrow_mut() = entries.clone());
    log::info!(
        "[functor-lang] sim started: {count} instances ({})",
        entries.join(", ")
    );
    Ok(count)
}

/// Advance the simulation by `frames` lockstep frames.
///
/// Surfaces a failed branch commit (stepping on from a scrub whose frame the
/// producers can no longer restore) as an error rather than stepping silently
/// nowhere.
#[wasm_bindgen(js_name = functorLangSimStep)]
pub fn sim_step(frames: u32) -> Result<(), JsValue> {
    if frames > MAX_STEP_FRAMES {
        return Err(JsValue::from_str(&format!(
            "step: {frames} frames at once would wedge the page — step at most \
             {MAX_STEP_FRAMES}, in chunks"
        )));
    }
    with_sim("step", |sim| {
        sim.step_n(frames as usize);
        match sim.take_last_error() {
            Some(e) => Err(format!("step: {e}")),
            None => Ok(()),
        }
    })
    .map_err(|e| JsValue::from_str(&e))
}

/// Number of instances in the running sim (0 when none is running).
#[wasm_bindgen(js_name = functorLangSimLen)]
pub fn sim_len() -> usize {
    SIM.with(|sim| sim.borrow().as_ref().map(|s| s.len()).unwrap_or(0))
}

/// One instance's model, pretty-printed. Free-form text for display and
/// assertions — callers must not parse it (see `protocol::GameProducer`).
#[wasm_bindgen(js_name = functorLangSimState)]
pub fn sim_state(id: usize) -> Result<String, JsValue> {
    with_sim("state", |sim| {
        if id >= sim.len() {
            return Err(format!(
                "state: no instance {id} — the sim has {}",
                sim.len()
            ));
        }
        Ok(sim.state(id))
    })
    .map_err(|e| JsValue::from_str(&e))
}

/// The live frame count, the frames a seek can reach, and where a seek is
/// currently parked — as `{frame, lo, hi, scrubPos}`, the shape a timeline UI
/// needs in one call. `lo`/`hi` are `null` before the first step.
#[wasm_bindgen(js_name = functorLangSimTimeline)]
pub fn sim_timeline() -> Result<JsValue, JsValue> {
    with_sim("timeline", |sim| {
        let obj = js_sys::Object::new();
        let set = |key: &str, value: JsValue| {
            let _ = js_sys::Reflect::set(&obj, &JsValue::from_str(key), &value);
        };
        set("frame", JsValue::from_f64(sim.frame() as f64));
        match sim.frame_range() {
            Some((lo, hi)) => {
                set("lo", JsValue::from_f64(lo as f64));
                set("hi", JsValue::from_f64(hi as f64));
            }
            None => {
                set("lo", JsValue::NULL);
                set("hi", JsValue::NULL);
            }
        }
        set(
            "scrubPos",
            match sim.scrub_pos() {
                Some(f) => JsValue::from_f64(f as f64),
                None => JsValue::NULL,
            },
        );
        Ok(obj.into())
    })
    .map_err(|e| JsValue::from_str(&e))
}

/// **Whole-environment seek**: park every instance AND the virtual network on
/// `frame`, so each pane shows its own view of that moment and the packets that
/// were in flight between them are in flight again.
///
/// Non-destructive while parked — the recorded future survives, so a timeline UI
/// can drag freely. The next [`sim_step`] commits the branch.
#[wasm_bindgen(js_name = functorLangSimSeek)]
pub fn sim_seek(frame: u32) -> Result<String, JsValue> {
    with_sim("seek", |sim| sim.seek(frame as u64)).map_err(|e| JsValue::from_str(&e))
}

/// Every instance's frame for this render, in instance order — or `None` when no
/// sim is running, which is the frame loop's signal to render the single game as
/// usual.
///
/// Returns descriptions, not pixels: GL stays in the frame loop, so this module
/// never touches a context. Takes `&mut` internally because a producer evaluates
/// its `draw` here.
pub(crate) fn render_frames(time: &functor_runtime_common::FrameTime) -> Option<Vec<Frame>> {
    SIM.with(|sim| {
        let mut slot = sim.try_borrow_mut().ok()?;
        let sim = slot.as_mut()?;
        Some((0..sim.len()).map(|i| sim.render(i, time.clone())).collect())
    })
}

/// One entry per instance, in pane order: `{id, role, entry, connections,
/// inboundInFlight}`.
///
/// This is what a pane label is drawn from — without it the view is three
/// unattributed 3D scenes and the reader cannot tell the authoritative world
/// from a client's copy of it. Rendering the labels is the page's job (DOM text
/// over the canvas), not the renderer's.
#[wasm_bindgen(js_name = functorLangSimPanes)]
pub fn sim_panes() -> Result<JsValue, JsValue> {
    with_sim("panes", |sim| {
        let entries = ENTRIES.with(|e| e.borrow().clone());
        let panes = js_sys::Array::new();
        for id in 0..sim.len() {
            let info = sim.client_info(id);
            let obj = js_sys::Object::new();
            let set = |key: &str, value: JsValue| {
                let _ = js_sys::Reflect::set(&obj, &JsValue::from_str(key), &value);
            };
            set("id", JsValue::from_f64(id as f64));
            set("role", JsValue::from_str(info.role.as_str()));
            set(
                "entry",
                JsValue::from_str(entries.get(id).map(String::as_str).unwrap_or("")),
            );
            set("connections", JsValue::from_f64(info.connections as f64));
            set(
                "inboundInFlight",
                JsValue::from_f64(info.inbound_in_flight as f64),
            );
            panes.push(&obj);
        }
        Ok(panes.into())
    })
    .map_err(|e| JsValue::from_str(&e))
}

/// Set the impairment on the link between two instances: `latencyTicks`,
/// `jitterTicks`, `loss` (0..1), `reorder`.
///
/// This is what makes divergence VISIBLE — with a clean link every pane shows the
/// same world, and the point of watching a server beside its clients is seeing
/// them disagree. Impairment is configuration rather than recorded state, so it
/// survives a rewind: dial it in, scrub back, and watch the same moment play out
/// under the new conditions.
#[wasm_bindgen(js_name = functorLangSimSetLink)]
pub fn sim_set_link(
    a: usize,
    b: usize,
    latency_ticks: u32,
    jitter_ticks: u32,
    loss: f32,
    reorder: bool,
) -> Result<(), JsValue> {
    with_sim("setLink", |sim| {
        if a >= sim.len() || b >= sim.len() {
            return Err(format!(
                "setLink: no such instance pair ({a}, {b}) — the sim has {}",
                sim.len()
            ));
        }
        if !(0.0..=1.0).contains(&loss) {
            return Err(format!("setLink: loss must be in 0..=1, got {loss}"));
        }
        sim.set_link(
            a,
            b,
            functor_netsim::Link {
                latency_ticks,
                jitter_ticks,
                loss,
                reorder,
            },
        );
        Ok(())
    })
    .map_err(|e| JsValue::from_str(&e))
}

/// Tear down the running sim (freeing its producers), if any, and resume the
/// single game. Every shared queue is cleared with it, so the resumed game
/// inherits none of the instances' pending commands — otherwise it would perform
/// their HTTP requests and play their sounds as if they were its own.
#[wasm_bindgen(js_name = functorLangStopSim)]
pub fn stop_sim() {
    SIM.with(|sim| *sim.borrow_mut() = None);
    ENTRIES.with(|e| e.borrow_mut().clear());
    clear_shared_command_queues();
}
