//! Netsim test of TYPED messages (`Effect.sendMsg` → `Net.Data`): a ping
//! server and a client exchange a shared-module ADT (`tests/fixtures/typed/
//! tproto.fun`) with no string codec — the value is framed by the host,
//! decoded by the host, and matched by ctor on the far side. The escalating
//! ping/pong counter proves repeated typed round-trips in both directions,
//! deterministically, with no GL and no sockets — including over a laggy
//! link.
//!
//! Still `#[ignore]`d by default (pulls the full desktop runtime as a
//! dev-dependency); run with:
//!
//! ```sh
//! cargo test -p functor-netsim --test typed -- --ignored --nocapture
//! ```

use functor_netsim::{InstanceId, Link, NetSim};
use functor_runtime_desktop::functor_lang_game::FunctorLangGame;

// Same process-global conn-queue caveat as tests/mp.rs: serialize the tests
// in this binary and clear residue at each start.
static NET_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn add_fixture(sim: &mut NetSim, entry: &str) -> InstanceId {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/typed")
        .join(entry);
    assert!(path.exists(), "missing fixture {}", path.display());
    sim.add_producer(Box::new(FunctorLangGame::create(path.to_str().unwrap())))
}

/// The `f64` value of a `name: <n>` field in a model's debug string.
fn field_of(state: &str, name: &str) -> f64 {
    let marker = format!("{name}: ");
    let start = state.find(&marker).unwrap_or_else(|| panic!("no `{name}` in {state}")) + marker.len();
    state[start..]
        .split(|c: char| c == ',' || c == '}' || c == ' ')
        .next()
        .and_then(|tok| tok.parse().ok())
        .unwrap_or_else(|| panic!("unparseable `{name}` in {state}"))
}

#[test]
#[ignore = "pulls the desktop runtime dev-dependency; run with --ignored"]
fn typed_messages_roundtrip_and_escalate() {
    let _guard = NET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _ = functor_runtime_common::net::drain_conn_commands();

    let mut sim = NetSim::new(1);
    let server = add_fixture(&mut sim, "tserver.fun");
    let client = add_fixture(&mut sim, "tclient.fun");

    sim.step_n(30);

    let server_state = sim.state(server);
    let client_state = sim.state(client);
    println!("SERVER: {server_state}");
    println!("CLIENT: {client_state}");

    // The exchange escalates Ping(1) → Pong(2) → Ping(3) → …: multiple typed
    // round-trips landed in both directions.
    assert!(
        field_of(&server_state, "pings") >= 3.0,
        "server should have answered several typed pings: {server_state}"
    );
    assert!(
        field_of(&client_state, "rounds") >= 3.0,
        "client should have completed several typed rounds: {client_state}"
    );
    // Pong payloads decode to the exact escalated values (2, 4, 6, …) — the
    // ADT survived the wire, not just "some message arrived".
    let last_pong = field_of(&client_state, "lastPong");
    assert!(
        last_pong >= 4.0 && (last_pong / 2.0).fract() == 0.0,
        "lastPong should be an even escalated value: {client_state}"
    );
}

#[test]
#[ignore = "pulls the desktop runtime dev-dependency; run with --ignored"]
fn typed_messages_survive_a_laggy_link() {
    let _guard = NET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _ = functor_runtime_common::net::drain_conn_commands();

    let mut sim = NetSim::new(1);
    let server = add_fixture(&mut sim, "tserver.fun");
    let client = add_fixture(&mut sim, "tclient.fun");
    sim.set_link(
        client,
        server,
        Link {
            latency_ticks: 5,
            jitter_ticks: 0,
            loss: 0.0,
            reorder: false,
        },
    );

    sim.step_n(80);
    let client_state = sim.state(client);
    assert!(
        field_of(&client_state, "rounds") >= 2.0,
        "typed round-trips should still complete over latency: {client_state}"
    );
}
