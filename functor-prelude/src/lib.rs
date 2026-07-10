//! The Functor host prelude as Functor Lang interface files (`.functori`).
//!
//! The Functor Lang typechecker resolves host externals (`Scene.cube`, …) against
//! interface-only modules injected at load time (see
//! [`functor_lang::project::load_with_prelude`] and `docs/functor-lang-interfaces.md`). This crate holds the
//! authoritative `.functori` text for those modules and exposes it as
//! `(module name, source)` pairs — the exact shape the loader wants.
//!
//! The `.functori` here declares only TYPES; the Rust implementations live in
//! `functor_runtime_common::functor_lang_prelude::FunctorHost`. A drift test in that
//! crate keeps the two in sync.

/// The host prelude interface modules, as `(module name, .functori source)` pairs.
///
/// The module name is what qualified access uses (`Scene.cube`); it is derived
/// here explicitly rather than from a file name so the loader gets it verbatim.
pub fn modules() -> Vec<(String, String)> {
    vec![
        module("Scene", include_str!("../prelude/scene.functori")),
        module("Angle", include_str!("../prelude/angle.functori")),
        module("Camera", include_str!("../prelude/camera.functori")),
        module("Frame", include_str!("../prelude/frame.functori")),
        module("Light", include_str!("../prelude/light.functori")),
        module("Fog", include_str!("../prelude/fog.functori")),
        module("Skybox", include_str!("../prelude/skybox.functori")),
        module("RenderTarget", include_str!("../prelude/render_target.functori")),
        module("Texture", include_str!("../prelude/texture.functori")),
        module("Time", include_str!("../prelude/time.functori")),
        module("Sub", include_str!("../prelude/sub.functori")),
        module("Effect", include_str!("../prelude/effect.functori")),
        module("Physics", include_str!("../prelude/physics.functori")),
        module("Ui", include_str!("../prelude/ui.functori")),
        module("AudioSource", include_str!("../prelude/audio_source.functori")),
        module("AudioScene", include_str!("../prelude/audio_scene.functori")),
    ]
}

fn module(name: &str, src: &str) -> (String, String) {
    (name.to_string(), src.to_string())
}
