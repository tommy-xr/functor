//! End-to-end netsim tests: a real server game + a real client game, run as
//! independent in-process MLE producers, wired through the virtual network and
//! stepped deterministically — no GL, no sockets.
//!
//! Uses the `mle-wsserverdemo` (greet + echo) and `mle-wsdemo` (connect + send)
//! samples. Their authorities match (127.0.0.1:9001) so the harness routes the
//! client to the server.
//!
//! The instances are `.mle` games (E3 phase 0b): no dylib build is needed, only
//! the committed `examples/mle-*/game.mle` sources. Still `#[ignore]`d by
//! default (they pull in the full desktop runtime as a dev-dependency); run
//! with:
//!
//! ```sh
//! cargo test -p functor-netsim --test sim -- --ignored --nocapture
//! ```

use functor_netsim::{InstanceId, Link, NetSim};
use functor_runtime_desktop::mle_game::MleGame;

// MLE producers share this process's global conn-command queue, so the tests in
// this binary (cargo runs them as parallel threads) must not build/step their
// sims concurrently. Serialize them, and clear any residue at each test's start.
static NET_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn mle_path(sample: &str) -> String {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(format!("examples/{sample}/game.mle"))
        .to_str()
        .unwrap()
        .to_string()
}

fn add_mle(sim: &mut NetSim, sample: &str) -> InstanceId {
    let path = mle_path(sample);
    assert!(
        std::path::Path::new(&path).exists(),
        "missing {path} (the committed MLE example)"
    );
    sim.add_producer(Box::new(MleGame::create(&path)))
}

#[test]
#[ignore = "pulls the desktop runtime dev-dependency; run with --ignored"]
fn server_and_client_exchange_messages() {
    let _guard = NET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _ = functor_runtime_common::net::drain_conn_commands();

    let mut sim = NetSim::new(1);
    let server = add_mle(&mut sim, "mle-wsserverdemo");
    let client = add_mle(&mut sim, "mle-wsdemo");

    // Default (perfect) link: the exchange settles in a handful of frames.
    sim.step_n(10);

    let client_state = sim.state(client);
    assert!(
        client_state.contains("got-message"),
        "client should have received a message from the server: {client_state}"
    );
    let server_state = sim.state(server);
    assert!(
        server_state.contains("client-connected") || server_state.contains("echoed"),
        "server should have seen the client connect: {server_state}"
    );
}

#[test]
#[ignore = "pulls the desktop runtime dev-dependency; run with --ignored"]
fn latency_delays_delivery_deterministically() {
    let _guard = NET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _ = functor_runtime_common::net::drain_conn_commands();

    let mut sim = NetSim::new(1);
    let server = add_mle(&mut sim, "mle-wsserverdemo");
    let client = add_mle(&mut sim, "mle-wsdemo");
    // 8-tick one-way latency on the client<->server link.
    sim.set_link(
        client,
        server,
        Link {
            latency_ticks: 8,
            jitter_ticks: 0,
            loss: 0.0,
            reorder: false,
        },
    );

    // A few frames in, the server's greeting can't have made the round trip yet.
    sim.step_n(4);
    assert!(
        !sim.state(client).contains("got-message"),
        "no message should have arrived this early under 8-tick latency: {}",
        sim.state(client)
    );

    // Given enough frames, it does arrive.
    sim.step_n(30);
    assert!(
        sim.state(client).contains("got-message"),
        "the message should arrive once enough ticks elapse: {}",
        sim.state(client)
    );
}
