# Debug runtime

An HTTP control server shared by the desktop and Quest runtimes that lets
an external client — a script, a test, or an LLM — **observe** and **drive** a running
game without a GPU window of its own. It is the runtime arm of Functor's LLM-native
goal: capture frames, query state, control the frame clock, and inject input over a
localhost socket.

On desktop, **`functor develop` starts it by default** on the well-known localhost
port **8077** — so a script or agent can attach to a live develop session without
being told a port:

```sh
./target/debug/functor -d examples/hello develop native
curl -s localhost:8077/ | jq
```

`run` stays opt-in: pass `--debug-port <PORT>` there (or on `develop`, to choose a
different port).

```sh
# the CLI runs the game in-process and interprets the .fun
./target/debug/functor -d examples/hello run native --debug-port 8077
```

If 8077 is already taken — a second `functor develop`, or a stale process —
`develop` logs one line and runs the game **without** a debug server rather than
failing to start; pass `--debug-port <PORT>` for a second session, or `--no-debug`
to skip the listener entirely. An explicit `--debug-port` that can't bind is still
a fatal error (a driver asked for *that* port and is waiting on it).

`--debug-port 0` binds an **OS-assigned free port**; the `[debug-server]
listening on http://…` stderr line always reports the ACTUAL bound port, so
automation parses it instead of assuming the requested one. This is the TS
SDK `launch()` default — parallel sessions can't collide, and the actual port
is on `runner.port`. Hard-code a port only when something external must find
the server at a known address (e.g. `adb forward` on device, or the VS Code
inspector, whose default is 8077 — point it at its own port while a `develop`
session is up).

The server binds **localhost by default** (`127.0.0.1:<PORT>`); `--debug-bind 0.0.0.0`
exposes it to the LAN for remote develop (see `POST /reload-source`) — there is no auth,
so bind wide only on networks where arbitrary game-code pushes are acceptable. That is
why only the *localhost* bind is on by default under `develop`: reaching it already
requires code execution on the machine. A bind is never widened implicitly —
`develop --debug-bind …` starts no server unless `--debug-port` is given too. HTTP
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

## Endpoints

`GET /` returns this list as JSON (discoverability).

| Method & path | Purpose |
| --- | --- |
| `POST /capture` | PNG (`image/png`) of the next rendered frame |
| `GET /state` | runtime state JSON: `frame`, `tts`, `model_revision` + `pending_net` (protocol v10 — see below), combined/legacy `viewport`, `views` (`main` on desktop; `left` + `right` on Quest), `input` (keyboard/mouse held + pressed/released sets and optional typed device domains), `model` (structured JSON — see below), `model_debug` (Rust `Debug` text) |
| `GET /scene` | current frame as JSON: `camera` + `scene` + `lights` |
| `GET /trace` | paused-inspector trace: the last real frame's entry-point invocations plus a synthesized `draw` pass, replayed while paused. Each site (binders AND variable reads, `site`) carries the full `value`, a depth-limited `preview`, and `kind` (primitive/composite — the editor's inline-vs-hover policy); `{ "paused": false, "invocations": [] }` while playing. Each invocation carries its returned value both as text (`result`, `result_preview`) and as STRUCTURE (`result_json`, the same grammar as `/state`'s `model` — so a client can tree it instead of parsing a rendering), under a 1 MiB budget shared by the document's structured results (the `result`/`preview` TEXT is not bounded by it): a value that would exceed the remaining budget is emitted as `{"$truncated": "trace budget"}` with `result_json_truncated: true` on that invocation, and the first refusal spends the rest of the budget. Paused docs also carry `coverage` (per-file span starts with the frame OFFSETS they executed on, over a ±120-frame journal ring — positive offsets appear when scrubbed behind the live head) and `runnable` (the static could-run set) — the recency gutter's data |
| `POST /input` | inject input (see below) |
| `POST /time` | control the frame clock (see below) |
| `POST /reload-source` | swap game logic from the request body (see below) |
| `POST /reload-project` | swap all sibling modules from a JSON array of `[path, source]` pairs, entry first; `?module=`/`?prefix=` declares the same-file entry role |
| `POST /load-project` | start a new sibling-module project from the same body, initializing its model from `init`; takes the same role query |
| `GET /project` | the running program's own `.fun` sources as a JSON array of `[path, source]` pairs, entry first — the read half of the push routes (see below) |
| `POST /reload-asset` | upload one project-relative texture/model/audio asset as a binary path+bytes envelope |
| `POST /sync-assets` | finish a sync from a JSON array of current asset paths; uploaded paths absent from the manifest are removed |
| `POST /rewind` | restore recorded model + physics to `{"frame":42}` (pin the clock first) |
| `GET /net/outbound` | **embedder transport only** — take-and-consume the game's queued `ConnCommand`s (see below) |
| `POST /net/deliver` | **embedder transport only** — deliver inbound network events into the game (see below) |

### `model` in `GET /state`

`model` is the structured view — a total, lossy JSON view of the live model,
and the default thing to read. (`model_debug` is the Rust-`Debug` pretty-print:
the human/eyeball view, strictly more faithful exactly where `model` is lossy —
full depth, construction order — but opaque text; don't parse it. Before
protocol v4, `model` carried that text and there was no structured view — gate
on `GET /`'s `protocol_version` before reading `model` as data.)
Plain data maps structurally
(records as objects, lists as arrays, immutable Maps as a `$map` entry list,
numbers/strings/bools as themselves);
everything else becomes a sigil-keyed object no source-authored record field
can collide with (`$` is not a Functor Lang identifier character — though a record
key that arrived off the network in a typed message can carry one, so treat
the sigils as a strong convention rather than a proof):

```jsonc
{"$map": [["a", 1.0], ["b", 2.0]]}      // Maps, in canonical key order
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

### `model_revision` and `pending_net` in `GET /state` (protocol v10)

**Pause freezes the CLOCK, not the network.** `POST /time {"type":"set"}` stops
time advancing; it does not stop the shell's sockets. A paused session keeps
accepting inbound messages and keeps folding them through `update`, so its
model changes while `frame` and `tts` stand perfectly still.

That makes **`frame` not a version label for a networked model** — a driver
that snapshots the model, waits, and compares `frame` to decide whether
anything happened will conclude "nothing did" while the authority has quietly
seated three players. `model_revision` is the label to use instead:

- **`model_revision`** counts how many times the model has been REPLACED by
  game logic since the program loaded — every entry point's return and every
  effect or network fold, counted at the producer's single model assignment.
  Monotone, never reset. Compare it against a previous read to answer "did
  anything land", whether or not the clock moved. Replacing the model from
  *outside* the game does not count — a hot reload (which rebinds it),
  `/load-project`, `/rewind`, a timeline seek — because those are operations
  the driver itself just performed and each answers with fresh state to
  re-baseline from.
- **`pending_net`** is how many inbound network events the shell has accepted
  from its transport but not yet delivered to the game — connection events and
  completed HTTP responses. **Poll until it is 0 to know a session is
  quiescent** before snapshotting a baseline. It cannot see a packet still on
  the wire between two processes, so it is a lower bound on outstanding
  network work: it proves nothing already received is unprocessed, not that
  nothing is coming.

Both are additive, so a pre-v10 runtime simply omits them and they read `0`.
A client that WAITS on either must gate on `GET /`'s `protocol_version` first —
a constant zero from an old runtime is indistinguishable from real quiescence.

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
{"type":"gamepad","left_stick":[0.0,1.0],"south":true}         // set the gamepad sample
{"type":"gamepad_clear"}                                       // drop it again
```

`mouse_button` is both an edge and level state, exactly like `key`: it calls the
game's `mouseButton` hook AND updates the held buttons that later steps'
`sampledInput` sees (`mouse.buttons`), so holding one across several
`/time advance` steps scripts full-auto fire. A release is delivered only if the
game saw the press. Unlike window buttons it ignores cursor capture — a
headless/hidden session has no capture to acquire.

The next fixed step also sees the transition in `pressedKeys`/`releasedKeys` or
`mouse.pressed`/`mouse.released`. Repeating a down command while it is already
held preserves the legacy event-hook call but does not create another sampled
press. A down/up burst before one step can appear in both sampled edge sets;
the held level reports the final state. After that step the edge fields clear,
including under `--fixed-time`, recording/replay, and forward projection.

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

`{"type":"gamepad"}` is the same contract for the gamepad domain: sampled
level state (no entry point call, recorded, replayable), a whole-sample
replacement whose omitted fields take their defaults (centered sticks,
released triggers/buttons), `deny_unknown_fields`, and `{"type":"gamepad_clear"}`
as the release half — today restoring no `gamepad` domain at all (no shell
polls a physical pad yet, so injection is the domain's only source). The
body is the snake_case [`gamepad` sample]
(#sampled-input-in-get-state) `GET /state` reports: `left_stick`/`right_stick`
(`[-1..1]`, up-positive Y), `left_trigger`/`right_trigger` (`0..1`), and the
positional booleans `south`/`east`/`west`/`north`, `left_bumper`/`right_bumper`,
`left_stick_pressed`/`right_stick_pressed`, `dpad_up`/`dpad_down`/`dpad_left`/
`dpad_right`, `start`/`select`.

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

Each non-blank line is one of these two forms; `#` starts a comment:

```text
<frame> <Key|Mouse.Button> <down|up>
<frame> Mouse.Move <x> <y>
```

A control is either a KEY name (`Right`, `Up`, `A`, `Space` — the same
`Key::from_name` spelling `POST /input` uses), a MOUSE BUTTON written with an
explicit `Mouse.` prefix, or the special `Mouse.Move` pointer form:

```
0  Right       down     # hold the right arrow key from frame 0
0  Mouse.Move  400 300  # establish the pointer at a logical point
1  Mouse.Move  560 220  # aim by moving before this frame's tick
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
The scripted transition also appears once in `mouse.pressed` or
`mouse.released` on that frame's sampled snapshot.

`Mouse.Move` coordinates are signed **logical points** with a top-left origin,
in exactly the same space as `Input.mouse.x` / `.y` and the debug state's
`input.mouse.surface_width` / `surface_height`. They are not framebuffer
pixels: on a Retina window the logical surface may be 800×600 while `viewport`
is 1600×1200. Motion is delivered before the named frame's `sampledInput` and
`tick`, and the new position is carried into that sampled snapshot and replay
history.

With `--emulate-xr`, the position drives the synthesized controller sample and
does not also invoke the legacy `mouseMove` hook, matching `POST /input`.

Scripts and `POST /input` intentionally preserve negative and out-of-surface
coordinates instead of rejecting them. Native pointer events can report the
same transient values, and clamping only one injection path would make replay
diverge from live input. A mapping API such as `Camera3D.toWorldRay` therefore
continues to return `None` outside the half-open logical surface; drivers can
read the published surface extent and choose or validate an in-range point.

A `Mouse.* up` with no preceding press is a **parse error**, not a silently
dropped line: live playback would suppress it while the forward-step trajectory
preview would replay it, so a stray release is rejected rather than allowed to
make the preview and the real run disagree.

### Sampled input in `GET /state`

`input` is runtime-owned data sampled for one fixed simulation step. Keyboard and
mouse keep their existing event entry points while also exposing deterministic
held and pressed/released sets. Quest adds `xr` while head tracking is valid;
`gamepad` appears while a pad sample is live (today: injected):

These edge fields were added in debug protocol v6. Against an older runtime,
clients should treat absent edge arrays/button sets as empty.

```jsonc
{
  "held_keys": [],
  "pressed_keys": [],
  "released_keys": [],
  "mouse": {
    "x": 0,
    "y": 0,
    "surface_width": 800,
    "surface_height": 600,
    "buttons": { "left": false, "right": false, "middle": false },
    "pressed": { "left": false, "right": false, "middle": false },
    "released": { "left": false, "right": false, "middle": false }
  },
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
  },
  "gamepad": {
    "left_stick": [0.0, 0.0],
    "right_stick": [0.0, 0.0],
    "left_trigger": 0.0,
    "right_trigger": 0.0,
    "south": false,
    "east": false,
    "west": false,
    "north": false,
    "left_bumper": false,
    "right_bumper": false,
    "left_stick_pressed": false,
    "right_stick_pressed": false,
    "dpad_up": false,
    "dpad_down": false,
    "dpad_left": false,
    "dpad_right": false,
    "start": false,
    "select": false
  }
}
```

`surface_width` and `surface_height` were added in debug protocol v7. They are
the logical pointer surface in the same coordinate space as `x` and `y`, not
the framebuffer dimensions in top-level `viewport` (which are `0×0` in
headless mode). A client that needs resize-correct pointer mapping must require
v7 or newer rather than infer the logical extent from `viewport`.

Tracked poses use OpenXR's rig-local convention: +X right, +Y up, -Z forward;
quaternions are `[x, y, z, w]`. Head and controller poses are relative to the
same center-eye reference that anchors the authored `Frame.camera`, so a game
can map them through the camera from the same model update without mixing
tracking-space coordinates into portable game state. `active` means an input
source is available for that hand. Grip and aim are independently nullable
because buttons can remain available during a temporary pose-tracking loss.
Analog values are normalized to `0..1`; thumbstick axes to `-1..1`.

Non-XR runtimes omit `xr`, and padless runtimes omit `gamepad` — each domain
is a typed sibling field, present only when its device is live. `gamepad`
carries the pad's held state: sticks as `[x, y]` in `-1..1` with up-positive
Y, triggers in `0..1`, positional face buttons (`south` is the bottom one),
bumpers, stick clicks, dpad, and `start`/`select`. No shell polls a physical
pad yet, so today the domain appears only while a sample is injected
(`{"type":"gamepad"}` above). Future mobile-touch support should add another
typed sibling field rather than target-specific endpoints or string-keyed
capability bags.

### `POST /time` — frame-loop control

```jsonc
{"type":"set","tts":2.0}                    // PAUSE: pin game time to a constant (dts=0)
{"type":"advance","dts":0.016}              // STEP: run exactly one frame with this dt, then hold
{"type":"advance","dts":0.016,"frames":120} // BATCH: queue 120 such steps in one request
{"type":"cancel"}                           // ABORT: drop queued steps, keep current tts, stay paused
{"type":"resume"}                           // RESUME: follow wall-clock again
```

`cancel` was added in debug protocol v8. A client that depends on confirmed
queued-step cleanup must reject older runtimes rather than treating a 404 or an
unknown command as a successful abort.

`advance.dts` must be finite and positive; invalid values are rejected before
anything is queued, so direct debug clients cannot move simulation time
backwards.

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

`cancel` clears both queued debug steps and fixed-frame catch-up without
rebasing `tts`; the model and clock remain aligned at the last step that
actually landed. It is the safe error/deadline cleanup for a batch an SDK or
submitted-code driver no longer intends to finish.

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

#### Declaring a same-file entry role (protocol v9)

A device session has no command line, so a project push declares which
same-file role to boot in the route's **query string** — the same two forms
functor.json and the web page's boot config carry:

```sh
curl -X POST --data-binary @project.json \
  'http://127.0.0.1:8123/load-project?module=Server'   # an inline `module Server { … }`
curl -X POST --data-binary @project.json \
  'http://127.0.0.1:8123/reload-project?prefix=server' # serverInit/serverTick/…
```

The runtime **re-resolves** the role against each pushed program, so an edit
that renames or deletes the block fails with an error naming it and the old
program keeps running — under the role it was already running.

Declaring nothing is not the same as declaring the plain contract: a push with
no role query leaves the role already in force alone (so `functor push` and the
MCP tools need no revision), while `?prefix=` explicitly selects the unprefixed
contract. `functor run vr` always declares, including `?prefix=` for a plain
project. A pre-v9 runtime ignores the query and boots the unprefixed contract;
a role declared twice, or one that is not an identifier, is a **400**.

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

### The embedder transport (protocol v11)

A runtime started with `--net-transport embedder` opens **no socket**. Its
persistent-connection traffic — everything `Sub.listen`, `Sub.connect`,
`Effect.send` and `Effect.close` produce — stays queued for the debug client,
and inbound events arrive from it. **The client is the network.** The default is
unchanged (`--net-transport sockets`): the runner dispatches to the real
tungstenite host exactly as before, and both endpoints below answer **409**,
because draining there would steal the real dispatcher's commands and
delivering there would inject events no peer sent.

This is the native half of the seam the web runtime already has
(`window.__functorNetTransport = "embedder"`, and the browser coordinator in
`site/src/net-coordinator.ts`). Natively, "the embedder" is whatever process
drives the debug server — for `functor mcp`'s session groups, that is the MCP
host itself (docs/mcp.md).

**`GET /net/outbound`** returns and CONSUMES the queued commands, as the
versioned `ConnCommand` JSON every host consumes (serde's externally-tagged
representation, byte payloads included):

```jsonc
[ {"Listen":  {"key":"127.0.0.1:9101","addr":"127.0.0.1:9101"}},
  {"Connect": {"key":"ws://127.0.0.1:9101/orbs","url":"ws://127.0.0.1:9101/orbs"}},
  {"Send":    {"conn":1,"payload":[102,117,110,58]}},
  {"CloseConn": {"conn":1}},
  {"CloseKey":  {"key":"127.0.0.1:9101"}} ]
```

**`POST /net/deliver`** takes a JSON array of events, the shape mirroring the
four producer push methods one-for-one (`key` is the routing key of the
`connect`/`listen` the event belongs to; a message is TEXT, as a real WebSocket
hands it over):

```jsonc
[ {"kind":"connected",    "key":"ws://127.0.0.1:9101/orbs","conn":1},
  {"kind":"message",      "key":"ws://127.0.0.1:9101/orbs","conn":1,"text":"hi"},
  {"kind":"disconnected", "key":"ws://127.0.0.1:9101/orbs","conn":1},
  {"kind":"error",        "key":"ws://127.0.0.1:9101/orbs","conn":1,"message":"…"} ]
```

The two directions use different shapes on purpose: egress is the already
versioned logic↔shell type, and ingress mirrors the push methods it feeds. A
negative `conn` is a **400** (it would reach the game as `u64::MAX`), and so is
a malformed batch — neither reaches the runtime loop.

Delivery is **synchronous**: each event folds through `update` before the
response, so a `200` means the model has already absorbed the batch. That is
what lets a driver route a packet and then step, and know the step saw it.
`pending_net` therefore stays 0 on this transport — nothing waits in a shell
channel.

`--net-transport embedder` without `--debug-port` is refused at startup: with no
client there is nothing to drain the queue or deliver into it, so the game's
network would silently be a black hole with an unbounded backlog behind it.

The device runtime answers 409 for both: a Quest session's network is a real
socket to a real peer, with no coordinator behind adb.

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

An **MCP server** over the same endpoints ships as `functor mcp` — sessions,
state, scene, capture, input, time, and rewind as standard tools for coding
agents (docs/mcp.md).

## Future directions

- **Multiplayer simulation.** Launch N runner instances, each on its own
  `--debug-port`, networked via `Sub.connect`/`Sub.listen`; pin all clocks and step
  them in lockstep, injecting input and observing state per client. This is the
  out-of-process counterpart to the browser's hosted panes + net coordinator
  (`docs/multiplayer.md`).
