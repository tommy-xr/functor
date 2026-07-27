# `functor mcp` — MCP over the debug runtime

`functor mcp` is an [MCP](https://modelcontextprotocol.io) server: it exposes the
[debug runtime](debug-runtime.md) — launch a game, read its model, pause it,
inject input, step the clock, capture a frame — as standard tools, over stdio.

The capability is not new; the *interface* is. Any MCP-speaking agent can now
drive a Functor game with no bespoke script, no HTTP plumbing, and no screen:
`launch_game` → `pause` → `send_input` → `step` → `get_state`, reading the
model back as structured JSON.

It is a plain HTTP client of the runtimes and lives entirely in the CLI. The
runtimes are unchanged.

## Registering it

With Claude Code:

```sh
claude mcp add functor -- functor mcp
```

For any other MCP client, the generic stdio-server config shape is:

```jsonc
{
  "mcpServers": {
    "functor": {
      "command": "functor",       // or an absolute path to the binary
      "args": ["mcp"]
    }
  }
}
```

`functor mcp` speaks JSON-RPC on **stdout**, so it takes no `-d` and prints
nothing else there — each game names its own directory when it is launched, and
a launched runtime's own output is captured rather than inherited.

## The tools

**Sessions.** The server manages N concurrent games at once. A session is a base
URL plus, when the server launched it, the child process.

| Tool | What it does |
| --- | --- |
| `launch_game` | Spawn a game as a child on a free port and return its session id. `mode` is `hidden` (default) or `headless` — see below. |
| `connect_game` | Attach to a runtime this server does **not** own (someone else's `--debug-port`, or an adb-forwarded Quest). |
| `list_sessions` | Every session: id, url, owned/attached, and whether it currently answers. |
| `stop_game` | Kill a launched game; merely forget an attached one. |

**Observing.**

| Tool | What it does |
| --- | --- |
| `get_state` | `frame`, `tts`, `pending_steps`, viewports, sampled `input`, and the model — read `model_json`, the structured view. |
| `get_scene` | The camera, scene graph, and lights `draw` produced. Pure data, so it works headlessly. |
| `get_trace` | The paused inspector trace: every entry point's binder and variable values for the last real frame. Pause first. |
| `capture_frame` | A PNG of the next rendered frame, returned as an MCP image block. |

**Driving.**

| Tool | What it does |
| --- | --- |
| `pause` | Pin the clock (defaults to the current `tts`), so nothing advances on its own. |
| `step` | Run `frames` steps of `dts` each, **wait for them to land**, and return the fresh state. |
| `resume` | Follow wall-clock time again. |
| `send_input` | Inject one `POST /input` command verbatim — key, mouse move/wheel/button, `ui_event`, or an `xr` sample. |
| `rewind` | Restore model + physics to a recorded frame (it pins the clock first, as `/rewind` requires). |
| `reload_source` / `reload_project` | Hot-reload the entry, or every sibling module, with the live model preserved. |

Two semantics are worth internalizing, because they are what make the loop
deterministic rather than racy:

- **`step` waits.** `POST /time advance` only *queues* steps. `step` polls until
  `pending_steps` is 0 before returning, so a caller can never read a
  half-landed batch. A batch (`frames > 1`) runs up to 8 ticks per rendered
  frame, so it has proportionally fewer input/network/render points — step one
  at a time when the game must see input or I/O between steps.
- **Input is level state.** A key, a held mouse button, and an injected XR sample
  stay in force across steps until released or replaced. That is how a paused
  session is scripted: press, step a few frames, release.

Runtime errors come back as tool errors carrying the runtime's own message — a
`/input` 400 explaining a misspelled field, a `/reload-source` 400 with the
rendered load error, a `/time` 409 naming a `--fixed-time` pin.

`launch_game` and `connect_game` both require **debug protocol v4 or newer**
(`docs/debug-runtime.md`), and say so if the runtime is older. Below that, the
guarantees above quietly stop holding: a pre-v3 runtime ignores a batched
`frames` and reports no `pending_steps` (so `step` would call a ten-frame batch
landed after one), and a pre-v4 one never sends `model_json`. This matters most
for a device APK, which versions independently of the CLI — rebuild it from the
same Functor version.

Launched games are killed when the server stops, whether its client closes
stdin or signals it (SIGTERM/Ctrl-C). Attached ones are always left running.

## `hidden` vs `headless`

`launch_game`'s `mode` chooses which one:

- **`hidden`** (default) creates a real GL context in a window that is never
  shown and never takes focus. Rendering happens, so **`capture_frame` returns
  pixels**. Needs a display/GPU.
- **`headless`** creates no GL context at all — no display, no GPU, ideal for CI
  or a remote box. `get_state`, `get_scene`, `send_input`, and `step` all work
  (the game's `draw` is pure data), but there is nothing to read back:
  **`capture_frame` fails** with an explanation. Audio is silent too, so
  `Audio.playThen` completion messages are not delivered.

## Driving a game deterministically

```jsonc
launch_game { "dir": "examples/counter", "mode": "headless" }
// → {"session":"s1","url":"http://127.0.0.1:53127","port":53127,"owned":true,…}

pause      { "session": "s1" }                       // pin the clock where it is
send_input { "session": "s1",
             "command": {"type":"ui_event","slot":0,"kind":"Clicked"} }
step       { "session": "s1" }                       // one frame, waited for
// → {"frame":9,"tts":0.13,"pending_steps":0,…,"model_json":{"count":1}}
stop_game  { "session": "s1" }
```

Nothing advanced between the click and the step, and `step` returned only once
the step had actually run — so `model_json.count` is the click's effect, not a
sample of a still-moving game.

## Connecting to a Quest

The device runtime serves the same protocol on loopback port 8123. Forward it
over USB, then attach:

```sh
adb forward tcp:8123 tcp:8123
```

```jsonc
connect_game { "url": "http://127.0.0.1:8123" }
```

The session is **attached**, not owned: `stop_game` forgets it and leaves the
headset running. Everything else is identical — `get_state`, `send_input`,
`step`, `reload_project`. `capture_frame` returns the side-by-side PNG of the
two raw eye buffers, and `/state.views` has `left` and `right` rather than
`main`. Injected `xr` samples are rejected on device (Quest resamples live
tracking every frame).

## See also

- [`docs/debug-runtime.md`](debug-runtime.md) — the HTTP surface these tools wrap,
  with the exact request/response shapes.
- `e2e/mcp-server.mjs` — the end-to-end proof, speaking raw JSON-RPC to the server.
- `tools/functor-sdk` — the typed TypeScript SDK over the same endpoints, for
  scripts rather than agents.
