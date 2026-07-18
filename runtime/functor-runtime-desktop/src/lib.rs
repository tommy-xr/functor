#![cfg_attr(feature = "strict", deny(warnings))]

//! The desktop runtime, as a library.
//!
//! Post-E3 there is a single `functor` binary: the CLI (`cli/`) drives this
//! crate's [`run`] IN-PROCESS for `functor run/develop native`, instead of
//! spawning a separate `functor-runner` process. [`run`] owns the GLFW/OpenGL
//! window + the game loop and must be called on the main thread (see its docs).
//!
//! The game **producers** (`game`/`functor_lang_game`) are also exposed so in-process
//! drivers in OTHER crates can construct them (the `functor-netsim` harness
//! drives an [`functor_lang_game::FunctorLangGame`] as one of its instances — E3 phase 0b).

pub mod game;
pub mod functor_lang_game;

// The run loop and its supporting modules pull native-only deps (glfw, tokio,
// reqwest, rodio, tiny_http, …) declared under this crate's
// `cfg(not(wasm32))` target section, so they are gated to native builds. The
// wasm-visible surface stays exactly `game` + `functor_lang_game` (the producers).
#[cfg(not(target_arch = "wasm32"))]
mod asset_watch;
#[cfg(not(target_arch = "wasm32"))]
mod audio;
#[cfg(not(target_arch = "wasm32"))]
mod debug_server;
#[cfg(not(target_arch = "wasm32"))]
mod net_dispatch;
#[cfg(not(target_arch = "wasm32"))]
mod replay_game;
#[cfg(not(target_arch = "wasm32"))]
mod run;
// The blitz-backed HTML/CSS webview overlay (native-only: the wasm runtime
// renders the same tree as a real DOM overlay instead). `pub` for the
// headless render test in tests/.
#[cfg(not(target_arch = "wasm32"))]
pub mod webview_overlay;
#[cfg(not(target_arch = "wasm32"))]
mod ws_host;
#[cfg(not(target_arch = "wasm32"))]
mod xreal;

#[cfg(not(target_arch = "wasm32"))]
pub use run::{run, Args};

// The remote-asset fetcher install, re-exported for the end-to-end test in
// tests/remote_assets.rs (the run loops install it themselves).
#[cfg(not(target_arch = "wasm32"))]
pub use net_dispatch::install_remote_asset_fetcher;
