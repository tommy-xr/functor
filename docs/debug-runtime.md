# Debug runtime

An optional HTTP control server on the desktop runtime (`functor-runner`) that lets
an external client — a script, a test, or an LLM — **observe** and **drive** a running
game without a GPU window of its own. It is the runtime arm of Functor's LLM-native
goal: capture frames, query state, control the frame clock, and inject input over a
localhost socket.

Start it by passing `--debug-port <PORT>`:

```sh
# via the CLI (rebuilds + runs the game)
./target/debug/functor -d examples/hello run native --debug-port 8077

# or the runner directly, against an already-built dylib
#   (cwd must be the game dir so assets resolve)
cd examples/hello
functor-runner --game-path build-native/target/debug/libgame_native.dylib --debug-port 8077
```

The server binds **localhost only** (`127.0.0.1:<PORT>`). HTTP handlers never touch GL;
each request is handed to the render loop and fulfilled once per frame.

## Headless mode

Add `--headless` to run with **no GL window** — the game loop and debug server run
without GLFW/OpenGL, so no display (or GPU) is needed. Ideal for CI, scripted runs,
and LLM-driven control:

```sh
functor-runner --game-path <dylib> --debug-port 8077 --headless
```

`/`, `/state`, `/scene`, `/input`, and `/time` all work (the game's `draw3d`
produces a pure `Frame`, so `/scene` is real data with no rendering). This is the
runtime expression of the LLM-native principle: drive and observe a game with no
GPU window. Limitations vs. windowed:

- `/capture` is unavailable (no pixels to read back) and returns `503`.
- Audio isn't played, and `Audio.playThen` completion messages are **not**
  delivered — don't gate game logic on audio completion when running headless.
- `--capture-frame` is rejected (it needs GL).

## Endpoints

`GET /` returns this list as JSON (discoverability).

| Method & path | Purpose |
| --- | --- |
| `POST /capture` | PNG (`image/png`) of the next rendered frame |
| `GET /state` | runtime state JSON: `frame`, `tts`, `viewport`, `input` (structured `held_keys` + `mouse`), `model` (Rust `Debug` text) |
| `GET /scene` | current frame as JSON: `camera` + `scene` + `lights` |
| `POST /input` | inject input (see below) |
| `POST /time` | control the frame clock (see below) |

### `POST /input`

JSON is tagged by `type`. Unknown keys/shapes return **400** with a message.

```jsonc
{"type":"key","key":"w","down":true}      // key press / release
{"type":"mouse_move","x":10,"y":20}       // absolute cursor position
{"type":"mouse_wheel","delta":1}          // scroll
```

### `POST /time` — frame-loop control

```jsonc
{"type":"set","tts":2.0}        // PAUSE: pin game time to a constant (dts=0)
{"type":"advance","dts":0.016}  // STEP: run exactly one frame with this dt, then hold
{"type":"resume"}               // RESUME: follow wall-clock again
```

`--fixed-time <T>` pins the clock from launch (equivalent to an initial `set`).
While the clock is pinned, **user keyboard/mouse input from the window is ignored**, but
injected `/input` still applies — so an external driver has deterministic control.

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
