//! The engine bundle (`functor-prelude`): reusable `.fun` modules execute,
//! host calls get real types, and the MVU `(model, effect)` lift still works
//! now that `Effect` has a concrete type instead of the old `Unknown` seam.

use std::collections::HashMap;

/// Check `src` as a single-file game with the complete engine bundle.
fn check(src: &str) -> Vec<String> {
    // Unique per CALL: tests run in parallel, and two sources of the same
    // length would otherwise share (and delete) one directory.
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "functor-prelude-typecheck-{}-{id}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("game.fun"), src).unwrap();
    let project = match functor_lang::project::load_with_bundled_modules(
        &dir.join("game.fun"),
        &HashMap::new(),
        &functor_prelude::bundled_modules(),
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
    assert!(
        diags.is_empty(),
        "effect lift should check clean: {diags:?}"
    );
}

/// …but a genuine `(model, Float)` vs `model` mismatch still errors — the lift
/// keys on the effect seam, not any tuple.
#[test]
fn real_tuple_mismatch_still_errors() {
    let diags = check("let f = (m) => match m with | true => (m, 1.0) | false => m");
    assert!(
        !diags.is_empty(),
        "a real (m, Float) vs m mismatch must error"
    );
}

/// The instancing surface typechecks as a pipeline, and its brands have
/// teeth: a bare number where an `Angle.t` or `Instance.t` belongs is a
/// check-time error.
#[test]
fn instancing_pipeline_checks_and_brands_reject() {
    let diags = check(
        "let copies: List<Instance.t> =\n\
         [Instance.at(Vec3.make(0.0, 0.0, 0.0))\n\
           |> Instance.scaleXYZ(1.0, 2.0, 1.0)\n\
           |> Instance.rotateY(Angle.degrees(45.0))\n\
           |> Instance.tint(Color.rgb(1.0, 0.5, 0.5))]\n\
         let scene: Scene.t =\n\
         Scene.cube() |> Scene.lit(Color.rgb(1.0, 1.0, 1.0)) |> Scene.instanced(copies)",
    );
    assert!(
        diags.is_empty(),
        "instancing pipeline should check: {diags:?}"
    );

    let diags = check("let bad = Instance.rotateY(45.0, Instance.at(Vec3.make(0.0, 0.0, 0.0)))");
    assert!(!diags.is_empty(), "a bare number is not an Angle.t");

    let diags = check("let bad = Scene.instanced([1.0], Scene.cube())");
    assert!(!diags.is_empty(), "a bare number is not an Instance.t");
}

/// Host calls carry real types from the prelude `.funi`, across namespaces.
#[test]
fn host_calls_have_real_types() {
    let diags = check(
        "let bad : float = Camera3D.lookAt(Vec3.make(0.0, 0.0, 0.0), Vec3.make(0.0, 0.0, 0.0))",
    );
    assert!(diags.iter().any(|m| m.contains("Camera3D.t")), "{diags:?}");
    let diags = check(
        "let bad : float =\n\
         Frame.create(Camera3D.lookAt(Vec3.make(0.0, 0.0, 0.0), Vec3.make(0.0, 0.0, 0.0)), Scene.cube())",
    );
    assert!(diags.iter().any(|m| m.contains("Frame.t")), "{diags:?}");
}

/// A camera ray is optional at the surface boundary, and its branded origin
/// and direction feed the existing Physics.cast Vec3 parameters directly.
#[test]
fn camera_world_ray_checks_as_physics_cast_input() {
    let diags = check(
        "let camera = Camera3D.lookAt(\n\
           Vec3.make(0.0, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0))\n\
         let pick = (mouse: Input.mouse): Option.t<Physics.rayHit> =>\n\
           match Camera3D.toWorldRay(mouse, camera) with\n\
           | Option.Some(ray) => Option.Some(Physics.cast(ray.origin, ray.direction, 100.0))\n\
           | Option.None => Option.None",
    );
    assert!(
        diags.is_empty(),
        "Camera3D.toWorldRay should feed Physics.cast directly: {diags:?}"
    );
}

/// Cursor rays can become model-space targets for the pure animation
/// post-pass without unpacking the Vec3 at the animation boundary.
#[test]
fn camera_world_ray_checks_as_anim_look_at_input() {
    let diags = check(
        "let camera = Camera3D.lookAt(\n\
           Vec3.make(0.0, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0))\n\
         let aim = (mouse: Input.mouse): Anim.t =>\n\
           match Camera3D.toWorldRay(mouse, camera) with\n\
           | Option.Some(ray) => Anim.rest() |> Anim.lookAt(\n\
               \"head\", ray.origin |> Vec3.add(ray.direction), Angle.degrees(80.0), 1.0)\n\
           | Option.None => Anim.rest()",
    );
    assert!(
        diags.is_empty(),
        "Camera3D.toWorldRay should feed Anim.lookAt directly: {diags:?}"
    );
}

/// The same cursor-derived model-space Vec3 can drive the stacked two-bone
/// post-pass without unpacking or weakening its brand.
#[test]
fn camera_world_ray_checks_as_anim_reach_input() {
    let diags = check(
        "let camera = Camera3D.lookAt(\n\
           Vec3.make(0.0, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0))\n\
         let reach = (mouse: Input.mouse): Anim.t =>\n\
           match Camera3D.toWorldRay(mouse, camera) with\n\
           | Option.Some(ray) => Anim.rest() |> Anim.reach(\n\
               \"upper\", \"lower\", \"hand\", ray.origin |> Vec3.add(ray.direction), 1.0)\n\
           | Option.None => Anim.rest()",
    );
    assert!(
        diags.is_empty(),
        "Camera3D.toWorldRay should feed Anim.reach directly: {diags:?}"
    );
}

/// Engine-owned `.fun` modules participate in the same typecheck as the host
/// interfaces they build upon.
#[test]
fn animator_is_available_without_a_project_sibling() {
    let diags = check(
        "let state = Animator.start(\"idle\", 0.0)\n\
         let next = Animator.play(\"run\", 1.0, state)\n\
         let pose : Anim.t = Animator.pose(next, 0.5, 1.25)",
    );
    assert!(
        diags.is_empty(),
        "bundled Animator should check clean: {diags:?}"
    );
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

/// Since the flag day (B.6), asset consumers take the branded Asset kinds:
/// Asset values check clean, and the retired bare-string coercion is a
/// CHECK-TIME error — stronger than the pre-B.1 stringly-typed surface.
#[test]
fn asset_consumers_take_asset_values_only() {
    let diags = check(
        "let byAsset = Scene.model(Asset.model(\"shark.glb\"))\n\
         let tex = Scene.plane() |> Scene.litTexture(Asset.texture(\"wood.png\"))\n\
         let texFile = Scene.plane() |> Scene.litTexture(Texture.file(\"wood.png\"))\n\
         let sfx = Effect.play(Asset.sound(\"boom.ogg\"))\n\
         let bed = AudioSource.ambient(\"bed\", Asset.sound(\"wind.ogg\"))",
    );
    assert!(
        diags.is_empty(),
        "asset forms should check clean: {diags:?}"
    );

    // The retired coercions fail the CHECK, naming the Asset kind.
    let diags = check("let s = Scene.model(\"shark.glb\")");
    assert!(
        diags.iter().any(|m| m.contains("Model")),
        "bare model path must be a check error: {diags:?}"
    );
    let diags = check("let s = Effect.play(\"boom.ogg\")");
    assert!(
        diags.iter().any(|m| m.contains("Sound")),
        "bare sound path must be a check error: {diags:?}"
    );
    let diags = check("let s = AudioSource.ambient(\"bed\", \"wind.ogg\")");
    assert!(
        diags.iter().any(|m| m.contains("Sound")),
        "bare soundscape path must be a check error: {diags:?}"
    );
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
    assert!(
        diags.is_empty(),
        "whilePending should check clean: {diags:?}"
    );
}

/// `Effect.preload`/`preloadThen` check clean with Asset values and produce
/// Effect.t (usable in the (model, effect) seam).
#[test]
fn preload_checks_clean() {
    let diags = check(
        "type Msg = | Warm\n\
         let boss = Asset.model(\"boss.glb\")\n\
         let a = Effect.preload(boss)\n\
         let b = Effect.preloadThen(boss, Warm)\n\
         let c = Effect.batch([a, b])\n\
         let update = (m, msg) => match msg with | Warm => (m, Effect.preload(Asset.texture(\"wood.png\")))",
    );
    assert!(diags.is_empty(), "preload should check clean: {diags:?}");
}

/// The `Vec3` accessors and arithmetic typecheck in both the direct-call and
/// the thread-last pipeline spelling, and compose with the prelude consumers
/// with no unpack-and-rebuild at the boundary.
#[test]
fn vec3_arithmetic_checks_clean() {
    let diags = check(
        "let origin = Vec3.make(0.0, 0.0, 0.0)\n\
         let a = Vec3.make(1.0, 2.0, 3.0)\n\
         let b = Vec3.make(4.0, 5.0, 6.0)\n\
         let sum = a |> Vec3.add(b)\n\
         let diff = a |> Vec3.sub(origin)\n\
         let scaled = diff |> Vec3.scale(2.0)\n\
         let unit = scaled |> Vec3.normalize()\n\
         let d: float = a |> Vec3.dot(b)\n\
         let perp = a |> Vec3.cross(b)\n\
         let len: float = unit |> Vec3.length()\n\
         let dist: float = a |> Vec3.distance(b)\n\
         let mid = a |> Vec3.lerp(b, 0.5)\n\
         let cx: float = Vec3.x(sum)\n\
         let cy: float = Vec3.y(sum)\n\
         let cz: float = Vec3.z(sum)\n\
         let total = cx + cy + cz + d + len + dist\n\
         let scene = Scene.cube() |> Scene.translate(perp) |> Scene.scale(total)\n\
         let draw = (m, tts) => Frame.create(Camera3D.lookAt(mid, unit), scene)",
    );
    assert!(
        diags.is_empty(),
        "Vec3 arithmetic should check clean: {diags:?}"
    );
}

/// The brand is still enforced STATICALLY: a bare number or a structural
/// `{x, y, z}` record where a Vec3 belongs stays a check-time error, so the
/// arithmetic did not open a coercion hole at the prelude boundary.
#[test]
fn vec3_arithmetic_rejects_unbranded_values_at_check_time() {
    assert!(
        !check("let bad = Vec3.length(1.0)").is_empty(),
        "a bare number must not check as a Vec3"
    );
    assert!(
        !check("let bad = Vec3.add({ x: 1.0, y: 2.0, z: 3.0 }, Vec3.make(0.0, 0.0, 0.0))")
            .is_empty(),
        "a bare record must not check as a Vec3"
    );
    assert!(
        !check("let bad: float = Vec3.make(1.0, 2.0, 3.0) |> Vec3.normalize()").is_empty(),
        "normalize returns a Vec3, not a float"
    );
}

// ------------------------------------------------------ unit-suffix literals

/// The prelude's own `unit` declarations resolve under the engine bundle: a
/// suffixed literal IS the branded call, so it checks as the branded type.
#[test]
fn builtin_unit_suffixes_check_as_their_branded_types() {
    for src in [
        "let turn: Angle.t = 90deg",
        "let turn: Angle.t = 0.5rad",
        "let d: Time.t = 0.5s",
        "let d: Time.t = 500ms",
        "let d: Time.t = 250us",
        "let d: Time.t = 2min",
        "let d: Time.t = 1hr",
    ] {
        assert!(check(src).is_empty(), "{src}: {:?}", check(src));
    }
    // …and they are branded, not floats.
    assert!(
        check("let bad: float = 90deg")
            .iter()
            .any(|m| m.contains("Angle.t")),
        "{:?}",
        check("let bad: float = 90deg")
    );
}

/// A suffixed literal satisfies a branded PARAMETER exactly like the
/// handwritten call it desugars to.
#[test]
fn unit_suffixes_satisfy_branded_parameters() {
    let diags = check(
        "type Msg = | Pulse\n\
         let subscriptions = (m) => Sub.every(0.5s, Pulse)\n\
         let update = (m, msg) => m\n\
         let scene = Scene.cube() |> Scene.rotateY(90deg)",
    );
    assert!(diags.is_empty(), "{diags:?}");
}

/// A bare number in a branded position teaches BOTH spellings now — the
/// suffix and the constructor call.
#[test]
fn a_bare_number_in_a_branded_position_teaches_the_suffix() {
    let diags = check("let scene = Scene.cube() |> Scene.rotateY(90.0)");
    assert!(
        diags.iter().any(|m| m.contains("write `90deg`")),
        "{diags:?}"
    );
}

/// An undeclared suffix lists the ones the prelude does declare.
#[test]
fn an_unknown_suffix_lists_the_prelude_units() {
    let diags = check("let bad = 90degrees");
    assert!(
        diags.iter().any(|m| m.contains("unknown unit `degrees`")
            && m.contains("`deg`")
            && m.contains("`ms`")),
        "{diags:?}"
    );
}

/// A project may declare its OWN unit beside the prelude's — units are
/// project-wide and compose.
#[test]
fn a_project_unit_lives_beside_the_prelude_units() {
    let diags = check(
        "type Px = | Px(value: float)\n\
         unit px = Px\n\
         let width: Px = 16px\n\
         let turn: Angle.t = 45deg",
    );
    assert!(diags.is_empty(), "{diags:?}");
}

/// Branded arithmetic under the real prelude: the declarations in
/// `angle.funi` / `time.funi` make `+`, `-`, and scalar `*` typecheck on
/// angles and durations — across suffixes, since an operator belongs to the
/// brand.
#[test]
fn prelude_brands_carry_their_declared_arithmetic() {
    let diags = check(
        "let turn: Angle.t = 90deg + 45deg\n\
         let back: Angle.t = 90deg - 1.5rad\n\
         let wide: Angle.t = 45deg * 2.0\n\
         let also: Angle.t = 2.0 * 45deg\n\
         let wait: Time.t = 1.5s - 200ms\n\
         let long: Time.t = 2min + 30s\n\
         let twice: Time.t = 0.5s * 2.0\n",
    );
    assert!(diags.is_empty(), "{diags:?}");
}

/// Branded COMPARISON under the real prelude: `angle.funi` / `time.funi`
/// declare `==` and `<`, and the four derived spellings follow — across
/// suffixes, since an operator belongs to the brand.
#[test]
fn prelude_brands_compare_and_order() {
    let diags = check(
        "let a: bool = 90deg == 90deg\n\
         let b: bool = 90deg != 45deg\n\
         let c: bool = 45deg < 1.5rad\n\
         let d: bool = 90deg > 45deg\n\
         let e: bool = 45deg <= 45deg\n\
         let f: bool = 90deg >= 45deg\n\
         let g: bool = 1.5s < 2000ms\n\
         let h: bool = 1s == 1000ms\n\
         let i: bool = 2min > 90s\n\
         let j: bool = (90deg + 45deg) > 90deg\n",
    );
    assert!(diags.is_empty(), "{diags:?}");
    // Brands still do not mix: an angle is not a duration.
    let diags = check("let bad: bool = 90deg < 1.5s\n");
    assert!(
        !diags.is_empty(),
        "an angle below a duration must not check"
    );
}

/// Half 2: an ENGINE value has no structural equality — the runtime refuses
/// it — so `==` on one is a CHECK-time error naming the type.
#[test]
fn equality_on_an_engine_value_is_a_check_error() {
    for (src, ty) in [
        ("let bad: bool = Scene.cube() == Scene.cube()\n", "Scene.t"),
        (
            "let bad: bool = Color.rgb(1.0, 0.0, 0.0) == Color.rgb(1.0, 0.0, 0.0)\n",
            "Color.t",
        ),
        (
            "let bad: bool = Vec3.make(0.0, 0.0, 0.0) != Vec3.make(1.0, 0.0, 0.0)\n",
            "Vec3.t",
        ),
    ] {
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|m| m.contains("engine values are opaque") && m.contains(ty)),
            "{src}: {diags:?}"
        );
    }
}

/// The two pure-data engine values offer an explicit structural walk instead,
/// so their refusal names it. Every other opaque type has no such hatch and
/// keeps the plain message.
#[test]
fn scene_and_frame_refusals_point_at_equals() {
    for (src, hint) in [
        (
            "let bad: bool = Scene.cube() == Scene.cube()\n",
            "`Scene.equals(a, b)` compares structurally",
        ),
        (
            "let camera = Camera3D.lookAt(Vec3.make(0.0, 1.0, -4.0), Vec3.make(0.0, 0.0, 0.0))\n\
             let bad: bool = Frame.create(camera, Scene.cube()) == Frame.create(camera, Scene.cube())\n",
            "`Frame.equals(a, b)` compares structurally",
        ),
    ] {
        let diags = check(src);
        assert!(diags.iter().any(|m| m.contains(hint)), "{src}: {diags:?}");
    }
    // A type with no `equals` is unchanged.
    let diags = check("let bad: bool = Color.rgb(1.0, 0.0, 0.0) == Color.rgb(1.0, 0.0, 0.0)\n");
    assert!(
        !diags.iter().any(|m| m.contains("compares structurally")),
        "{diags:?}"
    );
}

/// The escape hatch itself checks clean and answers `bool`.
#[test]
fn scene_and_frame_equals_check_clean() {
    let diags = check(
        "let camera = Camera3D.lookAt(Vec3.make(0.0, 1.0, -4.0), Vec3.make(0.0, 0.0, 0.0))\n\
         let sameScene: bool = Scene.equals(Scene.cube(), Scene.cube())\n\
         let sameFrame: bool = Frame.equals(Frame.create(camera, Scene.cube()),\n\
                                            Frame.create(camera, Scene.cube()))\n",
    );
    assert!(diags.is_empty(), "{diags:?}");
}

/// The rule walks what the sibling "functions cannot be compared" rule walks
/// — nested tuples, lists, maps, and a nominal's type arguments — so a host
/// value one level in is caught too. [xreview: Codex Medium, Claude Medium]
#[test]
fn equality_on_a_nested_engine_value_is_a_check_error_too() {
    for (src, ty) in [
        (
            "let bad: bool = (Scene.cube(), 1.0) == (Scene.cube(), 1.0)\n",
            "Scene.t",
        ),
        (
            "let bad: bool = ((Scene.cube(), 1.0), true) == ((Scene.cube(), 1.0), true)\n",
            "Scene.t",
        ),
        (
            "let bad: bool = [Scene.cube()] == [Scene.cube()]\n",
            "Scene.t",
        ),
        // A brand that DECLARES `==` is comparable as an operand, but not
        // nested: the structural walk never consults the brand table, so
        // this really is a certain runtime error. [xreview: Claude High]
        ("let bad: bool = (90deg, 1.0) == (90deg, 1.0)\n", "Angle.t"),
        ("let bad: bool = [1.5s] == [1.5s]\n", "Time.t"),
    ] {
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|m| m.contains("engine values are opaque") && m.contains(ty)),
            "{src}: {diags:?}"
        );
    }
    // …while the same brands still compare fine as OPERANDS.
    assert!(
        check("let a: bool = 90deg == 90deg\nlet b: bool = 1.5s == 1.5s\n").is_empty(),
        "a branded operand must stay comparable"
    );
}

/// The KNOWN limit, pinned deliberately rather than left as a surprise:
/// equality is polymorphic, so a generic helper's `a == b` carries no
/// constraint to its call sites. `same(Scene.cube(), Scene.cube())` therefore
/// still checks clean and meets the RUNTIME error — the gradual-seam backstop.
/// Closing this needs constraint-based typeclasses, which is a much larger
/// design than this change. [xreview: Codex High — accepted limitation]
#[test]
fn polymorphic_equality_is_not_constrained_by_the_opacity_rule() {
    let diags = check(
        "let same = (a, b) => a == b\n\
         let engines: bool = same(Scene.cube(), Scene.cube())\n",
    );
    assert!(
        diags.is_empty(),
        "an indirect comparison is NOT caught statically (yet): {diags:?}"
    );
}

/// …but the brands that DECLARE `==` are exempt, which is the whole point of
/// declaring it — and `Physics.tag`, a brand over a STRING, was never opaque
/// and keeps comparing structurally. Games read collision events that way
/// (`examples/physics`, `examples/physics-controller`), so this is the
/// regression that must never fire.
#[test]
fn brands_and_tags_still_compare() {
    let diags = check(
        "let angles: bool = 90deg == 90deg\n\
         let times: bool = 1s != 500ms\n\
         let ball = Physics.tag(\"ball\")\n\
         let hit: bool = ball == Physics.tag(\"ball\")\n\
         let miss: bool = ball != Physics.tag(\"wall\")\n",
    );
    assert!(diags.is_empty(), "{diags:?}");
}

/// …and what they do NOT declare still teaches, now naming what exists.
#[test]
fn an_undeclared_prelude_operator_names_the_declared_ones() {
    let diags = check("let bad: Angle.t = 90deg / 2.0\n");
    assert!(
        diags
            .iter()
            .any(|m| m.contains("`Angle.t` declares") && m.contains("but not `/`")),
        "{diags:?}"
    );
    // Mixing brands is still a type error: an angle is not a duration.
    let diags = check("let bad: Angle.t = 90deg + 1.5s\n");
    assert!(!diags.is_empty(), "an angle plus a duration must not check");
}

/// The prelude's `unit … (<op>)` declarations must LOAD under the plain,
/// hostless interpreter — the editor's expect gutter runs a project's defs
/// that way with the engine `.funi` interfaces linked, so an operator that
/// resolved its brand or its implementation eagerly failed every load with
/// ``unknown external `Angle.degrees` ``. Nothing about a unit operator may
/// need a host until it actually dispatches.
#[test]
fn prelude_unit_operators_load_under_the_plain_interpreter() {
    let hostless = |src: &str| {
        let project = functor_lang::project::load_sources_with_bundled_modules(
            vec![(std::path::PathBuf::from("game.fun"), src.to_string())],
            &functor_prelude::bundled_modules(),
        )
        .unwrap_or_else(|e| panic!("loads: {}", e.message));
        functor_lang::run_expects_budgeted(
            &project.module,
            &mut functor_lang::NoHost,
            Some(1_000_000),
        )
        .map(|reports| {
            reports
                .iter()
                .map(|report| report.outcome.status().0.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|failure| panic!("defs must load hostlessly: {}", failure.error.message))
    };

    assert_eq!(
        hostless("let area = (w, h) => w * h\nexpect area(3.0, 4.0) == 12.0\n"),
        ["pass"]
    );

    // …and a USER brand's operators still dispatch there, because building one
    // of its values needs no host: the hostless path degrades only where it
    // must.
    assert_eq!(
        hostless(
            "type Px = | Px(value: float)\n\
             unit px = Px\n\
             let unwrap = (p: Px): float => match p with | Px(n) => n\n\
             unit px (+) = (a, b) => Px(unwrap(a) + unwrap(b))\n\
             expect unwrap(16px + 4px) == 20.0\n"
        ),
        ["pass"]
    );
}

/// The cost of ad-hoc overloading, pinned deliberately: because the prelude
/// always declares `+`/`-`/`*` on `Angle.t` and `Time.t`, a helper whose
/// operands inference never pins down is now ambiguous and asks for an
/// annotation instead of silently defaulting to float. This is a BREAKING
/// change for existing unannotated helpers (`docs/functor-lang-units.md`), so
/// it is a test rather than a surprise.
#[test]
fn a_fully_unannotated_helper_asks_for_an_annotation() {
    let diags = check("let plus = (a, b) => a + b\n");
    assert!(
        diags
            .iter()
            .any(|m| m.contains("could be float arithmetic") && m.contains("annotate an operand")),
        "{diags:?}"
    );
    // Any of the ordinary ways of pinning a type resolves it.
    for src in [
        "let plus = (a: float, b) => a + b\n",
        "let plus = (a, b): float => a + b\n",
        "let bump = (a) => a + 1.0\n",
        "let square = (v) => v * v\n",
    ] {
        let diags = check(src);
        assert!(diags.is_empty(), "{src}: {diags:?}");
    }
}

/// A project declares operators on its OWN brand beside the prelude's, with
/// lambda implementations (a `.fun` may write bodies; a `.funi` may not).
#[test]
fn a_project_declares_operators_on_its_own_brand() {
    let diags = check(
        "type Px = | Px(value: float)\n\
         unit px = Px\n\
         let unwrap = (p: Px): float => match p with | Px(n) => n\n\
         unit px (+) = (a, b) => Px(unwrap(a) + unwrap(b))\n\
         let total: Px = 16px + 4px\n\
         let turn: Angle.t = 45deg + 45deg\n",
    );
    assert!(diags.is_empty(), "{diags:?}");
}

/// Redeclaring a prelude suffix is refused — one meaning per suffix, project
/// wide, exactly like a constructor name.
#[test]
fn redeclaring_a_prelude_suffix_is_an_error() {
    let diags = check(
        "type Px = | Px(value: float)\n\
         unit deg = Px\n",
    );
    assert!(
        diags.iter().any(|m| m.contains("duplicate unit `deg`")),
        "{diags:?}"
    );
}
