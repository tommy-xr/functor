//! End-to-end netsim tests: a real server game + a real client game, run as
//! independent in-process instances, wired through the virtual network and
//! stepped deterministically — no GL, no sockets.
//!
//! Uses the `wsserverdemo` (echo + greet) and `wsdemo` (connect + send) samples.
//! Their authorities match (127.0.0.1:9001) so the harness routes the client to
//! the server.
//!
//! Ignored by default: needs both sample dylibs built. Run with:
//!
//! ```sh
//! ./target/debug/functor -d examples/wsserverdemo build native
//! ./target/debug/functor -d examples/wsdemo build native
//! cargo test -p functor-netsim --test sim -- --ignored --nocapture
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

fn require_samples() -> (String, String) {
    let server = dylib("wsserverdemo");
    let client = dylib("wsdemo");
    assert!(
        std::path::Path::new(&server).exists() && std::path::Path::new(&client).exists(),
        "build the samples first (see this test's doc comment)"
    );
    (server, client)
}

#[test]
#[ignore = "needs wsserverdemo + wsdemo built native"]
fn server_and_client_exchange_messages() {
    let (server_dylib, client_dylib) = require_samples();

    let mut sim = NetSim::new(1);
    let server = sim.add(&server_dylib);
    let client = sim.add(&client_dylib);

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
#[ignore = "needs wsserverdemo + wsdemo built native"]
fn latency_delays_delivery_deterministically() {
    let (server_dylib, client_dylib) = require_samples();

    let mut sim = NetSim::new(1);
    let server = sim.add(&server_dylib);
    let client = sim.add(&client_dylib);
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
