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
| `H` / thumbstick click | draw a holding hand where it is tracked, or where its blocked prop implies |

`GET /state` shows the station, both hands (`tracked`, `squeezed`, `armed`,
`hover`), `handView`, the props (`held`, and `pose`/`velocity` read back from
the world, rig-local), the last release (`lastRelease`) and the score. The
headset requests 120 Hz with foveation off, floor-locked.

| Module | What it is |
| --- | --- |
| `game.fun` | the MVU glue: station selector, the step (holds, then slots where worn), physics, drawing |
| `stations.fun` | where each station stands and what it's built from; rig ↔ world |
| `hips.fun` | the body frame the slots ride: root and lagging yaw from the head, pin |
| `slots.fun` | the body slots: offsets, reach, what each takes, how each is drawn |
| `inventory.fun` | the inventory station's headless checks |
| `hold.fun` | one hand grabbing, holding and releasing a prop (any pose); the swept move; the throw and spin estimate |
| `props.fun` | the props: shape, collider, reach, authored grip, material, look |
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
  holding hand where it is tracked (`Tracked`, the default) or at the grip
  its prop's accepted pose implies (`Accepted`: a hand pushed through the
  wall is drawn at the wall). Then a translucent ghost shows the tracked hand
  when they are over 2 cm or 10° apart. Only drawing changes: the tracked
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
  Its heading lags the head's as examples/fullbody's hips do (copied from
  its `body.fun`): within 25° it drifts after it at 20°/s, past that it
  chases at 150°/s, and it never trails by more than 70°. Pitch doesn't
  tilt it; a head with no heading (rolled onto its side) keeps the last.
- **Pin.** While either hand is within 10 cm of a slot's reach the heading
  holds still (it still never trails past 70°), so the slot doesn't swim
  away as the hand arrives.
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
