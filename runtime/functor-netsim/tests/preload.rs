//! Per-instance preload ownership and whole-environment replay.

use std::sync::Mutex;

use functor_netsim::{InstanceId, NetSim};

mod support;
use support::{add_source_game, install_controlled_fetcher, ControlledFetches};

static PRELOAD_LOCK: Mutex<()> = Mutex::new(());

fn begin_controlled_preload_test() -> ControlledFetches {
    let _ = functor_runtime_common::asset::preload::drain_commands();
    functor_runtime_common::functor_lang_prelude::clear_preload_completions();
    install_controlled_fetcher()
}

fn add_preload_game(
    sim: &mut NetSim,
    root: &std::path::Path,
    name: &str,
    first: u32,
) -> InstanceId {
    let dir = root.join(name);
    let locator = format!(
        "https://netsim.invalid/{name}-{}-{first}.png",
        std::process::id()
    );
    let second = first * 10;
    let source = format!(
        "let warmed = Asset.texture({locator:?})\n\
         let init = 0.0\n\
         let update = (model, msg) =>\n\
           if msg == {first}.0\n\
           then (model + msg, Effect.preloadThen(warmed, {second}.0))\n\
           else model + msg\n\
         let tick = (model, dt, tts) =>\n\
           if model == 0.0\n\
           then (1.0, Effect.preloadThen(warmed, {first}.0))\n\
           else model\n\
         let draw = (model, tts) => Frame.create(\n\
           Camera3D.lookAt(Vec3.make(0.0, 0.0, -1.0), Vec3.make(0.0, 0.0, 0.0)),\n\
           Scene.group([]))\n"
    );
    add_source_game(sim, &dir, source)
}

fn states(sim: &NetSim, ids: &[InstanceId]) -> Vec<String> {
    ids.iter().map(|id| sim.state(*id)).collect()
}

fn preload_batch(locator: &str, count: usize) -> String {
    let effects = std::iter::repeat_n(
        format!("Effect.preloadThen(Asset.texture({locator:?}), 1.0)"),
        count,
    )
    .collect::<Vec<_>>()
    .join(", ");
    format!("Effect.batch([{effects}])")
}

fn add_delivery_game(
    sim: &mut NetSim,
    root: &std::path::Path,
    name: &str,
    marker: u32,
    subscription: &str,
) -> InstanceId {
    let dir = root.join(name);
    let locator = format!(
        "https://netsim.invalid/delivery-{name}-{}-{marker}.png",
        std::process::id()
    );
    let warmed = marker * 10;
    let source = format!(
        "let asset = Asset.texture({locator:?})\n\
         let toMsg = (event) =>\n\
           match event with\n\
           | Net.Connected(_) => {marker}.0\n\
           | _ => 0.0\n\
         let init = 0.0\n\
         let update = (model, msg) =>\n\
           if msg == {marker}.0\n\
           then (1.0, Effect.preloadThen(asset, {warmed}.0))\n\
           else model + msg\n\
         let subscriptions = (model) => {subscription}\n\
         let tick = (model, dt, tts) => model\n\
         let draw = (model, tts) => Frame.create(\n\
           Camera3D.lookAt(Vec3.make(0.0, 0.0, -1.0), Vec3.make(0.0, 0.0, 0.0)),\n\
           Scene.group([]))\n"
    );
    add_source_game(sim, &dir, source)
}

#[test]
fn network_delivery_preloads_stay_with_the_emitting_instance_and_replay() {
    let _guard = PRELOAD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let fetches = begin_controlled_preload_test();

    let root = std::env::temp_dir().join(format!(
        "functor-netsim-delivery-preload-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let mut sim = NetSim::new(7);
    let ids = [
        add_delivery_game(
            &mut sim,
            &root,
            "server",
            10,
            "Sub.listen(\"127.0.0.1:9321\", toMsg)",
        ),
        add_delivery_game(
            &mut sim,
            &root,
            "client",
            20,
            "Sub.connect(\"ws://127.0.0.1:9321/play\", toMsg)",
        ),
    ];

    // The Connected events are delivered after frame 0's snapshot cut. Each
    // update queues a preloadThen onto the process-global queue; capture must
    // happen before delivery advances to the other instance.
    sim.step();
    let anchor = sim.frame() - 1;
    let connected = states(&sim, &ids);
    assert_eq!(connected, vec!["1".to_string(), "1".to_string()]);

    let mut original = Vec::new();
    for _ in 0..3 {
        sim.step();
        original.push(states(&sim, &ids));
    }
    assert_eq!(fetches.lock().unwrap().len(), 2);
    assert_eq!(original.last().unwrap(), &connected);
    for (_, sender) in std::mem::take(&mut *fetches.lock().unwrap()) {
        sender
            .send(Err("controlled delivery preload settlement".to_string()))
            .expect("the pending asset future still owns its receiver");
    }
    for _ in 0..2 {
        sim.step();
        original.push(states(&sim, &ids));
    }
    assert_eq!(
        original.last().unwrap(),
        &vec!["101".to_string(), "201".to_string()],
        "each Connected handler must receive only its own preload completion"
    );

    // Seeking frame 0 restores its pending Connected events, whose replay must
    // recreate the per-instance preload outboxes. Stepping on then reproduces
    // the settlement frame exactly.
    sim.seek(anchor).expect("seek to connected-event snapshot");
    assert_eq!(states(&sim, &ids), connected);
    let mut replayed = Vec::new();
    for _ in 0..original.len() {
        sim.step();
        replayed.push(states(&sim, &ids));
    }
    assert_eq!(replayed, original);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn preload_then_is_instance_owned_and_replays_across_a_scrub() {
    let _guard = PRELOAD_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    // Hold both remote loads unresolved until the test releases them. This
    // guarantees the snapshot cuts through a genuinely in-flight preload,
    // rather than relying on timing or asset size.
    let fetches = begin_controlled_preload_test();

    let root = std::env::temp_dir().join(format!("functor-netsim-preload-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let mut sim = NetSim::new(11);
    let ids = [
        add_preload_game(&mut sim, &root, "first", 10),
        add_preload_game(&mut sim, &root, "second", 20),
    ];

    // Frame 0 captures each producer's first preload command. Frame 1 submits
    // both, and the controlled fetcher keeps them in flight at the snapshot.
    sim.step_n(2);
    let anchor = sim.frame() - 1;
    let anchor_states = states(&sim, &ids);
    assert_eq!(
        fetches.lock().unwrap().len(),
        2,
        "each instance must own and start its own request"
    );

    // Keep the futures unresolved across three more recorded frames. After the
    // live run settles them, the cache is warm; replay must nevertheless hold
    // their completion messages until the same original frame.
    let mut original = Vec::new();
    for _ in 0..3 {
        sim.step();
        original.push(states(&sim, &ids));
    }
    assert_eq!(
        original.last().unwrap(),
        &vec!["1".to_string(), "1".to_string()],
        "the controlled requests remain genuinely pending"
    );

    for (_, sender) in std::mem::take(&mut *fetches.lock().unwrap()) {
        sender
            .send(Err("controlled preload settlement".to_string()))
            .expect("the pending asset future still owns its receiver");
    }

    // The first settlement's update queues a SECOND preloadThen. That command
    // must remain with the producer whose completion handler emitted it; with
    // the old process-global drain, the following producer claimed it.
    for _ in 0..4 {
        sim.step();
        original.push(states(&sim, &ids));
    }
    assert_eq!(
        original.last().unwrap(),
        &vec!["111".to_string(), "221".to_string()],
        "each producer should receive both of its own chained completions"
    );

    // The anchor predates settlement. Its tokens have since been consumed, so
    // exact replay proves both the logical in-flight requests and their
    // completion messages were restored, and that completion landed before
    // the producer recorded each replayed tick.
    sim.seek(anchor).expect("seek to in-flight preload");
    assert_eq!(states(&sim, &ids), anchor_states);
    let mut replayed = Vec::new();
    for _ in 0..original.len() {
        sim.step();
        replayed.push(states(&sim, &ids));
    }
    assert_eq!(
        replayed, original,
        "branching before preload settlement must replay both producers exactly"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn replay_does_not_revive_a_preload_token_dropped_by_the_per_target_cap() {
    let _guard = PRELOAD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let fetches = begin_controlled_preload_test();

    let root =
        std::env::temp_dir().join(format!("functor-netsim-preload-cap-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let locator = format!("https://netsim.invalid/capped-{}.png", std::process::id());
    // The effect broker caps one drain at 1,000 nodes, so cross the 1,024
    // per-target token limit in two frames. Token 1 is intentionally evicted.
    let first = preload_batch(&locator, 900);
    let second = preload_batch(&locator, 125);
    let source = format!(
        "let init = 0.0\n\
         let update = (model, msg) => model + msg\n\
         let tick = (model, dt, tts) =>\n\
           if model == 0.0 then (1.0, {first})\n\
           else if model == 1.0 then (2.0, {second})\n\
           else model\n\
         let draw = (model, tts) => Frame.create(\n\
           Camera3D.lookAt(Vec3.make(0.0, 0.0, -1.0), Vec3.make(0.0, 0.0, 0.0)),\n\
           Scene.group([]))\n"
    );
    let mut sim = NetSim::new(17);
    let id = add_source_game(&mut sim, &root, source);

    sim.step();
    let anchor = sim.frame() - 1;
    let mut original = Vec::new();
    sim.step();
    original.push(sim.state(id));
    sim.step();
    original.push(sim.state(id));
    assert_eq!(fetches.lock().unwrap().len(), 1);
    for (_, sender) in std::mem::take(&mut *fetches.lock().unwrap()) {
        sender
            .send(Err("controlled capped preload settlement".to_string()))
            .expect("the pending asset future still owns its receiver");
    }
    sim.step();
    original.push(sim.state(id));
    assert_eq!(
        original,
        vec!["2".to_string(), "2".to_string(), "1026".to_string()],
        "exactly 1,024 of the 1,025 completion messages should be delivered"
    );

    // The failed asset is warm now. Without a dropped-token tombstone, token 1
    // would settle immediately after this seek and alter both timing and state.
    sim.seek(anchor).expect("seek before the capped batch");
    let mut replayed = Vec::new();
    for _ in 0..original.len() {
        sim.step();
        replayed.push(sim.state(id));
    }
    assert_eq!(replayed, original);

    let _ = std::fs::remove_dir_all(root);
}
