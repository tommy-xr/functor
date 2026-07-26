// world.fun — the level, as pure data.
//
// A slab is a box given by its CENTER and its full extents, which is what
// `Physics.box` wants; `top` derives the surface the player actually stands on.
// The moving deck's pose is an analytic function of the game clock, so the
// `physics` hook can re-declare it every frame without storing any per-platform
// state (the runtime drives a kinematic body toward each newly declared pose,
// and derives its velocity from that motion — which is what the controller's
// ground probe reads back as `carry`).

type Slab = { x: float, y: float, z: float, w: float, h: float, d: float }

// The plaza. Its top face is exactly y = 0, so ground-plane visuals line up
// with resting bodies.
let ground: Slab = { x: 0.0, y: -0.5, z: 0.0, w: 40.0, h: 1.0, d: 40.0 }

// A static platform to jump onto.
let ledge: Slab = { x: 9.0, y: 1.7, z: 0.0, w: 6.0, h: 0.6, d: 6.0 }

// A wall to run into. With a dynamic capsule this needs no controller code at
// all — the solver refuses the penetration.
let wall: Slab = { x: -7.0, y: 1.5, z: 0.0, w: 0.6, h: 3.0, d: 12.0 }

// The moving deck: a kinematic slab sliding along X.
let deckHome: Slab = { x: 0.0, y: 1.0, z: -8.0, w: 8.0, h: 0.5, d: 5.0 }
let deckTravel = 4.0
let deckOmega = 0.6

// The deck's center X at time t, and its analytic velocity — the value the
// controller should observe coming back out of `Physics.linearVelocity`.
let deckX = (t) => deckHome.x + deckTravel * Math.sin(deckOmega * t)
let deckVx = (t) => deckTravel * deckOmega * Math.cos(deckOmega * t)

let deckAt = (t) => { deckHome with x: deckX(t) }

// The player spawns just above the deck's home pose, so the run starts with a
// short fall onto a moving surface: a landing and a carry in the first second.
let spawn = { x: 0.0, y: 2.6, z: -8.0 }

// A slab's standing surface.
let top = (s: Slab) => s.y + s.h / 2.0

// Fall out of the world and respawn.
let voidY = -12.0

expect top(ground) == 0.0
expect top(deckHome) == 1.25

// The deck starts at its home X, moving at full speed (the derivative of a
// sine at zero is its maximum), so the carry is exercised immediately.
expect deckX(0.0) == 0.0
expect deckVx(0.0) == deckTravel * deckOmega

// A quarter period later it is at full travel and momentarily at rest.
expect (
  let quarter = Math.pi / 2.0 / deckOmega in
  Math.abs(deckX(quarter) - deckTravel) < 0.0001
    && Math.abs(deckVx(quarter)) < 0.0001
)

// The player spawns above the deck's standing surface, so the run begins with
// a fall onto it rather than beside it.
expect spawn.y > top(deckHome)
expect spawn.x == deckX(0.0)
