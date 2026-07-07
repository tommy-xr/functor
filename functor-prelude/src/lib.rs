//! The Functor host prelude as MLE interface files (`.mlei`).
//!
//! The MLE typechecker resolves host externals (`Scene.cube`, …) against
//! interface-only modules injected at load time (see
//! [`mle::project::load_with_prelude`] and `docs/mlei.md`). This crate holds the
//! authoritative `.mlei` text for those modules and exposes it as
//! `(module name, source)` pairs — the exact shape the loader wants.
//!
//! The `.mlei` here declares only TYPES; the Rust implementations live in
//! `functor_runtime_common::mle_prelude::FunctorHost`. A drift test in that
//! crate keeps the two in sync.

/// The host prelude interface modules, as `(module name, .mlei source)` pairs.
///
/// The module name is what qualified access uses (`Scene.cube`); it is derived
/// here explicitly rather than from a file name so the loader gets it verbatim.
pub fn modules() -> Vec<(String, String)> {
    vec![(
        "Scene".to_string(),
        include_str!("../prelude/scene.mlei").to_string(),
    )]
}
