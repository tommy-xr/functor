# `examples/vr-lab`

A VR interaction sandbox: a hub with one station per skill. Each station is a
module and an entry in `stations.fun`; a number key jumps to it and resets it.

```sh
functor -d examples/vr-lab run vr
functor -d examples/vr-lab run native            # the station from eye height, no hands
functor -d examples/vr-lab run native --emulate-xr --xr-eye-height 1.3
                                                 # mouse moves the right hand, Enter squeezes
functor -d examples/vr-lab test                  # inline expects, no GPU
npm run test:vr-lab                              # injected XR scenarios, headless
npm run demo:vr-lab -- grab-throw out.gif --mp4 out.mp4 --still out.png
```

| Input | Action |
| --- | --- |
| `1`-`9` | jump to that station and reset it |
| B / Y / menu | next station (reset) |
| grip (squeeze) | grab what's in reach; open to let go |
| trigger | fire the held gun (station 3) |
| grip on the shotgun's fore-end | hold it; draw it back and home to pump |
| A / X (holding the rifle's grip), `F` | the rifle's selector: semi / full-auto |
| A / X (holding the pistol) | its slide release, when locked back |
| thumbstick down (on a gun's trigger hand), `M` | drop the gun's magazine |
| `H` / thumbstick click | draw a holding hand where it is tracked, or (the default) where its blocked prop implies |

`GET /state` shows the station, both hands (`tracked`, `squeezed`, `armed`,
`hover`), `handView`, the props (`held`, and `pose`/`velocity` read back from
the world, rig-local), the last release (`lastRelease`), the score, and each hand's pulse edges
(`feltLeft`/`feltRight`); `haptics` counts the pulses sent. The
headset requests 120 Hz with foveation off, floor-locked.

Hands are drawn as capsule gloves (`Hand`): OpenXR's 26 hand joints posed
by forward kinematics from how far each finger curls, a capsule per bone.
Empty, the grip curls the middle, ring and little fingers round the
controller's handle and the trigger the index. Holding, each prop has a
preset (`Props.fingers`): a power wrap for the stick, the bottle's neck, a
magazine and a foregrip or fore-end; a gun's grip with the index on its
trigger; a pinch for a round or shell; an open palm for the ball and brick.
A hand's ghost is the same glove.

| Module | What it is |
| --- | --- |
| `game.fun` | the MVU glue: station selector, the step (holds, then slots where worn), physics, drawing |
| `stations.fun` | where each station stands and what it's built from; rig ↔ world |
| `hips.fun` | the body frame the slots ride: root and lagging yaw from the head, pin |
| `slots.fun` | the body slots: offsets, reach, what each takes, how each is drawn |
| `inventory.fun` | the inventory station's headless checks |
| `gun.fun` | what the guns share (trigger and fire modes, bore, shot, muzzle check, recoil, the ammo seam, magazine well, readout and look), and the pistol's shape and grip |
| `ammo.fun` | rounds and magazines as plain data: fire, eject, insert, rack (extract, close), the slide lock, and their conservation |
| `seating.fun` | a magazine guided into a well: the latch, the guided depth, the seat and the pull-out |
| `longarm.fun` | the rifle: shape, its two grips, the two-hand aim, the support hand's slide, break and look |
| `pump.fun` | the pump shotgun: shape, grips, its fore-end (the support line that slides with the pump), loading port and look |
| `armory.fun` | the armory station's headless checks |
| `pulses.fun` | each hold event's controller pulse, and the edges that fire it once |
| `hold.fun` | one hand grabbing, holding and releasing a prop (any pose); the swept move; the throw and spin estimate |
| `hand.fun` | the hand as data: the joints, the bones, finger curls and their forward kinematics, the hand on a controller, the capsule glove and its presets |
| `props.fun` | the props: shape, collider, reach, authored grip, the fingers on it, material, look |
| `digits.fun` | seven-segment numbers (the headset doesn't draw `ui`) |
| `quat.fun` | plain-data vectors and quaternions |

## Scenarios and videos

`e2e/vr-lab-scenarios.mjs` scripts each station as XR samples, one per fixed
step (format in `e2e/xr-scenario.mjs`). `e2e/vr-lab.mjs` plays them headlessly
and checks `/state`; `e2e/xr-demo.mjs` plays one in a hidden window, films it
from the scenario's spectator view (`POST /camera`) and writes a GIF/MP4. A
new station adds scenarios there, not a new driver.

### Recordings

A real session records to an XR fixture (format in `e2e/xr-fixture.mjs`):
each fixed step's XR sample and key edges, read from the runtime's input log
(`GET /input-log`). Wearing the headset, with `functor -d examples/vr-lab run
vr` running:

```sh
npm run record:vr-lab -- /tmp/throws.xr.json     # Ctrl-C to stop (or --seconds 20)
```

It restarts the game and resets station 1 first, so the fixture replays from
a fresh launch. A
fixture plays anywhere a scenario does: `npm run demo:vr-lab --
/tmp/throws.xr.json out.gif --mp4 out.mp4` films it on desktop, and a
scenario entry `{ fixture: "<path>" }` runs it under `e2e/vr-lab.mjs`
(`grab-throw-recorded` is the checked-in one, re-recorded with `npm run
record:vr-lab -- e2e/fixtures/vr-lab/grab-throw.xr.json --scenario
grab-throw`). Replay is on desktop. The headset also accepts injected XR
samples (`POST /input`), but fixture playback there is untested.

## Haptics

Each hold event buzzes the hand that felt it (`Effect.haptic`, from `tick`).
The amplitudes and durations are one table, `Pulses.pulse`:

| Event | When | Amplitude | Duration |
| --- | --- | --- | --- |
| ready | entering an empty slot that takes the held prop | 0.15 | 15 ms |
| latch | a held magazine (or shell) catching in a well's mouth (the loading port) | 0.2 | 12 ms |
| click | a trigger pull refused (the muzzle against the level) or dry (nothing chambered) | 0.25 | 10 ms |
| release | letting go (thrown or dropped) | 0.3 | 20 ms |
| load | a shell pushed into the shotgun's tube (its hand and the gun's) | 0.4 | 15 ms |
| grab | taking a prop, free or from the other hand (or a gun's slide) | 0.45 | 30 ms |
| slide back | the slide, charging handle or pump pulled to the back | 0.5 | 25 ms |
| unstow | taking a prop out of a slot | 0.55 | 40 ms |
| slide home | the slide (or pump) going home from the back (the racking hand and the gun's) | 0.65 | 30 ms |
| stow | putting it away | 0.7 | 50 ms |
| seat | a magazine pushed home in a well (its hand and the gun's) | 0.8 | 25 ms |
| blocked | the first step a held prop stops against the level (once per push) | 0.85 | 20 ms |
| fire | a shot (every hand on the gun) | 0.95 | 35 ms |
| break | the grip breaking (or the rifle's support) | 1.0 | 80 ms |

A held prop is blocked once the hand is 2 cm or 10° off where it holds it, and
free again under half that, so a hand resting at the edge doesn't buzz twice.
Each pulse also draws a brief ring at the hand, sized by its amplitude, so a
video (or a desktop run) shows what the hand felt. `npm run test:vr-lab`
checks each event's pulse in `GET /state.haptics`
(`e2e/vr-lab-haptics.mjs`).

## 1. Grab table

A table with four props on it — a ball, a stick (a 70 cm box with a grip
band), a brick (resting turned) and a bottle (a box collider round a
cylinder and a neck) — a wall against the table's left end, and a basket on
the floor a throw away, with a scoreboard behind it. Every prop takes the
same path:

- **Grab.** Either hand, within 12 cm of a prop's surface (16 cm to stay
  hovered: it glows); the nearest prop in reach. The squeeze has hysteresis
  (0.6 press, 0.4 release) and only a fresh press grabs: a hand that was
  squeezing when tracking dropped, when the grip broke or when the station
  reset must open first. Both hands pressing on one prop at once: the
  nearer wins.
- **Regrab.** The grab reads the prop's pose from the world (`Physics.pose`)
  and keeps the hand's offset to it, position and turn: it stays exactly
  where it was, held at the point and angle it was taken. A prop with an
  authored grip (the bottle's neck) snaps to it instead when the hand is
  within 5 cm of it, pointing within 45° of the way it is held there.
- **Hand-off.** A fresh squeeze of the other hand on a held prop takes it
  where it is (no snap), so the pose doesn't jump — even in the step the
  first hand opens.
- **Hold.** Each step the prop sweeps from where it is toward where the hand
  holds it, turning on the way (`Physics.sweep` against fixed bodies only, so
  never itself, at the pose it is drawn at), stops 4 mm short of what it hits
  and slides along it, and is declared kinematic there (`Physics.atPose`), so
  it shoves what it sweeps through. Pushed into the wall or the table it
  stays there; pulled back, it follows the hand out.
- **Break.** The hand 25 cm off where it holds the blocked prop, or turned a
  quarter turn past it: it drops where it was, with no velocity.
- **Throw.** Released, the prop turns dynamic where it was last held (where
  its body is) and is given, in the same frame, the least-squares slope of
  its accepted positions over the last 60 ms (rig-local, so a moving rig
  adds nothing; capped at 12 m/s) and of its orientations (capped at
  30 rad/s). (Not a declared `Physics.velocity`: that follows the divergence
  rule, applying only when the declaration changes.)
- **Tracking loss.** A held prop freezes and forgets its history; opening the
  hand when it comes back drops it rather than throwing it.
- **Reset** bumps the props' tags, so the old bodies go and new ones spawn at
  rest.
- **Hand presentation.** `handView` (`H` or a thumbstick click) draws a
  holding hand at the grip its prop's accepted pose implies (`Accepted`, the
  default: a hand pushed through the wall is drawn at the wall) or where it
  is tracked (`Tracked`). In `Accepted` a gun's support hand is drawn on its
  foregrip (or fore-end), and a racking hand on the slide (or charging
  handle) where the slide has it, so a hand off that line is drawn on it. A
  translucent ghost shows the tracked hand when they are over 2 cm or 10°
  apart. Only drawing changes: the tracked
  pose always drives the hold.

Worn checks still open: held-prop judder at 120 Hz (the hold updates at the
60 Hz fixed step), how throws feel, whether a prop stopping at the wall
(and the hand turning on past it) reads right, whether a flicked stick spins
as the wrist did (the 60 ms window's feel) and shoving with a held stick
feels solid, which hand view feels better when blocked (and the ghost's
2 cm threshold), whether regrabs and hand-offs feel natural, the neck's snap
radius, and the sweeps' cost on the headset (`npm run bench:quest`).

## 2. Inventory

The same table and props, at a second station, and three slots worn on the
body: a holster at each thigh (the brick or the bottle) and a pouch on the
belt in front (the ball). The stick fits none.

- **The hips.** The slots hang off a frame `{root, yaw}` that is inferred
  from the head alone (no tracked pelvis): the neck pivots 11 cm below and
  9 cm behind the eyes, and the pelvis is under it at 0.62 of its height.
  Its heading lags the head's as examples/fullbody's hips do (its
  `slewYaw`, copied): within 25° it drifts after it at 20°/s, past that it
  chases at 150°/s, and it never trails by more than 70°. Pitch doesn't
  tilt it; a head with no heading (rolled onto its side) keeps the last,
  and a lost head holds the whole frame.
- **Pin.** While either hand is within 10 cm of a slot's reach the heading
  holds still, so the slot doesn't swim away as the hand arrives; for
  1.5 s at most, so arms hanging by the holsters don't hold it back for
  good. Let go past the 70° twist limit, it catches up to the limit at once.
- **Slots.** Each is plain data (`slots.fun`): an offset in the hips' frame,
  an 8 cm reach round its centre, the kinds it takes, room for one. Drawn as
  a ring: steel (holsters) or amber (pouch) idle, green when a hand holding
  something it takes is in it, red when it won't (wrong kind, or full).
- **Deposit.** Open a hand holding a prop inside a slot that takes it and
  is empty: the prop goes there, at the slot's pose, and rides the hips. It
  has no body while slotted, so a worn prop shoves nothing (a kinematic one
  would knock props off the table as you walk past), and its pose is just
  model data.
- **Retrieve.** Squeeze inside the slot: the hand takes it back exactly
  where it sits (the regrab, without the neck snap), and it gets a body
  again.
- **One place at a time.** A prop is in a hand, in a slot or free. Each step
  decides storing first, then hand-offs, then ordinary drops and throws: the
  other hand can't take a prop being put away, and a prop let go in a slot
  that won't take it (wrong kind, or full) is let go as anywhere else. Two
  hands opening in one slot in one step: the hand nearer its centre stores,
  the right on a tie; the other's prop is let go. A hand that only just came
  back from tracking loss never stores (its prop drops), and a lost hand
  keeps holding.
- **Reset** returns every prop to the table and forgets the hips.

`GET /state` shows each prop's `slot` and the `hips` (`head`, `yaw`, `root`,
`pinned`). `npm run bench:vr-lab` times the station's logic per step
headlessly (release build).

Worn checks still open: the slots' heights and reach for real bodies (the
0.62 pelvis ratio, the 20 cm holster offset), the deadzone and slew when
turning, whether the pin engages early enough and lets go cleanly, and
whether the slots can be found without looking.

## 3. Armory

A bench with a pistol on it, a rifle and a pump shotgun racked beside it,
four spare magazines and a box of shells, the grab table's wall at its left end, and three targets on a
backstop 5.5 m down range, with a scoreboard. The holsters are worn here too,
and take the pistol; the pouch takes a magazine.

### The pistol

- **Grip.** The pistol is only taken at its grip: within 7 cm of where the
  fist goes (`Gun.gripZone`). By the barrel it isn't grabbable at all;
  snapping a barrel grab to the grip would flip and throw the gun ~15 cm
  into the hand, and hide where the hand really took it. Taken at the grip,
  it doesn't jump: it settles into the hand from where it was, at most 2 cm
  and 8.6° a step (`Hold.settle`), so drawing it from a holster is smooth
  too. Held off by the level meanwhile, it waits; let go while settling, it
  drops. Unlike other props, no hover hysteresis widens the 7 cm. Held, it is
  swept like every prop.
- **Bore.** Settled, the bore runs 5 cm above the fist, parallel to the grip
  pose's -Z, from the chamber 2 cm ahead of it to the muzzle 15 cm ahead.
  The sights sit on the slide 2.4 cm above the bore, parallel, so a shot
  lands that far under the sight line at any range.
- **Trigger.** Analog with hysteresis: a pull to 0.8 fires once; it must fall
  back to 0.3 before the next counts. Semi-automatic. A trigger held as the
  pistol is taken (off the bench, out of a holster, from the other hand) or
  as its hand's tracking comes back fires nothing until it is let out.
- **Shot.** Hitscan from the muzzle along the barrel, 30 m, of the pose the
  hold accepted (where the sweep let it be, never the tracked hand's),
  past the pistol's own body. A target hit records the point and tag, lights
  the target, leaves a marker and scores. A tracer shows for 0.1 s and a
  muzzle flash for 0.05 s.
- **Muzzle check.** Before a shot, a 2 mm ball is swept from the chamber to
  1 cm past the muzzle against fixed bodies. Touching one (the barrel held
  against the wall: the sweep stops the pistol 4 mm short of it) refuses the
  shot: no bullet, a click pulse instead of the fire pulse.
- **Recoil.** Visual only: the drawn pistol kicks up 8° and back 2 cm about
  the grip, easing out over 0.12 s. The body, the muzzle check and the next
  shot use the accepted pose, so it never moves the aim.
- **Ammo.** Finite: see [Ammunition](#ammunition).

`GET /state` shows `gun`: its `holder`, `trigger` (`pulled`, `armed`),
`kick`, `ammo`, `fired`, `refused`, `dry`, the `last` shot (`from`, `dir`,
`impact`) and the target `hits`. `npm run test:vr-lab` runs
`e2e/vr-lab-armory.mjs`.

Worn checks still open: whether the pistol sits naturally in the hand (the
barrel along the grip pose's -Z), whether shots land where the sights look,
the trigger thresholds on a real trigger, the recoil and fire pulse's feel,
and drawing from the thigh holster.

### The rifle

The same trigger, shot, muzzle check, recoil and ammo seam (`Gun`), with its
own grips and bore (`Longarm.weapon`).

- **Two grips.** The pistol grip (the trigger) is taken like the pistol's:
  within 7 cm, settling in. The foregrip is a line under the handguard,
  through the primary fist parallel to the bore, 14-40 cm ahead of it: the
  other hand's squeeze within 6 cm of it, while the first holds the grip,
  takes it as the support, if the break limits below would hold it there.
  A free or racked rifle is only taken by the grip.
- **Two-hand aim.** The barrel points from the primary fist to the support
  fist; its up is the primary hand's up made perpendicular to that, so the
  roll is the primary hand's (`Longarm.aimAt`). The primary fist stays on the
  grip. Taking the foregrip off that line doesn't jump the barrel: the
  difference eases out at 8.6° a step. Then the whole rifle is swept once a
  step to that pose, like any held prop (`Physics.sweep`, rotation-aware), and
  both grips and the muzzle come from the pose it accepts. Tracking only
  steers; the accepted pose never feeds back.
- **Slide and break.** Along the foregrip the support slides freely. Past
  12 cm off the primary hand's axis, or 8 cm past either end of the foregrip,
  it breaks off: a break pulse, and the hand must open before it takes the
  foregrip again. Opened, it lets go with a release pulse. Either way the
  rifle stays where the aim had it on that step, then eases back into the
  primary hand (2 cm, 8.6° a step at most).
- **One hand.** Rigid in the primary hand, like the pistol: no droop or lag.
- **Hand-off.** The primary hand opening while the support holds hands the
  rifle to it where it is: held by the foregrip, one-handed, with no trigger
  (the trigger never moves to the foregrip hand). Squeezing the pistol grip
  again takes it back, the foregrip hand staying on as the support (if the
  new grip's axis holds it; else it just lets go); the trigger starts
  disarmed until let out. A support hand losing tracking lets go; a lost
  primary hand freezes the rifle, its support able only to let go.
- **Fire modes.** Semi, or full-auto at 10 rounds a second counted in fixed
  steps (one on the pull, one every 6 steps held: N steps held after the
  pull fire floor(N / 6) + 1). A / X on the hand at the grip, or `F`,
  switches; the light on the receiver's left is green on semi, red on
  full-auto. A refused shot disarms the trigger, so a blocked barrel clicks
  once a pull, not at the rate. Each shot buzzes both hands on the rifle;
  a click, only the trigger hand.
- **Rack.** On the bench's right: let go within 20 cm of where it racks (by
  the hand holding it alone), it goes back, bodiless, like a slot. The rifle
  doesn't fit the thigh holsters.
- **Hand view.** `Accepted` (the default) draws the primary hand on the
  accepted grip, the support on the accepted foregrip and a racking hand on
  the charging handle, each with a ghost at its tracked pose past 2 cm or 10°. Nothing a held gun touches moves the camera.

`GET /state` shows `rifle` (as `gun`, plus `mode`) and the rifle prop's
`support` (`side`, `ease`, `aim`) and `fore`. `npm run test:vr-lab` runs
`e2e/vr-lab-rifle.mjs`.

Worn checks still open: two-hand grip comfort and stock-less aim stability,
where the slide and the 12 cm break land, whether the hand-off and retake feel
natural, the 10 Hz auto rate, the hand-view A/B with both hands blocked, and
the settle back to one hand after a support lets go.

### The shotgun

The rifle's hold (`Longarm`), with a pump (`pump.fun`, `Pump.weapon`).

- **Grips.** The pistol grip (the trigger), within 7 cm, settling in. The
  support hand takes the fore-end: the same line through the primary fist,
  28-42 cm ahead of it with the pump home, wherever the pump has it. The
  two-hand aim, the break (12 cm off the axis, or 8 cm past where the
  fore-end can be), the hand-off (the fore-end hand alone holds it, no
  trigger) and the retake are the rifle's.
- **Pump.** The support hand on the fore-end works it: the fore-end follows
  the hand back along the bore, clamped to its 9 cm travel (the aim doesn't
  move: the hand slides along the line it aims by). Past 80% back: a pulse,
  and the chambered shell is thrown out of the ejection port (live, a loose
  shell) or its fired hull (spent). Back under 20%: home, a pulse in both
  hands, the next shell fed from the tube. A partial stroke does nothing.
  Let go (or broken off, or its hand's tracking lost) the pump stays where
  it is; taken again it goes on from there. While the primary hand is lost it
  holds still.
- **One shot a stroke.** A shot leaves its hull in the chamber, so the next
  pull clicks dry until the pump goes back. The pump out of battery (back,
  or more than 20% open) clicks dry too. It never locks back.
- **Shot.** The muzzle check as the others, then eight pellets cast from the
  muzzle: one along the bore, seven evenly round it on a 1.2° cone (9.6 cm at
  the targets), the ring turned by the golden angle (137.5°) each shot, so
  the pattern varies from shot to shot and replays exactly. Each pellet that
  strikes a target leaves a mark there; a shot with any on a target scores
  one. The last two shots' marks stay. The recoil (visual) is 1.8× the
  pistol's.
- **Tube.** It starts empty and holds 6, plus one chambered (`6+1` on the
  readout on the receiver). A shell is loaded like a magazine is seated
  (`seating.fun`), at the loading port under the receiver, 12 cm ahead of
  the fist: lying along the bore, head back (within ~32°; any turn about its
  own axis), within 3 cm of the port's mouth, it latches (a pulse), and is
  guided 6 cm up into it at the hand's depth; home, it is in the tube (a
  load pulse in both hands). Reversed, tipped, or the tube full: it never
  latches. Pulled 5 cm off the port's axis, or tipped 46°, it comes out.
- **Shells.** A squeeze in the box on the bench (at the back, left of the pistol;
  lit while an empty hand is in it) takes a shell out, held as the hand
  points (down range, as the port takes it), until the box is empty (12). A
  shell pumped out live lies about as a loose round does, and loads again.
  Hulls lie about too and share the rounds' cap of six; past it, the oldest
  hulls go first.
- **Rack.** Between the pistol and the rifle; let go within 20 cm of where
  it racks (by the hand holding it alone), it goes back, bodiless.

`GET /state` shows `shotgun` (as `gun`, its `slide` the pump) and
`shellBox`. `npm run test:vr-lab` runs `e2e/vr-lab-shotgun.mjs`.

Worn checks still open: the fore-end's 28-42 cm reach, the pump's 9 cm stroke
and its 80% / 20% thresholds, finding the loading port by feel, the 3 cm
catch, and whether the 1.2° spread reads.

### Ammunition

Rounds are plain data (`ammo.fun`): a magazine is `{kind, rounds, capacity}`,
a gun's ammo `{well, mag, chambered, locked}`. Nothing makes or loses a
round: at every step, the rounds in every magazine (free, held, pouched,
latched, in a well), chamber and loose round, plus the shots fired and the
loose rounds taken away (`lost`), are what the station was reset with.

- **Loaded.** The pistol starts 12+1, the rifle 30+1 (`Ammo.capacity`); the
  shotgun unloaded, its 12 shells in the box. The conservation count takes
  in the box, the tube and loose shells too (hulls are litter, not rounds).
- **Fire.** A shot needs a chambered round (`Gun.canFire`); it spends it and
  feeds the next from the magazine (`Ammo.fire`, via `Gun.onFired`). Nothing
  chambered, the pull clicks dry: a click pulse, no shot, the trigger
  disarmed (so full-auto stops exactly at empty, one click a pull), and the
  readout flashes red for 0.3 s.
- **Slide lock.** A slide (or bolt) that closes on an empty magazine, after
  the last shot or a rack, locks back (`Ammo.close`): the pistol's slide
  stays 70% back, its barrel showing; the rifle's bolt shows back in the
  port (its charging handle goes home). With no magazine in, it closes on an
  empty chamber. Locked, a pull clicks dry; a magazine put in doesn't
  chamber; a rack, or A / X on the pistol's hand (its slide release; the
  rifle has none: on its hand A / X is the selector), releases it and
  chambers.
- **Readout.** Each gun shows its magazine's rounds + the chambered round on
  its back (`12+1`; `--+1` with the magazine out): the pistol on the slide's
  back face, the rifle on the receiver ahead of the rear sight, tipped toward
  the eye.
- **Magazine release.** The thumbstick pushed down on a gun's trigger hand
  (a fresh push past -0.7, released past -0.3), or `M`: the magazine drops out
  below the well (`Gun.dropOf`, clear of the gun's collider; if the level is
  in the way, e.g. the gun held low over the bench, it stays in) as a free prop
  with its rounds, its count on its back. It is held, thrown, and put in the
  pouch like any prop; the chambered round stays in the gun and fires.
- **Spares.** Two pistol and two rifle magazines, full, stand in cradles on
  the bench's left (`Slots.cradle`: like the rifle rack, a slot whose prop has
  no body, so resting spares cost no physics). A cradle takes back any
  magazine of its kind.
- **Seating** (`seating.fun`, each well's `Gun.guide`). A held magazine of
  the well's kind, the well empty (and no other magazine latched to it), latches when its centre is within 4 cm of
  where it sits at the mouth (where an ejected one drops out) and its axis is
  within ~32° of the well's (dot ≥ 0.85), not turned back to front (keyed):
  a latch pulse. Upside down, the wrong kind, or tipped: nothing. Latched,
  it has no body (it is part of the gun: the gun's hand carries it, and
  nothing sweeps it against the gun) and sits on the well's axis, turned as
  the well is, easing there from where it was (2 cm, 8.6° a step at most),
  at the depth the hand pushes it to. Pushed home (within 4 mm) it seats:
  `Ammo.insert`, a seat pulse in both hands, and it is the gun's. Pulled
  7 cm off the axis (or that far below the mouth), or tipped past ~46°
  (dot 0.7), it leaves where it is, back in the hand, easing to where the
  hand held it, with no body until it has (it may still be inside the gun),
  and it can't latch again until then. Let go while latched,
  it falls out of the mouth. The gun can be held, or anywhere a mouth is
  reachable: lying on its side on the bench, or the rifle in its rack. The
  pistol standing upright on the bench has its mouth against the bench top,
  so nothing reaches it there.
- **Rack.** The other hand squeezed at a held gun's slide (the pistol's rear
  serrations, 3.5 cm zone) or the rifle's charging handle (the receiver's
  top rear, 4 cm) takes it (a grab pulse) and draws it back along the bore,
  clamped to its travel (4 cm, 7 cm); 8 cm off its line, the hand lets go.
  Past 80% of the travel: back (a pulse), a chambered round thrown out
  (`Ammo.extract`). Then let go, or returned under 20%, it goes home (a
  pulse in both hands) and feeds (`Ammo.close`). Short of 80%, nothing
  happens. A locked pistol slide is taken where it rests, so a nudge past
  80% and letting go releases it; the rifle's handle is pulled all the way.
  A slide held back is out of battery: its trigger clicks. The racking
  hand losing tracking keeps the slide where it is.
- **Loose rounds.** A round thrown out is a small dynamic prop (a 9 mm × 29
  or 57 mm cartridge), out of the port to the right, up and back (about
  1.9 m/s, with the gun's own velocity), spinning. It can be picked up and
  thrown; nothing takes it yet (but a shotgun shell, which loads). At most 6
  lie about, hulls included: past that the oldest not in a hand is taken
  away, hulls first, a round counted `lost`. One come to rest on the
  level (the bench, the floor) lies there bodiless, costing no physics,
  until a hand takes it; a round resting on a prop keeps its body.

`GET /state` shows each gun's `ammo` and `slide`, each magazine prop's `mag`
and `latch`, and `lost`. `npm run test:vr-lab` runs `e2e/vr-lab-ammo.mjs` and
`e2e/vr-lab-reload.mjs` (the shotgun's tube and pump: `e2e/vr-lab-shotgun.mjs`).

Worn checks still open: reaching the thumbstick down on the trigger hand
without shifting the grip, the readouts' legibility at arm's length, whether
the dry click reads as a dry pull, and finding the pouch with a magazine.
Reloading: whether the 4 cm / 0.85 latch is too sticky or too strict, the
seat depth's feel and its click, the slide's and charging handle's stroke
and 80% threshold, whether a locked slide reads at a glance, and reloading
without looking.
