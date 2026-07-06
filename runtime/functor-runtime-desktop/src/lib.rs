//! Library facade for the desktop runtime's game producers, so in-process
//! drivers in OTHER crates can construct them (the `functor-netsim` harness
//! drives an [`mle_game::MleGame`] as one of its instances — E3 phase 0b).
//!
//! The binaries (`functor-runner` via `main.rs`, `functor-netsim-viz`) keep
//! their own module tree; this exposes only the producers an external driver
//! needs. Keep it minimal — it exists for the test/harness seam, not as the
//! runner's public API.

pub mod game;
pub mod mle_game;
