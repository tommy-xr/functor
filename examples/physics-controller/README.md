# `examples/physics-controller`

A platformer character controller where the character is a **real dynamic
physics body**, steered entirely from `tick`.

This is the counterpart to `examples/platformer`, which hand-rolls its
kinematics as a pure function and uses no physics world at all. The
interesting comparison is what each one has to write, and what it gets for
free.

```sh
functor -d examples/physics-controller test          # 39 expects (32 controller, 7 level data)
functor -d examples/physics-controller run native    # WASD / arrows, SPACE to jump, ENTER to respawn
```

## The shape of it

The whole control loop is **one frame**, and it all lives in `tick`:

```functor
let tick = (model, dt, tts) =>
  …
  let pos   = Physics.position(playerTag) in         // read  (synchronous)
  let vel   = Physics.linearVelocity(playerTag) in
  let probe = groundProbe(pos) in
  let carry = surfaceVelocity(probe) in
  let sensed = Control.sense(dt, obs, m.jumpEdge, m.ctl) in   // decide (pure)
  let want   = Control.desiredVelocity(dt, obs, carry, sensed) in
  (next, Physics.setVelocity(playerTag, Vec3.make(want.x, want.y, want.z)))  // write
```

The reads see the previous step (physics runs after `tick`) and the command
applies at the step immediately following this `tick`, so the loop closes
within one frame. That one step of read latency is inherent to any
read → decide → step simulation, not a Functor restriction.

`control.fun` holds every decision as a pure function of an *observation*, so
the entire feel layer — coyote time, jump buffering, acceleration, the ground
clamp, landing response — is covered by `expect` tests that run in
milliseconds with no GPU, no window, and no rapier world.

The character body **must** be declared `|> Physics.upright`. A dynamic capsule
that can rotate picks up angular velocity from any glancing contact and topples
— and a tipped capsule's lowest point is no longer `feetOffset` below its
center, so the grounding probe and the ground clamp both start lying. (This
attribute did not exist before this example; adding it is part of this change.)

### The one idea worth stealing

Moving-platform carry is one line, with **no platform identity, no per-platform
state, and no "which deck was I on last frame"**:

```functor
let surfaceVelocity = (probe) =>
  if probe.hit then Physics.linearVelocity(probe.tag) else zeroVelocity
```

The grounding probe already reports *which body* it hit, so asking that body
for its velocity is the whole mechanic. A fixed body reads back zero, so static
ground needs no special case, and a kinematic deck whose declared pose is
re-derived each frame reads back the velocity rapier derived from that motion.

## Verifying it without a human at the window

Two headless paths, both reproducible from a bare clone.

```sh
functor -d examples/physics-controller test

# set `traceOn = true` in game.fun for the per-frame rows the table below cites
functor -d examples/physics-controller run native \
  --input-script "$PWD/examples/physics-controller/ride-and-jump.input" \
  --script-dt 0.0166667 --capture-frame /tmp/f.png --capture-at-frame 400
```

`ride-and-jump.input` is the committed scripted run: the character falls onto a
**moving** deck, rides it with zero input, walks off its edge, jumps from the
plaza, and finally runs into a wall and stays there.

Both are deterministic. Two runs of the 400-frame script produced **identical
traces and a byte-identical capture PNG**.

## Evidence, mechanic by mechanic

Measured from the scripted run above. The capsule's rest height is 0.90 above
whatever it stands on; the plaza's top face is `y = 0` and the deck's is
`y = 1.25`, so the expected resting centers are 0.90 and 2.15.

| Mechanic | Verdict | Evidence |
| --- | --- | --- |
| Grounded detection (`castExcluding` probe) | **CLEAN** | `grounded` flips at exactly the five expected frames (11, 102, 121, 133, 169). No flicker on any seam across 400 frames. |
| Jump | **CLEAN** | Apex rise 1.1612 vs. the theoretical `v²/2g` = 1.1782 (98.6%; the deficit is one step of discretization). |
| Coyote time | **CLEAN** | Walked off the deck at f=102; `coyote` drains 0.1033 → 0.0000 over frames 102–109, i.e. exactly the 0.12 s window. |
| Jump buffering | **CLEAN** | Pure test: a press while airborne fires on the touchdown frame. |
| Landing detection / squash | **CLEAN** | Fires once per touchdown as an edge (f=11, 121, 169), never as a level; `landImpact` records 3.67 / 6.97 / 6.73. |
| Riding the moving platform | **CLEAN** | `probe.tag` → `linearVelocity` tracks the deck's analytic velocity to within 0.030 on a 2.4 u/s deck. Relative offset `x − deckX` held at −0.48 ± 0.02 over 80 frames — bounded and oscillatory, not accumulating. |
| Wall | **CLEAN, and free** | Held into the wall for 220 frames: stopped at `x = −6.3012` against a predicted −6.3000 (wall face −6.7, capsule radius 0.4) and stayed — `x` and `z` both constant to four decimals from f=260 to f=399. Zero collision code in the game. |
| Edge / ledge fall-off | **CLEAN, and free** | Walking off the deck's edge simply stops being grounded; gravity and the solver do the rest. |
| Standing height | **CLEAN, with a caveat** | A constant 0.8988 on the plaza and 2.1462–2.1488 on the deck. Getting here needed a ground clamp, a post-jump lockout, and `Physics.upright` — see below. |
| Keeping the capsule upright | **WAS IMPOSSIBLE — fixed in this change** | A rotating capsule visibly toppled (40-80° off vertical mid-jump) and then crept 0.19 units sideways along the wall with no input. No rotation lock, angular damping, or angular-velocity command existed in the `Physics` API, so this was unfixable in game code. This change adds `Physics.upright`. |

### The one thing that was genuinely impossible

**A dynamic capsule could not be kept upright.** Nothing in the `Physics`
module locked rotation, damped angular velocity, or let a game command it, so a
character body picked up spin from every glancing contact. Measured: the
capsule sat 40–80° off vertical mid-jump (it kept rotating, so the angle
depends on the frame), and once it leaned on the wall it crept
0.19 units along `z` over 140 frames with no input, still accelerating. It also
silently corrupted the controller, because a tipped capsule's lowest point is
`radius + halfHeight·cos θ` below its center rather than the fixed
`feetOffset` the probe and the ground clamp assume — which showed up as a
±0.031 ripple in the standing height.

This change adds `Physics.upright`, a nullary body attribute in the shape of
`Physics.sensor`, mapping to rapier's `LockedAxes::ROTATION_LOCKED`. With it,
the standing height is a **constant 0.8988** and the wall contact is
**motionless in both `x` and `z`**. It is the smallest surface that fixes the
problem, and it is off by default so existing bodies tumble exactly as before.

### The two things that were awkward but correct

Both are about `Physics.setVelocity` replacing **all three** velocity
components — there is no horizontal-only command — so the controller must say
something about `vy` every single frame. Two plausible answers are wrong, and
both failures were *observable*, not theoretical:

1. **Echoing the read back sank the character half a unit into the floor.**
   The read is one step stale, so while grounded it still carries the downward
   speed the solver just cancelled at the contact. Re-commanding it drives the
   capsule in. Measured: it rested at `y = 0.40` instead of 0.90. A downstream
   symptom was that the wall stop landed at `x = −5.85` instead of −6.30,
   because a half-buried capsule contacts the wall's corner rather than its
   face.

2. **Commanding `0` while grounded still sank it, slowly.** Overwriting `vy`
   every frame also destroys the contact's own pushout impulse, so each step's
   gravity increment accumulates as penetration: `y = 0.98` drifting to 0.40
   over 260 frames.

The fix is the standard character-controller **ground clamp**: while grounded,
own the standing height explicitly by correcting toward the rest distance the
probe already measured, `error / dt`, bounded. It also buys stick-to-a-
descending-deck for free.

That fix then exposed a third, equally classic problem: **the ground clamp
cancelled the jump.** For the first frames after takeoff the feet are still
within the probe's reach, so the character read as grounded and the clamp
yanked it back down — it rose 0.12 units instead of 1.16, with `vy` flipping
+6.83 → −6.37 in one frame. The fix is a short post-jump lockout during which
grounding is ignored (`jumpLockTime = 0.1`).

None of these three are Functor-specific; every engine's character controller
carries the same three counter-measures. They are called out here because they
are exactly the traps an author hits on day one, and all three are now pinned
by regression `expect`s in `control.fun`.

## Does this want a `postTick` hook?

No — and the measurement is the reason, so it is recorded here.

The argument for a `postTick` (running after the physics step, before `draw`)
is that it could react to a physics outcome with one frame less delay. Measured
against the actual landings:

- Landing on the deck: the probe first reported grounded at **f=11**, when the
  feet were still **0.137 above** the deck's surface and falling at 3.67 u/s.
  Physical contact resolved at f=13–14.
- Landing on the plaza: grounded at **f=121**, feet **0.085 above** the surface.
  Contact resolved at f=122.

The landing response therefore already fires *1–3 frames **before** physical
contact*, because the grounding probe reaches ahead of the feet by `probeSkin`.
A `postTick` would make it fire one frame **earlier still** — further ahead of
contact, not closer to it. **The sign of the dominant error term is the opposite
of what `postTick` would correct**, and `probeSkin` is a single game-owned
constant that already gives the author finer control over that timing than a
hook could.

The remaining arguments do not survive contact with the API either:

- Anything purely **visual** can already read the fresh post-step world:
  `draw` runs after the step, which is how this example's camera never trails
  the body it follows.
- Post-step **events** already have a home: `Physics.events` delivers contacts
  through `update` after the step.
- A controller is a **closed loop**, so only total loop latency matters. `tick`
  reads stale and commands into the very next step: one frame. A `postTick`
  would read fresh but its commands would wait a full frame (the write
  asymmetry), which is *worse* for responsiveness, because input arrives
  pre-step.

The genuine gaps this exercise found are not hooks at all, and neither would
have been fixed by `postTick`:

1. **No rotation lock** (fixed here as `Physics.upright`). This was the only
   *impossible* item, and it is a body attribute, not a lifecycle question.
2. **`Physics.setVelocity` is all-or-nothing across three axes**, which is what
   forced the ground clamp to be written by hand. A per-axis or
   horizontal-only velocity command would have removed both sinking failure
   modes outright. Still open, and a much smaller, better-targeted change than
   a new hook — the problem is the *shape of the command*, not when it runs.

That both real gaps turned out to be one-line API surface rather than frame
ordering is itself the argument: the `tick`-plus-synchronous-queries loop was
never the thing standing in the way.
