//! The host prelude (`functor-prelude`) as a check-time overlay (functori 2e):
//! host calls get real types, and the MVU `(model, effect)` lift still works
//! now that `Effect` has a concrete type instead of the old `Unknown` seam.

use std::collections::HashMap;

/// Check `src` as a single-file game WITH the host prelude injected.
fn check(src: &str) -> Vec<String> {
    let dir =
        std::env::temp_dir().join(format!("functor-prelude-typecheck-{}", src.len()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("game.functor"), src).unwrap();
    let project = match functor_lang::project::load_with_prelude(
        &dir.join("game.functor"),
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

/// Host calls carry real types from the prelude `.functori`, across namespaces.
#[test]
fn host_calls_have_real_types() {
    let diags = check("let bad : float = Camera.lookAt(0.0, 0.0, 0.0, 0.0, 0.0, 0.0)");
    assert!(diags.iter().any(|m| m.contains("Camera.t")), "{diags:?}");
    let diags = check(
        "let bad : float =\n\
         Frame.create(Camera.lookAt(0.0, 0.0, 0.0, 0.0, 0.0, 0.0), Scene.cube())",
    );
    assert!(diags.iter().any(|m| m.contains("Frame.t")), "{diags:?}");
}
