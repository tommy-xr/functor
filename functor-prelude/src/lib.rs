//! The Functor host bundle: interface files (`.funi`) and reusable Functor
//! Lang implementation modules (`.fun`).
//!
//! The Functor Lang typechecker resolves host externals (`Scene.cube`, …) against
//! interface-only modules injected at load time (see
//! [`functor_lang::project::load_with_bundled_modules`] and
//! `docs/functor-lang-interfaces.md`). This crate holds the authoritative
//! engine-owned source for every target.
//!
//! The `.funi` files declare only TYPES; their Rust implementations live in
//! `functor_runtime_common::functor_lang_prelude::FunctorHost`. A drift test in that
//! crate keeps the two in sync. Bundled `.fun` files are ordinary executable
//! modules linked and evaluated by `functor-lang`.

use functor_lang::project::BundledModule;

/// The complete engine-owned Functor Lang bundle.
///
/// Hosts should pass this to the `*_with_bundled_modules` loader matching
/// their source form. Keeping interfaces and implementations together makes
/// their availability identical in native, wasm, CLI, and editor tooling.
pub fn bundled_modules() -> Vec<BundledModule> {
    let mut bundled = modules()
        .into_iter()
        .map(|(name, src)| BundledModule::interface(name, src))
        .collect::<Vec<_>>();
    bundled.push(BundledModule::implementation(
        "Animator",
        include_str!("../stdlib/animator.fun"),
    ));
    bundled
}

/// The host prelude interface modules, as `(module name, .funi source)` pairs.
///
/// The module name is what qualified access uses (`Scene.cube`); it is derived
/// here explicitly rather than from a file name so the loader gets it verbatim.
/// This interface-only view remains useful to the registry drift tests; hosts
/// should load [`bundled_modules`] instead.
pub fn modules() -> Vec<(String, String)> {
    vec![
        module("Scene", include_str!("../prelude/scene.funi")),
        module("Terrain", include_str!("../prelude/terrain.funi")),
        module("Anim", include_str!("../prelude/anim.funi")),
        module("Asset", include_str!("../prelude/asset.funi")),
        module("Angle", include_str!("../prelude/angle.funi")),
        module("Color", include_str!("../prelude/color.funi")),
        module("Vec3", include_str!("../prelude/vec3.funi")),
        module("Camera", include_str!("../prelude/camera.funi")),
        module("Camera2D", include_str!("../prelude/camera2d.funi")),
        module("Sprite", include_str!("../prelude/sprite.funi")),
        module("Frame", include_str!("../prelude/frame.funi")),
        module("Light", include_str!("../prelude/light.funi")),
        module("Fog", include_str!("../prelude/fog.funi")),
        module("Skybox", include_str!("../prelude/skybox.funi")),
        module(
            "RenderTarget",
            include_str!("../prelude/render_target.funi"),
        ),
        module("Texture", include_str!("../prelude/texture.funi")),
        module("Time", include_str!("../prelude/time.funi")),
        module("Input", include_str!("../prelude/input.funi")),
        module("Sub", include_str!("../prelude/sub.funi")),
        module("Effect", include_str!("../prelude/effect.funi")),
        module("Physics", include_str!("../prelude/physics.funi")),
        module("Ui", include_str!("../prelude/ui.funi")),
        module("Html", include_str!("../prelude/html.funi")),
        module("Attr", include_str!("../prelude/attr.funi")),
        module("Style", include_str!("../prelude/style.funi")),
        module("AudioSource", include_str!("../prelude/audio_source.funi")),
        module("AudioScene", include_str!("../prelude/audio_scene.funi")),
    ]
}

fn module(name: &str, src: &str) -> (String, String) {
    (name.to_string(), src.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use functor_lang::project::BundledModuleKind;

    #[test]
    fn engine_bundle_contains_interfaces_and_animator_implementation() {
        let bundled = bundled_modules();
        assert!(bundled.iter().any(|module| {
            module.name() == "Scene" && module.kind() == BundledModuleKind::Interface
        }));
        let animator = bundled
            .iter()
            .find(|module| module.name() == "Animator")
            .expect("Animator is distributed with every engine host");
        assert_eq!(animator.kind(), BundledModuleKind::Implementation);
        assert!(animator.source().contains("let pose"));
    }
}
