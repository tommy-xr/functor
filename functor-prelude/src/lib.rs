//! The Functor host prelude as Functor Lang interface files (`.funi`).
//!
//! The Functor Lang typechecker resolves host externals (`Scene.cube`, …) against
//! interface-only modules injected at load time (see
//! [`functor_lang::project::load_with_prelude`] and `docs/functor-lang-interfaces.md`). This crate holds the
//! authoritative `.funi` text for those modules and exposes it as
//! `(module name, source)` pairs — the exact shape the loader wants.
//!
//! The `.funi` here declares only TYPES; the Rust implementations live in
//! `functor_runtime_common::functor_lang_prelude::FunctorHost`. A drift test in that
//! crate keeps the two in sync.

/// The host prelude interface modules, as `(module name, .funi source)` pairs.
///
/// The module name is what qualified access uses (`Scene.cube`); it is derived
/// here explicitly rather than from a file name so the loader gets it verbatim.
pub fn modules() -> Vec<(String, String)> {
    vec![
        module("Scene", include_str!("../prelude/scene.funi")),
        module("Anim", include_str!("../prelude/anim.funi")),
        module("Angle", include_str!("../prelude/angle.funi")),
        module("Color", include_str!("../prelude/color.funi")),
        module("Vec3", include_str!("../prelude/vec3.funi")),
        module("Camera", include_str!("../prelude/camera.funi")),
        module("Frame", include_str!("../prelude/frame.funi")),
        module("Light", include_str!("../prelude/light.funi")),
        module("Fog", include_str!("../prelude/fog.funi")),
        module("Skybox", include_str!("../prelude/skybox.funi")),
        module("RenderTarget", include_str!("../prelude/render_target.funi")),
        module("Texture", include_str!("../prelude/texture.funi")),
        module("Time", include_str!("../prelude/time.funi")),
        module("Sub", include_str!("../prelude/sub.funi")),
        module("Effect", include_str!("../prelude/effect.funi")),
        module("Physics", include_str!("../prelude/physics.funi")),
        module("Ui", include_str!("../prelude/ui.funi")),
        module("AudioSource", include_str!("../prelude/audio_source.funi")),
        module("AudioScene", include_str!("../prelude/audio_scene.funi")),
    ]
}

fn module(name: &str, src: &str) -> (String, String) {
    (name.to_string(), src.to_string())
}
