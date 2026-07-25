use std::sync::{Arc, Mutex};

use functor_netsim::{InstanceId, NetSim};
use functor_runtime_common::io::{set_remote_fetcher, RemoteFetchSender};
use functor_runtime_desktop::functor_lang_game::FunctorLangGame;

pub type ControlledFetches = Arc<Mutex<Vec<(String, RemoteFetchSender)>>>;

pub fn install_controlled_fetcher() -> ControlledFetches {
    let fetches = Arc::new(Mutex::new(Vec::new()));
    let captured = fetches.clone();
    set_remote_fetcher(move |url, sender| {
        captured.lock().unwrap().push((url, sender));
    });
    fetches
}

pub fn add_source_game(
    sim: &mut NetSim,
    dir: &std::path::Path,
    source: String,
) -> InstanceId {
    std::fs::create_dir_all(dir).unwrap();
    let game = dir.join("game.fun");
    std::fs::write(&game, source).unwrap();
    sim.add_producer(Box::new(FunctorLangGame::create(
        game.to_str().unwrap(),
    )))
}
