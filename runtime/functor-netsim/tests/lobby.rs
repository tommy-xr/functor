//! Netsim test of the master-server flow (`examples/lobby`): a master, a
//! game server, and a client run as in-process producers over the virtual
//! network. The game server registers with the master over its own
//! connection (typed `Register`); the client discovers it (`ListServers` /
//! `Servers`), opens a SECOND, model-driven connection to the discovered
//! addr, `Join`s, and lands in `Playing` on the server's `Welcome` — the
//! whole lobby handshake, typed end to end, no sockets, no GL.
//!
//! Still `#[ignore]`d by default (pulls the full desktop runtime as a
//! dev-dependency); run with:
//!
//! ```sh
//! cargo test -p functor-netsim --test lobby -- --ignored --nocapture
//! ```

use functor_netsim::{InstanceId, NetSim};
use functor_runtime_desktop::functor_lang_game::FunctorLangGame;

// Same process-global conn-queue caveat as tests/mp.rs: serialize the tests
// in this binary and clear residue at each start.
static NET_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn add_lobby(sim: &mut NetSim, entry: &str) -> InstanceId {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/lobby")
        .join(entry);
    assert!(path.exists(), "missing {}", path.display());
    sim.add_producer(Box::new(FunctorLangGame::create(path.to_str().unwrap())))
}

#[test]
#[ignore = "pulls the desktop runtime dev-dependency; run with --ignored"]
fn client_discovers_the_server_through_the_master_and_joins() {
    let _guard = NET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _ = functor_runtime_common::net::drain_conn_commands();

    let mut sim = NetSim::new(1);
    let master = add_lobby(&mut sim, "master.fun");
    let server = add_lobby(&mut sim, "gameserver.fun");
    let client = add_lobby(&mut sim, "client.fun");

    // register → list → discover → connect → join → welcome takes several
    // round trips (plus the client's empty-list re-asks if it raced the
    // server's registration).
    sim.step_n(60);

    let master_state = sim.state(master);
    let server_state = sim.state(server);
    let client_state = sim.state(client);
    println!("MASTER: {master_state}");
    println!("SERVER: {server_state}");
    println!("CLIENT: {client_state}");

    // The game server registered (connection-scoped) and shows up by name.
    assert!(master_state.contains("arena-1"), "master should list the server: {master_state}");
    assert!(server_state.contains("registered: true"), "server should be registered: {server_state}");
    // The client joined THROUGH discovery: it holds the discovered addr and
    // the server's Welcome — and the server counted the join.
    assert!(
        client_state.contains("Playing") && client_state.contains("hello, newcomer"),
        "client should reach Playing via the discovered server: {client_state}"
    );
    // A non-empty roster: the joined client's cid is on the list.
    assert!(
        server_state.contains("joined: [") && !server_state.contains("joined: []"),
        "server should have the client on its roster: {server_state}"
    );
}
