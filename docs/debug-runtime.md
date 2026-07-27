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

The browser IDE and sandbox expose this as their shared **device** panel: enter
the runtime URL, choose **push + go live**, and subsequent source edits preserve
the remote model. The sandbox uploads the selected example's declared local
assets before source and finalizes asset deletion only after source is accepted
(the IDE does not yet author binary assets). The panel polls `/state` and
displays `/capture`. When
linking Quest on its forwarded `8123`, serve the site on a different loopback
port (for example `npm run site:serve -- --port 8124`). Chromium may show its
one-time local-network-access prompt on the first connection.

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

## Choose a deterministic capture workflow

These two modes answer different questions and cannot be combined:

| Goal | Clock | Input | Capture |
| --- | --- | --- | --- |
| Capture one authored pose or animation time | `--fixed-time T` pins every frame to `tts = T` with `dts = 0` | none; `--input-script` conflicts with this mode | `--capture-frame out.png` (optionally pin pixels with `--capture-size WIDTHxHEIGHT`) |
| Capture gameplay after a reproducible sequence | `--input-script actions.input --script-dt DT` advances exactly `DT` per rendered frame | events in `actions.input`, applied before that frame's tick | add `--capture-at-frame N --capture-frame out.png` |

Use a hidden debug server instead when an external driver needs to inspect the
model between actions, inject pointer motion, or decide its next input from the
previous result. Start without `--fixed-time`, pause with `POST /time`, then
alternate injected input, advances, state reads, and captures. See
[Two workflows](#two-workflows) for the complete loop.

## Endpoints

`GET /` returns this list as JSON (discoverability).

| Method & path | Purpose |
| --- | --- |
| `POST /capture` | PNG (`image/png`) of the next rendered frame |
| `GET /state` | runtime state JSON: `frame`, `tts`, combined/legacy `viewport`, `views` (`main` on desktop; `left` + `right` on Quest), `input` (structured `held_keys` + `mouse` + optional typed device domains), `model` (structured JSON — see below), `model_debug` (Rust `Debug` text) |
| `GET /scene` | current frame as JSON: `camera` + `scene` + `lights` |
| `GET /trace` | paused-inspector trace: the last real frame's entry-point invocations plus a synthesized `draw` pass, replayed while paused. Each site (binders AND variable reads, `site`) carries the full `value`, a depth-limited `preview`, and `kind` (primitive/composite — the editor's inline-vs-hover policy); `{ "paused": false, "invocations": [] }` while playing. Paused docs also carry `coverage` (per-file span starts with the frame OFFSETS they executed on, over a ±120-frame journal ring — positive offsets appear when scrubbed behind the live head) and `runnable` (the static could-run set) — the recency gutter's data |
| `POST /input` | inject input (see below) |
| `POST /time` | control the frame clock (see below) |
| `POST /reload-source` | swap game logic from the request body (see below) |
| `POST /reload-project` | swap all sibling modules from a JSON array of `[path, source]` pairs, entry first |
| `POST /load-project` | start a new sibling-module project from the same body, initializing its model from `init` |
| `GET /project` | the running program's own `.fun` sources as a JSON array of `[path, source]` pairs, entry first — the read half of the push routes (see below) |
| `POST /reload-asset` | upload one project-relative texture/model/audio asset as a binary path+bytes envelope |
| `POST /sync-assets` | finish a sync from a JSON array of current asset paths; uploaded paths absent from the manifest are removed |
| `POST /rewind` | restore recorded model + physics to `{"frame":42}` (pin the clock first) |

### `model` in `GET /state`

`model` is the structured view — a total, lossy JSON view of the live model,
and the default thing to read. (`model_debug` is the Rust-`Debug` pretty-print:
the human/eyeball view, strictly more faithful exactly where `model` is lossy —
full depth, construction order — but opaque text; don't parse it. Before
protocol v4, `model` carried that text and there was no structured view — gate
on `GET /`'s `protocol_version` before reading `model` as data.)
Plain data maps structurally
(records as objects, lists as arrays, numbers/strings/bools as themselves);
everything else becomes a sigil-keyed object no source-authored record field
can collide with (`$` is not a Functor Lang identifier character — though a record
key that arrived off the network in a typed message can carry one, so treat
the sigils as a strong convention rather than a proof):

```jsonc
{"$tuple": [1.0, 2.0]}                 // tuples, kept distinct from lists
{"$ctor": "Playing", "args": [7.0]}    // variants: constructor + positional args
{"$fn": "<fn(dt)>"}                    // closures/callables (their Display form)
{"$host": "SceneNode"}                 // opaque host values
{"$number": "NaN"}                     // NaN/Infinity/-Infinity (JS spellings)
{"$truncated": "max depth"}            // nesting past the bound (120 emitted
                                       // containers — within what stock JSON
                                       // parsers accept)
```

It is a one-way observation format (there is deliberately no parser back),
and it is `null` for producers without a structured model (e.g. `--replay`,
whose `model_debug` still describes the replay position).

### `POST /input`

JSON is tagged by `type`. Unknown keys/shapes return **400** with a message.

```jsonc
{"type":"key","key":"w","down":true}      // key press / release
{"type":"mouse_move","x":10,"y":20}       // absolute cursor position
{"type":"mouse_wheel","delta":1}          // scroll
{"type":"mouse_button","button":"left","down":true}            // "left"|"right"|"middle"
{"type":"ui_event","slot":0,"kind":"Clicked"}                  // click widget slot 0
{"type":"ui_event","slot":1,"kind":{"SliderChanged":0.5}}      // drag slider slot 1
{"type":"ui_event","slot":2,"kind":{"TextChanged":"hi"}}       // edit text input slot 2
{"type":"xr","left":{...},"right":{...},"head":{...}}          // set the XR device sample
{"type":"xr_clear"}                                            // drop it again
```

`mouse_button` is both an edge and level state, exactly like `key`: it calls the
game's `mouseButton` hook AND updates the held buttons that later steps'
`sampledInput` sees (`mouse.buttons`), so holding one across several
`/time advance` steps scripts full-auto fire. A release is delivered only if the
game saw the press. Unlike window buttons it ignores cursor capture — a
headless/hidden session has no capture to acquire.

`ui_event` drives the game's interactive UI widgets without pixels or
hit-testing (docs/ui-interaction.md): `slot` is the widget's index in the
frame's `ui(model)` tree, in construction order over the interactive widgets.
An event for a slot the current view doesn't have is dropped (with a one-line
runtime report), and the endpoint still returns 200 — delivery, not handling,
is what's acknowledged.

#### `{"type":"xr"}` — inject tracked poses (desktop)

XR is **sampled**, not evented, so this command does not call an entry point: it
sets the XR sample that every following fixed step feeds to `sampledInput`,
through the exact path a headset takes. That means it also lands in the recorded
input log, so a scripted pose sequence replays identically.

The body is a whole [`xr` sample](#sampled-input-in-get-state) — the same shape
`GET /state` reports — and it is **level state**, like a held key: it stays in
force until the next `xr` command replaces it, or `{"type":"xr_clear"}` releases
it (restoring the `--emulate-xr` rig, or no `xr` domain at all). Every field is
optional and takes its default when omitted (hand inactive, no pose, `0.0`, an
**identity** orientation), so name both hands each step rather than relying on a
partial body to merge. An **unknown** field is a 400, not a default — a
misspelled `"triger"` would otherwise succeed and pin the game to a
nothing-tracked sample:

```sh
curl -s -X POST $H/input -d '{
  "type": "xr",
  "head":  { "position": [0.0, 0.0, 0.0] },
  "left":  { "active": true,
             "grip": { "position": [-0.3, -0.1, -0.6],
                       "orientation": [0.0, 0.38, 0.0, 0.92] } },
  "right": { "active": true,
             "grip": { "position": [-0.05, -0.05, 0.12] },
             "trigger": 1.0 }
}'
```

An injected sample **overrides `--emulate-xr`** and supplies the `xr` domain even
without it. That is the point: the mouse/keyboard emulator pins both hands to
`z = -0.55` with identity orientations, so gestures like pulling a hand back
toward your face, or aiming with a rotated grip, are inexpressible there and can
only be driven this way. `held_keys` and `mouse` stay live alongside it.

Pair it with `POST /time` to step one frame per pose — and **wait for `frame` to
increment before sending the next pose**. Advances accumulate (one advance is
one stepped frame), but injected input is *level* state, not a queue: it is
applied when the request is serviced, so any steps still queued when the next
pose arrives run against the NEW pose. Waiting is what pins one pose to one
frame:

```sh
for i in $(seq 0 20); do
  before=$(curl -s $H/state | jq .frame)
  curl -s -X POST $H/input -d "$(pose_at $i)" >/dev/null   # your pose generator
  curl -s -X POST $H/time  -d '{"type":"advance","dts":0.016}' >/dev/null
  until [ "$(curl -s $H/state | jq .frame)" -gt "$before" ]; do :; done
done
curl -s $H/state | jq -r .model
```

`e2e/xr-pose-injection.mjs` is the same loop in JavaScript, with assertions.

The device runtime rejects this command with **400**: Quest resamples the domain
from live OpenXR tracking every frame, so an injected sample could never be seen.

### `--input-script` — deterministic offline input

`POST /input` needs a live driver. For a *reproducible* run — a golden still, a
regression capture — use `--input-script <file>` instead: the runner replays the
file against a fixed `--script-dt` per frame, so frame N is always the same sim
state, and `--capture-at-frame N` grabs a byte-identical still every time.

Each non-blank line is `<frame> <control> <down|up>`; `#` starts a comment.
A `<control>` is either a KEY name (`Right`, `Up`, `A`, `Space` — the same
`Key::from_name` spelling `POST /input` uses) or a MOUSE BUTTON written with an
explicit `Mouse.` prefix:

```
0  Right       down     # hold the right arrow key from frame 0
2  2           down     # digit keys are bare: `2`, not `Num2`
3  2           up
4  Mouse.Left  down     # press and hold the left mouse button
28 Mouse.Left  up
```

The `Mouse.` prefix is **required**, and is the whole disambiguation: `Left`,
`Right` and `Middle` are valid *key* names too, so a bare button name would
silently script the arrow key. `Mouse.Left` is also exactly the spelling the
game's `mouseButton` hook receives. Buttons carry the same edge + level
semantics as injected ones — the `mouseButton` hook fires AND `mouse.buttons`
updates for later `sampledInput` steps, so holding one scripts full-auto fire
(`examples/shooting-range/firing.input` is the worked example).

A `Mouse.* up` with no preceding press is a **parse error**, not a silently
dropped line: live playback would suppress it while the `--ghost` forward-step
preview would replay it, so a stray release is rejected rather than allowed to
make the preview and the real run disagree.

Pointer MOTION is not scriptable yet — `mouse_move` is injection-only, since it
needs a two-coordinate line shape rather than this `<control> <down|up>` triple.

For example, this captures deterministic gameplay at zero-based simulation
frame 120 (one 60 Hz step per rendered frame):

```sh
./target/debug/functor -d examples/hello run native \
  --input-script actions.input --script-dt 0.016666667 \
  --capture-at-frame 120 --capture-frame scripted.png \
  --capture-size 1280x720
```

Do not add `--fixed-time`: clap rejects that combination because fixed time
means `dts = 0`, while scripted playback advances by `--script-dt`.

### Sampled input in `GET /state`

`input` is runtime-owned data sampled for one simulation frame. Keyboard and
mouse keep their existing event entry points; continuously sampled devices add
typed sibling domains to the same record. Quest currently adds `xr` while head
tracking is valid:

```jsonc
{
  "held_keys": [],
  "mouse": { "x": 0, "y": 0, "buttons": { "left": false, "right": false, "middle": false } },
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
{"type":"set","tts":2.0}                    // PAUSE: pin game time to a constant (dts=0)
{"type":"advance","dts":0.016}              // STEP: run exactly one frame with this dt, then hold
{"type":"advance","dts":0.016,"frames":120} // BATCH: queue 120 such steps in one request
{"type":"resume"}                           // RESUME: follow wall-clock again
```

**Advances accumulate.** Each queued step runs exactly once, in order: `n`
advances always run `n` model steps, whenever they arrive relative to a frame.
A single advance is therefore one observable stepped frame — step, read
`/state`, step again.

**`frames` is the batch form** (default `1`), for skipping ahead when you do
*not* need to observe between steps: one round trip instead of `n`, and the
queue drains at up to 8 steps per rendered frame rather than one, so a
600-frame skip costs ~75 rendered frames instead of 600. The clock parks at the
end exactly as a single advance does.

Two things follow from that speed. The response returns when the steps are
*queued*, so poll `GET /state` until **`pending_steps` is 0** to know the batch
has fully landed. And because up to 8 ticks run inside one rendered frame, a
batch has ~`n/8` of the per-rendered-frame I/O points that `n` single advances
would give it — network delivery, effect results, injected input, and rendering
all happen once per rendered frame, not once per tick. Step one at a time when
the game needs to see input or I/O between steps. (Batches are capped at
1,000,000 queued steps; past that the request is a 409.)

**`--fixed-time <T>` is not an initial `set`.** It is an *unconditional* capture
pin: every frame is `{dts: 0, tts: T}`, and no clock control — pause, step,
rebase — can move it. That is what makes golden captures byte-identical, so
`/time` does not get to weaken it: `set`, `advance`, and `resume` all return
**409 Conflict** with a message naming the pin while `--fixed-time` is in
effect. To start pinned *and* step, launch WITHOUT `--fixed-time` and
`POST {"type":"set","tts":T}` first.

While the clock is pinned (either way), **user keyboard/mouse input from the window is
ignored**, but injected `/input` still applies — so an external driver has deterministic
control.

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

### `GET /project` — read the running sources back (protocol v5)

The reply is a JSON array of `[path, source]` pairs, entry first — exactly the
body `/reload-project` accepts, so the two compose:

```sh
curl -s http://127.0.0.1:8123/project | jq -r '.[0][1]' > game.fun
```

After a `/reload-source` or `/reload-project` push, this is the only place the
program's current source exists: the runtime is running text that may never
have been written to a file (an agent authoring over the wire, the browser
IDE). It reports the PROJECT's own modules — bundled prelude/stdlib modules
are excluded — by file name. A producer whose logic is not source-shaped
(`--replay`) answers **501**, and a pre-v5 runtime answers **404**.

`functor mcp`'s `save_project` tool is built on it (docs/mcp.md).

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
curl -s -X POST $H/input -d '{"type":"mouse_button","button":"left","down":true}'
curl -s -X POST $H/input -d '{"type":"mouse_move","x":320,"y":180}'
curl -s -X POST $H/time  -d '{"type":"advance","dts":0.016,"frames":4}'
until [ "$(curl -s $H/state | jq .pending_steps)" -eq 0 ]; do :; done
curl -s $H/state | jq .model                                     # model after all queued ticks
curl -s -X POST $H/capture -o step.png
```

`pending_steps == 0` means the requested clock ticks have drained. HTTP, WebSocket,
audio, and other asynchronous effects may complete later; poll an application-specific
model condition when the workflow depends on one of those results.

Digit keys use the same bare wire spelling here as in input scripts:
`{"type":"key","key":"2","down":true}`, not `"Num2"`.

## Tooling

A typed TypeScript SDK over these endpoints (single-client + a multi-client lockstep
session for simulating multiplayer games) lives in `tools/functor-sdk`. A client can
point at either `http://127.0.0.1:8077` (desktop) or the adb-forwarded
`http://127.0.0.1:8123` (Quest) without changing API calls.

An **MCP server** over the same endpoints ships as `functor mcp` — sessions,
state, scene, capture, input, time, and rewind as standard tools for coding
agents (docs/mcp.md).

## Future directions

- **Multiplayer simulation.** Launch N runner instances, each on its own
  `--debug-port`, networked via `Sub.connect`/`Sub.listen`; pin all clocks and step
  them in lockstep, injecting input and observing state per client. This is the
  out-of-process counterpart to the in-process `functor-netsim` harness.
