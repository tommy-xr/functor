// physics — the reference physics example (docs/physics.md, Phase 2c+3+4+5).
// Crates and a ball tumble onto a slab, drawn at their live simulated poses
// via Physics.transformed. Press SPACE to re-drop everything: the changed
// declared spawn poses teleport the bodies (the divergence rule) and the
// pile falls again. Press K to KICK the ball — a Physics.applyImpulse
// effect, returned beside the model, applied at the next physics step.
// Press R to fire a RAY straight down over the pile: the raycast's result
// record folds through `update`, which kicks whatever the ray hit — a query
// chaining into a command, answered against this frame's stepped world.
// Collision events (Physics.events) flash whatever the ball touches: the
// touched crate glows until the ball's next contact. Time travel is the
// SHELL's job, not the game's: open the scrubber overlay (`~` on the
// desktop runner) to pause, scrub, and rewind the whole scene — model and
// physics world together (docs/time-travel.md).
// Run with:
//
//   functor -d examples/physics run native
//
// The physics world lives host-side: edit this file while it runs and (as
// long as tags and declarations are unchanged) the bodies stay exactly
// where they were — hot reload keeps model + world.

let crateCount = 5.0

// Pseudo-scatter: a deterministic hash of crate index + drop generation, so
// every SPACE press declares fresh spawn poses without any RNG. Within a
// generation the declarations are byte-identical each frame, so the
// divergence rule leaves settled bodies alone (they can sleep).
let scatterX = (drop, i) => Math.sin(i * 12.9898 + drop * 3.7) * 1.6
let scatterZ = (drop, i) => Math.cos(i * 78.2330 + drop * 1.3) * 1.6

// Branded body identities — declared once, used as the VALUE at every site
// (declaration, reads, commands, event comparisons). noTag is the "ball not
// involved" sentinel matching the engine's zeroed-miss convention.
let groundTag = Physics.tag("ground")
let ballTag = Physics.tag("ball")
let noTag = Physics.tag("")
let crateTag = (i) => Physics.tag(Text.concat("crate-", Text.fromFloat(i)))

let crateBody = (drop, i) =>
  Physics.dynamic(crateTag(i), Physics.box(1.0, 1.0, 1.0))
    |> Physics.at(scatterX(drop, i), 3.0 + i * 1.3, scatterZ(drop, i))
    |> Physics.friction(0.6)
    |> Physics.restitution(0.2)

// One body per slot: 0 = ground, 1 = ball, the rest = crates — built with a
// literal match because Functor Lang has no List.append yet. The ball's scatter seed
// (9.0) is just a value disjoint from the crate indices 0..4. The slab sits
// with its TOP face at y = 0, so visuals on the ground plane line up with
// resting bodies.
let bodyAt = (drop, i) =>
  match i with
  | 0.0 => Physics.fixed(groundTag, Physics.box(24.0, 0.4, 24.0)) |> Physics.at(0.0, -0.2, 0.0)
  | 1.0 =>
    // Nearly-vertical drop beside the crates (the small drop-dependent wobble
    // keeps SPACE's re-drop teleport firing), so the ball settles in frame.
    (Physics.dynamic(ballTag, Physics.sphere(0.6))
      |> Physics.at(3.2 + scatterX(drop, 9.0) * 0.2, 5.5, 0.5 + scatterZ(drop, 9.0) * 0.2)
      |> Physics.restitution(0.55))
  | n => crateBody(drop, n - 2.0)

let physics = (model) =>
  Physics.scene(0.0, -9.81, 0.0,
    List.range(crateCount + 2.0) |> List.map((i) => bodyAt(model.drop, i)))

// Tints stay in 0..1 for any crateCount thanks to clamp01; the ball's
// latest touch glows (a Physics.events Contact set `hot`).
let crateVisual = (model, i) =>
  (match crateTag(i) == model.hot with
   | true => Scene.cube() |> Scene.emissive(Color.rgb(1.0, 0.45, 0.15))
   | false =>
     Scene.cube()
       |> Scene.lit(Color.rgb(Math.clamp01(0.55 + 0.09 * i), Math.clamp01(0.42 - 0.04 * i), 0.28)))
    |> Physics.transformed(crateTag(i))

// The message ADT: ctor taggers wrap each async result in its own arm —
// raycast answers and contact events share one `update` without ambiguity.
type Msg<'h, 'e> =
  | GotHit(hit: 'h)
  | Contact(ev: 'e)

let init = { drop: 0.0, spaceHeld: false, hot: noTag }

let tick = (model, dt, tts) => model

// Rising-edge detection: GLFW delivers key REPEATS as `isDown = true` too,
// so holding SPACE would re-drop every repeat without the spaceHeld latch.
// The kick fires on K's RELEASE instead — key-ups never repeat, so no latch
// is needed; arms mix plain-model and (model, effect) returns freely.
let input = (model, key, isDown) =>
  match key with
  | Key.Space =>
    (match isDown with
     | false => { model with spaceHeld: false }
     | true =>
       (match model.spaceHeld with
        | true => model
        | false => { model with drop: model.drop + 1.0, spaceHeld: true }))
  | Key.K =>
    (match isDown with
     | true => model
     | false => (model, Physics.applyImpulse(ballTag, -2.6, 5.0, -0.4)))
  | Key.R =>
    (match isDown with
     | true => model
     | false =>
       // Aim straight down over THIS generation's crate-0 spawn — effect
       // arguments are ordinary expressions of the model. The tagger is the
       // GotHit constructor: the result record arrives wrapped in Msg.
       (model,
        Physics.raycast(scatterX(model.drop, 0.0), 8.0, scatterZ(model.drop, 0.0),
                        0.0, -1.0, 0.0, 20.0, GotHit)))
  | _ => model

// Contact events name both tags in rapier's pair order — find the ball's
// partner (or noTag when the ball isn't involved).
let otherOf = (e) =>
  match e.a == ballTag with
  | true => e.b
  | false =>
    (match e.b == ballTag with
     | true => e.a
     | false => noTag)

let subscriptions = (model) => Physics.events(Contact)

// One update, two message kinds. GotHit: the raycast result, post-step
// fresh ("commands apply at the step; queries answer after it") — kick
// whatever the ray hit (the slab excluded: kicking a fixed body warns).
// Contact: flash the ball's latest touch (crates glow via `hot`).
let update = (model, msg) =>
  match msg with
  | GotHit(hit) =>
    (match hit.hit with
     | false => model
     | true =>
       (match hit.tag == groundTag with
        | true => model
        | false => (model, Physics.applyImpulse(hit.tag, 0.0, 6.0, 0.0))))
  | Contact(e) =>
    (match e.started with
     | false => model
     | true =>
       let other = otherOf(e) in
       (match other == noTag with
        | true => model
        | false =>
          // Touching the slab isn't interesting — only crates glow.
          (match other == groundTag with
           | true => model
           | false => { model with hot: other })))

let draw = (model, tts) =>
  Frame.createLit(
    Camera.lookAt(10.5, 7.5, -10.5, 0.0, 0.8, 0.0),
    Scene.group([
      Scene.plane() |> Scene.scale(24.0) |> Scene.lit(Color.rgb(0.55, 0.58, 0.62)),
      Scene.sphere() |> Scene.scale(0.6) |> Scene.lit(Color.rgb(0.95, 0.35, 0.25)) |> Physics.transformed(ballTag),
      Scene.group(List.range(crateCount) |> List.map((i) => crateVisual(model, i))),
    ]),
    [
      Light.ambient(Color.rgb(0.12, 0.12, 0.15)),
      Light.directional(0.5, -1.0, 0.35, Color.rgb(1.0, 0.98, 0.95), 0.9) |> Light.castShadows,
    ])
