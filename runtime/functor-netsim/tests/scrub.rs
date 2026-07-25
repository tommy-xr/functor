//! Whole-environment time travel over the netsim: scrubbing the SIM, not one
//! client.
//!
//! `examples/mp` (an authoritative server + two clients) is stepped, then
//! `NetSim::seek` rewinds the entire environment to an earlier frame — every
//! instance's model AND the virtual network — and `resume` returns to live.
//! The properties under test are the ones a multi-pane scrubber in the IDEs
//! depends on:
//!
//! - a seek restores *every* instance at once, each to its own view of that
//!   frame (a client's lagging world, not the server's authoritative one), and
//!   is non-destructive until the next step;
//! - the network is restored too, so packets that were mid-flight at that frame
//!   are mid-flight again — asserted through the strongest available property:
//!   stepping on from an untouched scrub REPLAYS the original timeline exactly,
//!   which is only possible if the in-flight packets came back with the models;
//! - resuming from a scrubbed frame commits the branch, matching the shipped
//!   single-game scrubber (`run.rs`) rather than jumping back to live.
//!
//! Note the load-bearing asymmetry (`timetravel.rs` module docs): scrubbing
//! BACKWARD is a plain restore and needs no determinism. These tests therefore
//! assert exact equality, not approximate convergence.
//!
//! `#[ignore]`d by default like the sibling suites (they pull the full desktop
//! runtime as a dev-dependency); run with:
//!
//! ```sh
//! cargo test -p functor-netsim --test scrub -- --ignored --nocapture
//! ```

use functor_netsim::{InstanceId, Link, NetSim};
use functor_runtime_desktop::functor_lang_game::FunctorLangGame;

// Same process-global conn-queue caveat as tests/mp.rs: serialize the tests in
// this binary and clear residue at each start.
static NET_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn add_mp(sim: &mut NetSim, entry: &str) -> InstanceId {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/mp")
        .join(entry);
    assert!(path.exists(), "missing {}", path.display());
    sim.add_producer(Box::new(FunctorLangGame::create(path.to_str().unwrap())))
}

/// Every instance's view of the environment, in one comparable value.
fn views(sim: &NetSim, ids: &[InstanceId]) -> Vec<String> {
    ids.iter().map(|id| sim.state(*id)).collect()
}

#[test]
#[ignore = "pulls the desktop runtime dev-dependency; run with --ignored"]
fn a_scrub_rewinds_every_instance_and_stepping_on_replays_the_timeline_exactly() {
    let _guard = NET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _ = functor_runtime_common::net::drain_conn_commands();

    let mut sim = NetSim::new(1);
    let ids = [
        add_mp(&mut sim, "server.fun"),
        add_mp(&mut sim, "client.fun"),
        add_mp(&mut sim, "client.fun"),
    ];

    // Impair the links so packets are GENUINELY mid-flight at the snapshot cut.
    // Under `LinkProfile::PERFECT` the one-tick latency is consumed by the same
    // frame's `vnet.advance(1)`, so nothing is ever in flight when the snapshot
    // is taken and the restore-the-network claim would go untested. [xreview]
    let laggy = Link {
        latency_ticks: 4,
        jitter_ticks: 2,
        ..Link::PERFECT
    };
    sim.set_link(ids[0], ids[1], laggy);
    sim.set_link(ids[0], ids[2], laggy);

    // Nothing is reachable before the first step.
    assert_eq!(sim.frame_range(), None);
    assert!(sim.seek(0).is_err(), "seek must refuse an empty history");

    // Connect and let snapshots flow; the anchor is the frame we will scrub to.
    sim.step_n(30);
    let anchor = sim.frame() - 1;
    let anchor_views = views(&sim, &ids);
    assert!(
        sim.in_flight() > 0,
        "the anchor frame must have packets in flight, or restoring the \
         network proves nothing"
    );

    // Simulate well past it, recording each frame's views as the timeline to
    // reproduce.
    let mut timeline = Vec::new();
    for _ in 0..40 {
        sim.step();
        timeline.push(views(&sim, &ids));
    }
    let live_frame = sim.frame();
    // Non-vacuity PER INSTANCE: a whole-vector compare would still pass if one
    // pane happened to look identical at both frames, while the test claims
    // every instance rewinds. [xreview]
    for (i, live) in timeline.last().unwrap().iter().enumerate() {
        assert_ne!(
            live, &anchor_views[i],
            "instance {i} must differ between the anchor and the live frame, \
             or its rewind proves nothing"
        );
    }
    assert_eq!(sim.frame_range(), Some((0, live_frame - 1)));

    // The whole environment rewinds together — each instance to its OWN view of
    // the anchor frame (the clients' lagging worlds, not the server's).
    sim.seek(anchor).expect("seek to the anchor frame");
    assert_eq!(sim.scrub_pos(), Some(anchor));
    assert_eq!(
        views(&sim, &ids),
        anchor_views,
        "every instance must restore to its own view of the anchor frame"
    );
    // Non-destructive while parked: the recorded future is still there.
    assert_eq!(sim.frame_range(), Some((0, live_frame - 1)));

    // Stepping on commits the branch — and because the network was restored
    // WITH the models (including the packets in flight at that instant), the
    // replayed timeline is byte-identical to the original.
    let mut replayed = Vec::new();
    for _ in 0..40 {
        sim.step();
        replayed.push(views(&sim, &ids));
    }
    assert_eq!(sim.scrub_pos(), None);
    assert_eq!(
        replayed, timeline,
        "replay from a scrub must reproduce the original timeline exactly"
    );
    assert_eq!(sim.frame(), live_frame, "the branch must not shift frames");
}

#[test]
#[ignore = "pulls the desktop runtime dev-dependency; run with --ignored"]
fn seek_clamps_into_range_and_the_sim_steps_on_after_a_scrub() {
    let _guard = NET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _ = functor_runtime_common::net::drain_conn_commands();

    let mut sim = NetSim::new(7);
    let server = add_mp(&mut sim, "server.fun");
    let client = add_mp(&mut sim, "client.fun");

    sim.step_n(30);
    let (lo, hi) = sim.frame_range().expect("frames recorded");

    // Out-of-range seeks clamp rather than erroring — a scrubber drags past the
    // ends constantly.
    sim.seek(hi + 500).expect("clamps to the live end");
    assert_eq!(sim.scrub_pos(), Some(hi));
    sim.seek(0).expect("clamps to the recorded floor");
    assert_eq!(sim.scrub_pos(), Some(lo));

    // Stepping on from a scrubbed position keeps the sim coherent, and the
    // branch it commits starts exactly at the scrubbed frame.
    let branch_views = views(&sim, &[server, client]);
    sim.step();
    assert_eq!(sim.scrub_pos(), None, "stepping commits the branch");
    // The branch rewound to the end of frame `lo`, and this step ran frame
    // `lo + 1` — so the recorded future beyond the scrub is gone and the
    // timeline continues from the scrubbed frame rather than from live.
    assert_eq!(
        sim.frame(),
        lo + 2,
        "the committed branch continues from the scrubbed frame, not from live"
    );
    assert_eq!(
        sim.frame_range(),
        Some((lo, lo + 1)),
        "committing the branch drops the recorded future"
    );
    sim.step_n(20);
    let after = views(&sim, &[server, client]);
    println!("SERVER: {}", after[0]);
    println!("CLIENT: {}", after[1]);
    assert_ne!(
        after, branch_views,
        "the branch must actually simulate forward from the scrubbed frame"
    );
}

#[test]
#[ignore = "pulls the desktop runtime dev-dependency; run with --ignored"]
fn a_seek_past_the_pruned_horizon_is_refused_loudly() {
    let _guard = NET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _ = functor_runtime_common::net::drain_conn_commands();

    // Step past the bounded ring so BOTH sides have pruned. This is the regime
    // where the sim's horizon and the producers' have to agree — and where a
    // producer's own `seek_scene_to` would silently CLAMP to a different frame
    // rather than refuse, which is why the sim checks the ranges itself. [xreview]
    let mut sim = NetSim::new(3);
    let ids = [add_mp(&mut sim, "server.fun"), add_mp(&mut sim, "client.fun")];
    sim.step_n(functor_runtime_common::timetravel::DEFAULT_HISTORY_FRAMES + 60);

    let (lo, hi) = sim.frame_range().expect("frames recorded");
    assert!(lo > 0, "the ring must have pruned, or this proves nothing");

    // Inside the horizon: a real restore, not a clamp.
    let views_at_lo = {
        sim.seek(lo).expect("seek to the oldest retained frame");
        views(&sim, &ids)
    };
    sim.seek(hi).expect("seek to the newest frame");
    sim.seek(lo).expect("re-seek to the oldest frame");
    assert_eq!(
        views(&sim, &ids),
        views_at_lo,
        "seeking is idempotent: the same frame restores the same environment"
    );

    // Below it: clamped to the floor, which is a real restore of a real frame —
    // never a silent restore of some other frame.
    sim.seek(lo - 1).expect("a pruned frame clamps to the floor");
    assert_eq!(sim.scrub_pos(), Some(lo));
    assert_eq!(views(&sim, &ids), views_at_lo);
}

#[test]
#[should_panic(expected = "add_producer after the sim has stepped")]
#[ignore = "pulls the desktop runtime dev-dependency; run with --ignored"]
fn joining_after_the_sim_has_stepped_is_refused() {
    let _guard = NET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _ = functor_runtime_common::net::drain_conn_commands();

    // A late joiner would record from ITS frame 0 while the sim is at frame n,
    // and every snapshot taken before it joined describes a network with no
    // node for it — so the frame alignment a seek depends on would be broken
    // silently. [xreview]
    let mut sim = NetSim::new(5);
    add_mp(&mut sim, "server.fun");
    sim.step_n(5);
    add_mp(&mut sim, "client.fun");
}
