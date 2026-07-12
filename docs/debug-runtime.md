# Debug runtime

An optional HTTP control server on the desktop runtime (run in-process by the `functor`
CLI) that lets
an external client — a script, a test, or an LLM — **observe** and **drive** a running
game without a GPU window of its own. It is the runtime arm of Functor's LLM-native
goal: capture frames, query state, control the frame clock, and inject input over a
localhost socket.

Start it by passing `--debug-port <PORT>`:

```sh
# the CLI runs the game in-process and interprets the .fun
./target/debug/functor -d examples/hello run native --debug-port 8077
```

The server binds **localhost by default** (`127.0.0.1:<PORT>`); `--debug-bind 0.0.0.0`
exposes it to the LAN for remote develop (see `POST /reload-source`) — there is no auth,
so bind wide only on networks where arbitrary game-code pushes are acceptable. HTTP
handlers never touch GL; each request is handed to the render loop and fulfilled once
per frame.

## Headless mode

Add `--headless` to run with **no GL window** — the game loop and debug server run
without GLFW/OpenGL, so no display (or GPU) is needed. Ideal for CI, scripted runs,
and LLM-driven control:

```sh
./target/debug/functor -d examples/hello run native --debug-port 8077 --headless
```

`/`, `/state`, `/scene`, `/input`, and `/time` all work (the game's `draw`
produces a pure `Frame`, so `/scene` is real data with no rendering). This is the
runtime expression of the LLM-native principle: drive and observe a game with no
GPU window. Limitations vs. windowed:

- `/capture` is unavailable (no pixels to read back) and returns `503`.
- Audio isn't played, and `Audio.playThen` completion messages are **not**
  delivered — don't gate game logic on audio completion when running headless.
- `--capture-frame` is rejected (it needs GL).

## Hidden window mode

If you need **pixels** but not a window, add `--hidden` instead: the GL window is
created but never shown, never takes focus, and never captures the cursor, so a
run doesn't steal input from whoever is at the machine. A hidden window keeps a
valid GL context and framebuffer, so rendering, `/capture`, and `--capture-frame`
work unchanged (audio too). `--capture-frame` implies `--hidden` — a scripted
screenshot run has no reason to grab your mouse.

```sh
./target/debug/functor -d examples/hello run native --debug-port 8077 --hidden
```

## Endpoints

`GET /` returns this list as JSON (discoverability).

| Method & path | Purpose |
| --- | --- |
| `POST /capture` | PNG (`image/png`) of the next rendered frame |
| `GET /state` | runtime state JSON: `frame`, `tts`, `viewport`, `input` (structured `held_keys` + `mouse`), `model` (Rust `Debug` text) |
| `GET /scene` | current frame as JSON: `camera` + `scene` + `lights` |
| `GET /trace` | paused-inspector trace: the last real frame's entry-point invocations (per-binding-site values + result) replayed while paused; `{ "paused": false, "invocations": [] }` while playing |
| `POST /input` | inject input (see below) |
| `POST /time` | control the frame clock (see below) |
| `POST /reload-source` | swap game logic from the request body (see below) |

### `POST /input`

JSON is tagged by `type`. Unknown keys/shapes return **400** with a message.

```jsonc
{"type":"key","key":"w","down":true}      // key press / release
{"type":"mouse_move","x":10,"y":20}       // absolute cursor position
{"type":"mouse_wheel","delta":1}          // scroll
{"type":"ui_event","slot":0,"kind":"Clicked"}                  // click widget slot 0
{"type":"ui_event","slot":1,"kind":{"SliderChanged":0.5}}      // drag slider slot 1
{"type":"ui_event","slot":2,"kind":{"TextChanged":"hi"}}       // edit text input slot 2
```

`ui_event` drives the game's interactive UI widgets without pixels or
hit-testing (docs/ui-interaction.md): `slot` is the widget's index in the
frame's `ui(model)` tree, in construction order over the interactive widgets.
An event for a slot the current view doesn't have is dropped (with a one-line
runtime report), and the endpoint still returns 200 — delivery, not handling,
is what's acknowledged.

### `POST /time` — frame-loop control

```jsonc
{"type":"set","tts":2.0}        // PAUSE: pin game time to a constant (dts=0)
{"type":"advance","dts":0.016}  // STEP: run exactly one frame with this dt, then hold
{"type":"resume"}               // RESUME: follow wall-clock again
```

`--fixed-time <T>` pins the clock from launch (equivalent to an initial `set`).
While the clock is pinned, **user keyboard/mouse input from the window is ignored**, but
injected `/input` still applies — so an external driver has deterministic control.

### `POST /reload-source` — network hot-reload (Functor Lang)

The body is the raw `.fun` source. The runner validates it and swaps the session with
**the model preserved** — the same semantics as the file-watch reload. A broken push
returns **400** with the rendered load error and keeps the old program running; producers
whose logic isn't source-shaped (e.g. the `--replay` producer) also return 400. This is the
remote develop path: run the game on another machine or device
(`--debug-port <P> --debug-bind 0.0.0.0`), then push from the project dir:

```sh
functor -d mygame push <host>:<port>          # push once
functor -d mygame push <host>:<port> --watch  # re-push on every save
```

(`curl --data-binary @game.fun http://<host>:<port>/reload-source` works too.)

## Two workflows

**Observe a human playing.** Leave the clock on wall-clock and poll:

```sh
curl -s localhost:8077/state | jq        # frame, time, model
curl -s localhost:8077/scene | jq .camera
curl -s -X POST localhost:8077/capture -o frame.png
```

**Drive the game (LLM / test plays it).** Pin the clock, act, step, observe — a
deterministic loop:

```sh
H=localhost:8077
curl -s -X POST $H/time  -d '{"type":"set","tts":0}'             # pause
curl -s -X POST $H/input -d '{"type":"key","key":"up","down":true}'
curl -s -X POST $H/time  -d '{"type":"advance","dts":0.016}'      # step one frame
curl -s $H/state | jq .model                                     # see the effect
curl -s -X POST $H/capture -o step.png
```

## Tooling

A typed TypeScript SDK over these endpoints (single-client + a multi-client lockstep
session for simulating multiplayer games) lives in `tools/functor-sdk` *(planned)*.

## Future directions

- **Multiplayer simulation.** Launch N runner instances, each on its own
  `--debug-port`, networked via `Sub.connect`/`Sub.listen`; pin all clocks and step
  them in lockstep, injecting input and observing state per client. This is the
  out-of-process counterpart to the in-process `functor-netsim` harness.
- **Richer observation.** `/state.input` now reports held keys + mouse as structured
  JSON (game-agnostic, runtime-owned). Still open: a parseable snapshot of the game
  *model* itself (today `Debug` text — the model isn't `Serialize`).
- **Discoverability/ergonomics.** First-class `pause` / single-`step` verbs.
