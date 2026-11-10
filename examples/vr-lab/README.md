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

`GET /state` shows the station, both hands (`tracked`, `squeezed`, `armed`,
`hover`), the ball (`held`, `ballPos`, `ballVel`), the last release and the
score. The headset requests 120 Hz with foveation off, floor-locked.

| Module | What it is |
| --- | --- |
| `game.fun` | the MVU glue: station selector, the grab table's step, physics, drawing |
| `stations.fun` | where each station stands and what it's built from; rig ↔ world |
| `hold.fun` | one hand grabbing, holding and releasing an item; the throw estimate |
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

A ball on a table, a wall against the table's left end, and a basket on the
floor a throw away, with a scoreboard behind it.

- **Grab.** Either hand, within 12 cm of the ball's surface (16 cm to stay
  hovered: the ball glows). The squeeze has hysteresis (0.6 press, 0.4
  release) and only a fresh press grabs: a hand that was squeezing when
  tracking dropped, when the grip broke or when the station reset must open
  first. Both hands pressing at once: the nearer wins.
- **Hold.** The ball keeps its collider. Each step it sweeps from where it is
  toward the hand (`Physics.shapeCastExcluding`), stops just short of what it
  hits and slides along it, and is declared kinematic there. Pushed into the
  wall it stays at the wall; pulled back, it follows the hand out.
- **Break.** More than 25 cm between the hand and the ball: it drops where it
  stopped, with no velocity.
- **Throw.** The velocity is the least-squares slope of the ball's accepted
  positions over the last 60 ms (rig-local, so a moving rig adds nothing),
  capped at 12 m/s. Released, the ball is declared dynamic and given that
  velocity in the same frame. (Not a declared `Physics.velocity`: that
  follows the divergence rule, applying only when the declaration changes.)
- **Tracking loss.** A held ball freezes and forgets its history; opening the
  hand when it comes back drops the ball rather than throwing it.
- **Reset** bumps the ball's tag, so the old body goes and a new one spawns
  at rest.

Worn checks still open: held-ball judder at 120 Hz (the hold updates at the
60 Hz fixed step), and how throws feel.
