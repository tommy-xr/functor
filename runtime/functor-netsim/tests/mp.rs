//! Netsim test of the multiplayer prototype: one authoritative server and two
//! clients (`examples/mp` — a multi-entry project whose roles share a wire
//! protocol via the `Protocol` sibling module), run as independent in-process
//! Functor Lang producers through the virtual network and stepped deterministically.
//! Proves the full loop -- clients connect, the server spawns + simulates
//! them, broadcasts world snapshots, and both clients converge on the same
//! world -- with no GL and no sockets.
//!
//! The instances are `.fun` games (E3 phase 0b): no dylib build is needed,
//! only the committed `examples/mp/*.fun` sources. Still `#[ignore]`d by
//! default (they pull in the full desktop runtime as a dev-dependency); run
//! with:
//!
//! ```sh
//! cargo test -p functor-netsim --test mp -- --ignored --nocapture
//! ```

use functor_netsim::{InstanceId, Link, NetSim};
use functor_runtime_desktop::functor_lang_game::FunctorLangGame;

// Functor Lang producers share this process's global conn-command queue, so the tests in
// this binary (cargo runs them as parallel threads) must not build/step their
// sims concurrently. Serialize them, and clear any residue at each test's start.
static NET_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn functor_lang_path(entry: &str) -> String {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(format!("examples/mp/{entry}"))
        .to_str()
        .unwrap()
        .to_string()
}

fn add_functor_lang(sim: &mut NetSim, entry: &str) -> InstanceId {
    let path = functor_lang_path(entry);
    assert!(
        std::path::Path::new(&path).exists(),
        "missing {path} (the committed Functor Lang example)"
    );
    sim.add_producer(Box::new(FunctorLangGame::create(&path)))
}

#[test]
#[ignore = "pulls the desktop runtime dev-dependency; run with --ignored"]
fn server_broadcasts_world_and_clients_converge() {
    let _guard = NET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _ = functor_runtime_common::net::drain_conn_commands();

    let mut sim = NetSim::new(1);
    let server = add_functor_lang(&mut sim, "server.fun");
    let c1 = add_functor_lang(&mut sim, "client.fun");
    let c2 = add_functor_lang(&mut sim, "client.fun");

    // Let everyone connect and a few snapshots flow.
    sim.step_n(20);
    let early_c1 = sim.state(c1);

    // Keep simulating so the (server-authoritative) movement advances.
    sim.step_n(40);

    let server_state = sim.state(server);
    let c1_state = sim.state(c1);
    let c2_state = sim.state(c2);
    println!("SERVER : {server_state}");
    println!("CLIENT1: {c1_state}");
    println!("CLIENT2: {c2_state}");

    // Server tracked both clients.
    assert!(
        server_state.contains("nextPid = 2") || server_state.contains("nextPid: 2"),
        "server should have spawned 2 players: {server_state}"
    );
    // Both clients received world snapshots.
    assert!(c1_state.contains("in-world"), "client1: {c1_state}");
    assert!(c2_state.contains("in-world"), "client2: {c2_state}");
    // Movement advanced (the authoritative sim is running + broadcasting).
    assert_ne!(early_c1, c1_state, "client1's world should have changed as players moved");
    // Convergence: both clients render the same world (same server broadcast).
    assert_eq!(
        world_of(&c1_state),
        world_of(&c2_state),
        "clients should converge on the same world"
    );
}

/// The `world: ...` slice of a client's debug state (before `, status`).
fn world_of(state: &str) -> &str {
    let start = state.find("world:").expect("world field");
    let end = state[start..].find(", status").expect("status field") + start;
    &state[start..end]
}

#[test]
#[ignore = "pulls the desktop runtime dev-dependency; run with --ignored"]
fn latency_delays_what_the_client_sees() {
    let _guard = NET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _ = functor_runtime_common::net::drain_conn_commands();

    let mut sim = NetSim::new(1);
    let server = add_functor_lang(&mut sim, "server.fun");
    let client = add_functor_lang(&mut sim, "client.fun");
    // A laggy link: the client's view trails the server.
    sim.set_link(
        client,
        server,
        Link {
            latency_ticks: 10,
            jitter_ticks: 0,
            loss: 0.0,
            reorder: false,
        },
    );

    sim.step_n(60);
    assert!(
        sim.state(client).contains("in-world"),
        "client should still reach the world over a laggy link: {}",
        sim.state(client)
    );
}
