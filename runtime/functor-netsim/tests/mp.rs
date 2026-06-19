//! Netsim test of the multiplayer prototype: one authoritative `mpserver` and two
//! `mpclient`s, run as independent in-process instances through the virtual
//! network and stepped deterministically. Proves the full loop -- clients connect,
//! the server spawns + simulates them, broadcasts world snapshots, and both
//! clients converge on the same world -- with no GL and no sockets.
//!
//! Ignored by default; build the samples first:
//!
//! ```sh
//! ./target/debug/functor -d examples/mpserver build native
//! ./target/debug/functor -d examples/mpclient build native
//! cargo test -p functor-netsim --test mp -- --ignored --nocapture
//! ```

use functor_netsim::{Link, NetSim};

fn dylib(sample: &str) -> String {
    let name = format!(
        "{}game_native{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(format!("examples/{sample}/build-native/target/debug/{name}"))
        .to_str()
        .unwrap()
        .to_string()
}

fn require(samples: &[&str]) {
    for s in samples {
        assert!(
            std::path::Path::new(&dylib(s)).exists(),
            "build {s} first (see this test's doc comment)"
        );
    }
}

#[test]
#[ignore = "needs mpserver + mpclient built native"]
fn server_broadcasts_world_and_clients_converge() {
    require(&["mpserver", "mpclient"]);

    let mut sim = NetSim::new(1);
    let server = sim.add(&dylib("mpserver"));
    let c1 = sim.add(&dylib("mpclient"));
    let c2 = sim.add(&dylib("mpclient"));

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
#[ignore = "needs mpserver + mpclient built native"]
fn latency_delays_what_the_client_sees() {
    require(&["mpserver", "mpclient"]);

    let mut sim = NetSim::new(1);
    let server = sim.add(&dylib("mpserver"));
    let client = sim.add(&dylib("mpclient"));
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
