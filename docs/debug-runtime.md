# Debug runtime

An HTTP control server shared by the desktop and Quest runtimes that lets
an external client — a script, a test, or an LLM — **observe** and **drive** a running
game without a GPU window of its own. It is the runtime arm of Functor's LLM-native
goal: capture frames, query state, control the frame clock, and inject input over a
localhost socket.

On desktop, start it by passing `--debug-port <PORT>`:

```sh
# the CLI runs the game in-process and interprets the .fun
./target/debug/functor -d examples/hello run native --debug-port 8077
```

The server binds **localhost by default** (`127.0.0.1:<PORT>`); `--debug-bind 0.0.0.0`
exposes it to the LAN for remote develop (see `POST /reload-source`) — there is no auth,
so bind wide only on networks where arbitrary game-code pushes are acceptable. HTTP
handlers never touch GL; each request is handed to the render loop and fulfilled once
per frame.

On Quest the same protocol is always available on device loopback port 8123.
Forward it over USB, then use the same `curl` or SDK calls:

```sh
adb forward tcp:8123 tcp:8123
curl -s localhost:8123/ | jq
```

The wire surface is intentionally isomorphic. Target differences appear as data:
desktop `/state.views` contains one `main` view, while Quest contains `left` and
`right`; Quest `/capture` is a side-by-side PNG of those two raw eye buffers.
The server answers browser CORS/private-network preflights for a **locally served**
web IDE. Browser origins are accepted only when their host is exactly `localhost`,
`127.0.0.1`, or `[::1]`; hosted sites are rejected so they cannot drive the
unauthenticated developer control port. Close the adb forward and desktop runner
when they are not in use.

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
| `GET /state` | runtime state JSON: `frame`, `tts`, combined/legacy `viewport`, `views` (`main` on desktop; `left` + `right` on Quest), `input` (structured `held_keys` + `mouse` + optional typed device domains), `model` (Rust `Debug` text) |
| `GET /scene` | current frame as JSON: `camera` + `scene` + `lights` |
| `GET /trace` | paused-inspector trace: the last real frame's entry-point invocations plus a synthesized `draw` pass, replayed while paused. Each site (binders AND variable reads, `site`) carries the full `value`, a depth-limited `preview`, and `kind` (primitive/composite — the editor's inline-vs-hover policy); `{ "paused": false, "invocations": [] }` while playing. Paused docs also carry `coverage` (per-file span starts with the frame OFFSETS they executed on, over a ±120-frame journal ring — positive offsets appear when scrubbed behind the live head) and `runnable` (the static could-run set) — the recency gutter's data |
| `POST /input` | inject input (see below) |
| `POST /time` | control the frame clock (see below) |
| `POST /reload-source` | swap game logic from the request body (see below) |
| `POST /reload-project` | swap all sibling modules from a JSON array of `[path, source]` pairs, entry first |
| `POST /load-project` | start a new sibling-module project from the same body, initializing its model from `init` |
| `POST /reload-asset` | upload one project-relative texture/model/audio asset as a binary path+bytes envelope |
| `POST /sync-assets` | finish a sync from a JSON array of current asset paths; uploaded paths absent from the manifest are removed |
| `POST /rewind` | restore recorded model + physics to `{"frame":42}` (pin the clock first) |

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

### Sampled input in `GET /state`

`input` is runtime-owned data sampled for one simulation frame. Keyboard and
mouse keep their existing event entry points; continuously sampled devices add
typed sibling domains to the same record. Quest currently adds `xr` while head
tracking is valid:

```jsonc
{
  "held_keys": [],
  "mouse": { "x": 0, "y": 0 },
  "xr": {
    "head": {
      "position": [0.0, 0.0, 0.0],
      "orientation": [0.0, 0.0, 0.0, 1.0]
    },
    "left": {
      "active": true,
      "grip": { "position": [-0.2, -0.3, -0.4], "orientation": [0, 0, 0, 1] },
      "aim": { "position": [-0.2, -0.3, -0.4], "orientation": [0, 0, 0, 1] },
      "trigger": 0.0,
      "squeeze": 0.0,
      "thumbstick": [0.0, 0.0],
      "primary_pressed": false,
      "secondary_pressed": false,
      "thumbstick_pressed": false,
      "menu_pressed": false
    },
    "right": {
      "active": false,
      "grip": null,
      "aim": null,
      "trigger": 0.0,
      "squeeze": 0.0,
      "thumbstick": [0.0, 0.0],
      "primary_pressed": false,
      "secondary_pressed": false,
      "thumbstick_pressed": false,
      "menu_pressed": false
    }
  }
}
```

Tracked poses use OpenXR's rig-local convention: +X right, +Y up, -Z forward;
quaternions are `[x, y, z, w]`. Head and controller poses are relative to the
same center-eye reference that anchors the authored `Frame.camera`, so a game
can map them through the camera from the same model update without mixing
tracking-space coordinates into portable game state. `active` means an input
source is available for that hand. Grip and aim are independently nullable
because buttons can remain available during a temporary pose-tracking loss.
Analog values are normalized to `0..1`; thumbstick axes to `-1..1`.

Non-XR runtimes omit `xr`, preserving the previous desktop JSON shape. Future
gamepad and mobile-touch support should add typed sibling fields rather than
target-specific endpoints or string-keyed capability bags.

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

`functor run vr` uses `/load-project` for the initial push, so the headset
starts with the project's `init` model. Its watch loop then uses
`/reload-project`, preserving that live model across edits. Both routes carry
all sibling `.fun`/`.funi` modules, with the same file-as-module behavior as
desktop.

### Project asset sync

Source and assets remain separate operations. `POST /reload-asset` carries one
file: a big-endian four-byte UTF-8 path length, that many path bytes, then the
raw asset bytes. Paths are forward-slash, project-relative locators with no
`.`/`..` segments. One file may be up to 256 MB. After uploading added or
changed files, `POST /sync-assets` receives the complete current path list and
evicts uploads deleted on the host.

`functor run vr` performs this automatically for self-contained `.glb` models,
textures, and sounds. (`.gltf` models with external URI dependencies remain a
renderer limitation and are not live-synchronized.) It scans recursively,
excluding hidden paths and the generated root `dist/` tree; the watch loop
fingerprints metadata and reads large bytes only when an asset changes.
Replacing an upload evicts cached model/texture/skybox decodes so the next
frame uses the new bytes. The TypeScript SDK exposes the same flow as
`client.reloadAsset(path, bytes)` and `client.reloadAssets(files)`. It also
distinguishes `client.loadProject(files)` (new `init` model) from
`client.reloadProject(files)` (model preserved).

Sound bytes participate in transport/cache synchronization, but the Quest
shell does not yet have an Android audio-output host. They therefore do not
drive `Sub.assets` or play yet; audio output remains a separate device-runtime
milestone.

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
session for simulating multiplayer games) lives in `tools/functor-sdk`. A client can
point at either `http://127.0.0.1:8077` (desktop) or the adb-forwarded
`http://127.0.0.1:8123` (Quest) without changing API calls.

## Future directions

- **Multiplayer simulation.** Launch N runner instances, each on its own
  `--debug-port`, networked via `Sub.connect`/`Sub.listen`; pin all clocks and step
  them in lockstep, injecting input and observing state per client. This is the
  out-of-process counterpart to the in-process `functor-netsim` harness.
- **Richer observation.** `/state.input` reports held keys, mouse, and optional
  typed sampled-device domains as game-agnostic runtime data. Still open: a
  parseable snapshot of the game *model* itself (today `Debug` text — the model
  isn't `Serialize`).
