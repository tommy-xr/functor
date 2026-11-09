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
| `game.fun` | the MVU glue: station selector, the grab table's step, physics, drawing |
| `stations.fun` | where each station stands and what it's built from; rig ↔ world |
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
