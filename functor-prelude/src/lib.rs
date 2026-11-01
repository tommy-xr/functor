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
    bundled.push(BundledModule::implementation(
        "Decode",
        include_str!("../stdlib/decode.fun"),
    ));
    bundled.push(BundledModule::implementation(
        "Encode",
        include_str!("../stdlib/encode.fun"),
    ));
    bundled.push(BundledModule::implementation(
        "Codec",
        include_str!("../stdlib/codec.fun"),
    ));
    bundled.push(BundledModule::implementation(
        "Connector",
        include_str!("../stdlib/connector.fun"),
    ));
    bundled.push(BundledModule::implementation(
        "Fit",
        include_str!("../stdlib/fit.fun"),
    ));
    bundled.push(BundledModule::implementation(
        "Hardware",
        include_str!("../stdlib/hardware.fun"),
    ));
    bundled.push(BundledModule::implementation(
        "Thread",
        include_str!("../stdlib/thread.fun"),
    ));
    bundled.push(BundledModule::implementation(
        "OrbitCamera",
        include_str!("../stdlib/orbit_camera.fun"),
    ));
    bundled.push(BundledModule::implementation(
        "PartViewer",
        include_str!("../stdlib/part_viewer.fun"),
    ));
    bundled.push(BundledModule::implementation(
        "PartViews",
        include_str!("../stdlib/part_views.fun"),
    ));
    bundled.push(BundledModule::implementation(
        "PartReview",
        include_str!("../stdlib/part_review.fun"),
    ));
    // A file that is only a `part` runs in the PartViewer.
    bundled.push(BundledModule::entry_default(
        "PartViewerDefault",
        "part",
        include_str!("../stdlib/part_viewer_default.fun"),
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
        module("Instance", include_str!("../prelude/instance.funi")),
        module("Terrain", include_str!("../prelude/terrain.funi")),
        module("Solid", include_str!("../prelude/solid.funi")),
        module("Sketch", include_str!("../prelude/sketch.funi")),
        module("Typeface", include_str!("../prelude/typeface.funi")),
        module("Bytes", include_str!("../prelude/bytes.funi")),
        module("Binary", include_str!("../prelude/binary.funi")),
        module("File", include_str!("../prelude/file.funi")),
        module("Path", include_str!("../prelude/path.funi")),
        module("Mesh", include_str!("../prelude/mesh.funi")),
        module("Cells", include_str!("../prelude/cells.funi")),
        module("Model", include_str!("../prelude/model.funi")),
        module("Anim", include_str!("../prelude/anim.funi")),
        module("Asset", include_str!("../prelude/asset.funi")),
        module("Angle", include_str!("../prelude/angle.funi")),
        module("Color", include_str!("../prelude/color.funi")),
        module("Vec3", include_str!("../prelude/vec3.funi")),
        module("Camera3D", include_str!("../prelude/camera3d.funi")),
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
        module("Lightmap", include_str!("../prelude/lightmap.funi")),
        module("Time", include_str!("../prelude/time.funi")),
        module("Input", include_str!("../prelude/input.funi")),
        module("Sub", include_str!("../prelude/sub.funi")),
        module("Effect", include_str!("../prelude/effect.funi")),
        module("Persistence", include_str!("../prelude/persistence.funi")),
        module("Physics", include_str!("../prelude/physics.funi")),
        module("Ui", include_str!("../prelude/ui.funi")),
        module("Html", include_str!("../prelude/html.funi")),
        module("Attr", include_str!("../prelude/attr.funi")),
        module("Style", include_str!("../prelude/style.funi")),
        module("AudioSource", include_str!("../prelude/audio_source.funi")),
        module("AudioScene", include_str!("../prelude/audio_scene.funi")),
        module("Xr", include_str!("../prelude/xr.funi")),
        module("Source", include_str!("../prelude/source.funi")),
        module("Config", include_str!("../prelude/config.funi")),
        module("Settings", include_str!("../prelude/settings.funi")),
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

    /// `reads::CLOSURE_HOLDERS` names the host types that can hold a
    /// function: each is a prelude type, and a built-in taking a function or
    /// a holder returns a holder or a type variable, unless it keeps none of it
    /// (the memo builders only call theirs). Type-variable taggers
    /// (`Ui.slider`'s) aren't seen here; their built-ins return holders.
    #[test]
    fn closure_holders_cover_every_builtin_that_keeps_a_function() {
        use functor_lang::ast::TypeName;
        use functor_lang::reads::CLOSURE_HOLDERS;
        // Built-ins that take a function or holder but keep none of it.
        const KEEPS_NONE: [&str; 3] = ["Scene.memo", "Physics.memo", "Frame.equals"];
        fn mentions(ty: &TypeName, hit: &dyn Fn(&str) -> bool) -> bool {
            hit(&ty.name) || ty.args.iter().any(|arg| mentions(arg, hit))
        }
        let sources = vec![("game.fun".into(), "let main = () => 1.0\n".to_string())];
        let project = functor_lang::project::load_sources_with_bundled_modules(
            sources,
            &bundled_modules(),
        )
        .unwrap_or_else(|e| panic!("{}", e.render()));
        let module = &project.module;
        for holder in CLOSURE_HOLDERS {
            assert!(
                module.types.iter().any(|decl| decl.name == holder),
                "`{holder}` is not a prelude type"
            );
        }
        for sig in &module.signatures {
            let Some((ret, params)) = (sig.ty.name == "=>")
                .then(|| sig.ty.args.split_last())
                .flatten()
            else {
                continue;
            };
            let holder = |name: &str| CLOSURE_HOLDERS.contains(&name);
            let may_hold = |name: &str| holder(name) || name.starts_with('\'');
            let takes_fn = params
                .iter()
                .any(|p| mentions(p, &|name| name == "=>" || holder(name)));
            if takes_fn && !KEEPS_NONE.contains(&sig.name.as_str()) {
                assert!(
                    mentions(ret, &may_hold),
                    "`{}` takes a function or holder but returns no holder or type variable — \
add its type there, or to KEEPS_NONE if it keeps none of it",
                    sig.name
                );
            }
        }
    }
}
