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
    vec![
        module("Scene", include_str!("../prelude/scene.mlei")),
        module("Angle", include_str!("../prelude/angle.mlei")),
        module("Camera", include_str!("../prelude/camera.mlei")),
        module("Frame", include_str!("../prelude/frame.mlei")),
        module("Light", include_str!("../prelude/light.mlei")),
        module("Fog", include_str!("../prelude/fog.mlei")),
        module("Skybox", include_str!("../prelude/skybox.mlei")),
        module("RenderTarget", include_str!("../prelude/render_target.mlei")),
        module("Texture", include_str!("../prelude/texture.mlei")),
        module("Time", include_str!("../prelude/time.mlei")),
        module("Sub", include_str!("../prelude/sub.mlei")),
        module("Effect", include_str!("../prelude/effect.mlei")),
        module("Physics", include_str!("../prelude/physics.mlei")),
        module("Ui", include_str!("../prelude/ui.mlei")),
        module("AudioSource", include_str!("../prelude/audio_source.mlei")),
        module("AudioScene", include_str!("../prelude/audio_scene.mlei")),
    ]
}

fn module(name: &str, src: &str) -> (String, String) {
    (name.to_string(), src.to_string())
}
