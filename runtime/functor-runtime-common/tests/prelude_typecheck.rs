//! The host prelude (`functor-prelude`) as a check-time overlay (funi 2e):
//! host calls get real types, and the MVU `(model, effect)` lift still works
//! now that `Effect` has a concrete type instead of the old `Unknown` seam.

use std::collections::HashMap;

/// Check `src` as a single-file game WITH the host prelude injected.
fn check(src: &str) -> Vec<String> {
    let dir =
        std::env::temp_dir().join(format!("functor-prelude-typecheck-{}", src.len()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("game.fun"), src).unwrap();
    let project = match functor_lang::project::load_with_prelude(
        &dir.join("game.fun"),
        &HashMap::new(),
        &functor_prelude::modules(),
    ) {
        Ok(project) => project,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&dir);
            return vec![format!("LOAD: {}", e.render())];
        }
    };
    let diags = project.check().into_iter().map(|d| d.message).collect();
    let _ = std::fs::remove_dir_all(&dir);
    diags
}

/// The MVU bare-model lift: an arm returning `m` beside one returning
/// `(m, effect)` joins as the pair — even though `Effect` is now a real type,
/// not `Unknown` (the regression `is_effect_seam` fixes).
#[test]
fn effect_returning_update_checks_clean() {
    let diags = check(
        "let update = (m, msg) =>\n\
         match msg with | true => (m, Effect.none()) | false => m",
    );
    assert!(diags.is_empty(), "effect lift should check clean: {diags:?}");
}

/// …but a genuine `(model, Float)` vs `model` mismatch still errors — the lift
/// keys on the effect seam, not any tuple.
#[test]
fn real_tuple_mismatch_still_errors() {
    let diags = check("let f = (m) => match m with | true => (m, 1.0) | false => m");
    assert!(!diags.is_empty(), "a real (m, Float) vs m mismatch must error");
}

/// Host calls carry real types from the prelude `.funi`, across namespaces.
#[test]
fn host_calls_have_real_types() {
    let diags = check("let bad : float = Camera.lookAt(Vec3.make(0.0, 0.0, 0.0), Vec3.make(0.0, 0.0, 0.0))");
    assert!(diags.iter().any(|m| m.contains("Camera.t")), "{diags:?}");
    let diags = check(
        "let bad : float =\n\
         Frame.create(Camera.lookAt(Vec3.make(0.0, 0.0, 0.0), Vec3.make(0.0, 0.0, 0.0)), Scene.cube())",
    );
    assert!(diags.iter().any(|m| m.contains("Frame.t")), "{diags:?}");
}

// --- typed assets (Track B.1) ---

/// The `Asset` constructors are fully typed: a non-string argument is a
/// check-time diagnostic, and each kind's annotation holds.
#[test]
fn asset_constructors_are_typed() {
    let diags = check("let a = Asset.model(42.0)");
    assert!(!diags.is_empty(), "Asset.model(42.0) must be a check error");

    let diags = check("let a : Asset.Model = Asset.model(\"barrel.glb\")");
    assert!(diags.is_empty(), "kind annotation should hold: {diags:?}");

    // A kind mismatch is a check-time error — the whole point of the brand.
    let diags = check("let a : Asset.Sound = Asset.model(\"barrel.glb\")");
    assert!(
        diags.iter().any(|m| m.contains("Model")),
        "Asset.Model vs Asset.Sound must error: {diags:?}"
    );
}

/// During the typed-asset migration, asset-consuming functions accept BOTH
/// the bare path string (deprecated at the flag day) and the branded Asset
/// value — their asset parameter is gradually typed, so both forms check
/// clean; the host enforces kinds at runtime.
#[test]
fn asset_consumers_accept_both_forms() {
    let diags = check(
        "let byString = Scene.model(\"shark.glb\")\n\
         let byAsset = Scene.model(Asset.model(\"shark.glb\"))\n\
         let tex = Scene.plane() |> Scene.litTexture(Asset.texture(\"wood.png\"))\n\
         let texFile = Scene.plane() |> Scene.litTexture(Texture.file(\"wood.png\"))\n\
         let sfx = Effect.play(Asset.sound(\"boom.ogg\"))\n\
         let sfxString = Effect.play(\"boom.ogg\")\n\
         let bed = AudioSource.ambient(\"bed\", Asset.sound(\"wind.ogg\"))",
    );
    assert!(diags.is_empty(), "both forms should check clean: {diags:?}");
}

/// `Asset.whilePending` is gradually typed but ties its result to the asset
/// argument, so a chained locator still flows into `Scene.model` cleanly.
#[test]
fn while_pending_checks_clean_in_both_positions() {
    let diags = check(
        "let proxy = Asset.model(\"low.glb\")\n\
         let boss = Asset.model(\"boss.glb\") |> Asset.whilePending(proxy)\n\
         let scene = Scene.model(boss)\n\
         let tex = Asset.texture(\"wood.png\") |> Asset.whilePending(Asset.texture(\"grey.png\"))\n\
         let mat = Scene.plane() |> Scene.litTexture(tex)",
    );
    assert!(diags.is_empty(), "whilePending should check clean: {diags:?}");
}
