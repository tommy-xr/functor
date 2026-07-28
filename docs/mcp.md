# `functor mcp` — MCP over the debug runtime

`functor mcp` is an [MCP](https://modelcontextprotocol.io) server: it exposes the
[debug runtime](debug-runtime.md) — launch a game, read its model, pause it,
inject input, step the clock, capture a frame — as standard tools, over stdio.

The capability is not new; the *interface* is. Any MCP-speaking agent can now
drive a Functor game with no bespoke script, no HTTP plumbing, and no screen:
`launch_game` → `pause` → `send_input` → `step` → `get_state`, reading the
model back as structured JSON.

It is a plain HTTP client of the runtimes and lives entirely in the CLI. The
one runtime-side addition it needs is `GET /project` (debug protocol v5), the
read half of the project-push routes, which `save_project` is built on.

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
URL plus, when the server launched it, the child process. State-changing calls
on one session share an async operation gate: an overlapping call waits for the
active call to finish, then runs without interleaving. This is mutual exclusion,
not a promised FIFO sequencer; relative waiter order is unspecified. Cancellation
while queued stops waiting and prevents the mutation. Once a mutation acquires
the gate, it runs to its operation boundary even if its MCP response is
cancelled. Exact normalized base-URL aliases created by this MCP server share
the same gate, while different exact URLs continue concurrently. `connect_game`
reserves that gate before discovery, so it cannot race stop and insert a dead
alias afterward. Normalization currently removes trailing `/` only, so hostname
aliases such as `localhost` versus `127.0.0.1` are not recognized, and direct
HTTP clients outside this MCP process are not coordinated. Read-only
`get_state`/`get_scene`/`get_trace` calls do not take the gate.

`stop_game` marks closing before it waits for the gate and completes cleanup
after that mark even if its MCP response is cancelled. Stopping an attached
session closes/removes only that id and leaves other aliases valid. Stopping an
owned session closes the owner, every exact-URL alias, and any pending connect;
keeps those closing records through child kill/wait; then removes the group.
New and already-queued mutations on a closing id reject without runtime I/O.

| Tool | What it does |
| --- | --- |
| `launch_game` | Spawn a game as a child on a free port and return its session id. The project comes from `dir` **or** from `files` — the whole project inline (see below). `mode` is `hidden` (default) or `headless`. |
| `connect_game` | Attach to a runtime this server does **not** own (a human's `functor develop` on port 8077, someone else's `--debug-port`, or an adb-forwarded Quest). |
| `list_sessions` | Every session: id, url, owned/attached, and whether it currently answers. |
| `stop_game` | Kill a launched game and close its exact-URL aliases/pending connects; merely forget one attached id. |

**Observing.**

| Tool | What it does |
| --- | --- |
| `get_state` | `frame`, `tts`, `pending_steps`, viewports, sampled `input`, and the model — the structured JSON view (the default read; `model_debug` is the Debug text). |
| `get_scene` | The camera, scene graph, and lights `draw` produced. Pure data, so it works headlessly. |
| `get_trace` | The paused inspector trace: every entry point's binder and variable values for the last real frame. Pause first. |
| `capture_frame` | A PNG of the next rendered frame, returned as an MCP image block. |

**Driving.**

| Tool | What it does |
| --- | --- |
| `pause` | Pin the clock (defaults to the current `tts`), so nothing advances on its own. |
| `step` | Run 1–10,000 `frames` of `dts` each, **wait for them to land**, and return the fresh state. |
| `resume` | Follow wall-clock time again. |
| `send_input` | Inject one `POST /input` command verbatim — key, mouse move/wheel/button, `ui_event`, or an `xr` sample. |
| `rewind` | Restore model + physics to a recorded frame (it pins the clock first, as `/rewind` requires). |
| `reload_source` / `reload_project` | Hot-reload the entry, or every sibling module, with the live model preserved. |
| `validate_automation_code` | Parse one restricted SDK builder expression without a session or side effects; return its normalized plan, canonical round-trip source, diagnostics, and budget use. |
| `run_automation_code` | Fully validate, then execute a restricted SDK builder expression as one ordered pause/input/step/assert/inspect/capture plan. |

**Authoring.**

| Tool | What it does |
| --- | --- |
| `init_game` | Scaffold a starter project on disk — the same `functor.json` + `game.fun` `functor init` writes. `template` is `"3d"` (default) or `"fps"`. Never overwrites; its `dir` goes straight into `launch_game`. |
| `save_project` | Write a session's **current** source to a directory. The sources come from the RUNTIME (`GET /project`), so they include every wire-only edit. Refuses a directory that already holds a project unless `overwrite`. |

**Learning the language and the API.** Two session-free tools, and they are
different halves: `language_guide` is the LANGUAGE (how to write `.fun` at all),
`api_reference` is the prelude API (what `Scene.cube` takes and returns).

| Tool | What it does |
| --- | --- |
| `language_guide` | The Functor Lang language guide — syntax, semantics, modules, the `init`/`tick`/`draw` game contract, hot-reload rules. No args returns the table of contents plus the quick facts; `section` returns one section's full text. |
| `api_reference` | Search the embedded `.funi` prelude — the same reference `functor docs` renders — by name, qualified path (`Scene.cube`), signature, or doc text. `module` narrows to one module, and lists all of it when `query` is omitted. **No session needed**: it answers before any game is launched. |

`language_guide` exists because Functor Lang is **not** F# or OCaml, and an
agent that guesses from those habits writes parse errors: assignment is `:=`,
pipelines are thread-*last* (`x |> f(a)` is `f(a, x)`), `if/then/else` is only
an expression, and there are no loops and no `<>`. Claude Code agents get this
from the repository's `functor-lang` skill; over MCP, every other client gets
the same text.

It **is** that skill: `.claude/skills/functor-lang/SKILL.md` is embedded in the
binary verbatim and split into sections by its own markdown headings, so there
is no second copy to rot — the skill is already required to track the language
(`CLAUDE.md`), and this surface follows it automatically. Sections are named by
their slugged heading (`syntax-subset`, `semantics-rules-that-will-bite-you`); a
unique fragment (`"game contract"`) resolves too, and an unknown or ambiguous
one comes back as a teaching error naming the candidates. A section runs to the
next heading of **any** level, so a subsection is fetched on its own rather than
inflating its parent — a parent that has any ends with a `Continues in:` line
naming them. Nothing is truncated: the section is the narrowing, exactly as a
`module` listing is for `api_reference`.

```jsonc
language_guide {}                                // TOC + the quick facts
language_guide { "section": "syntax-subset" }    // that section, verbatim
```

API matches come back best-first — the item's own name, then its qualified path,
then its signature, then its prose — capped at 20 per search, with a line
saying so. A `module` listing is never capped: the module is the narrowing.
An unknown module, or a search with neither a query nor a module, answers with
the list of prelude modules rather than an empty result.

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
landed after one), and a pre-v4 one sends Debug text under `model` instead of
structured JSON. This matters most
for a device APK, which versions independently of the CLI — rebuild it from the
same Functor version. `save_project` additionally needs **v5** (`GET /project`)
and says so on an older runtime.

Launched games are killed when the server stops, whether its client closes
stdin or signals it (SIGTERM/Ctrl-C). Attached ones are always left running.

## Restricted automation-code PoC

The two automation-code tools are a deliberately small experiment inspired by
n8n's code-shaped workflow builder. They do **not** add a JavaScript runtime to
`functor mcp`. Submitted source is parsed as one allowlisted fluent expression,
lowered to plain JSON-like plan data, and fully validated before a session is
looked up or any runtime HTTP request is made:

```text
restricted SDK source → lexer/parser → normalized AutomationPlan
                      → budgets + semantic validation
                      → existing typed debug-runtime operations
```

```js
automation("photo mouse-look proof")
  .pause(2)
  .mouseMove(400, 300)
  .mouseMove(600, 200)
  .step({ frames: 2, dts: 0.016 })
  .expectModelClose("yawOffset", -0.6, 0.0001)
  .inspect("settled")
  .capture("proof");
```

Call `validate_automation_code` without a session first. On success it returns
the normalized `plan`, deterministic `canonical_code`, and both used and maximum
budgets. Submitting `canonical_code` to validation again produces the identical
plan. Invalid validation also succeeds as a tool call but returns `valid: false`
with a line/column diagnostic, so an agent can repair source without decoding a
protocol error.

Allowed methods are `pause`, `keyDown`, `keyUp`, `pressKey`, `mouseMove`,
`mouseDown`, `mouseUp`, `mouseWheel`, `uiClick`, `step`, `inspect`,
`expectModel`, `expectModelClose`, and `capture`. `pressKey("r")` lowers to key
down → one waited 16ms step → key up, including a best-effort release if either
the down request or step fails; it is the edge-action shortcut jam authors
repeatedly needed. `expectModelClose(path, expected, absTolerance)` is the
bounded assertion for floating-point game state; both numeric arguments must be
finite and tolerance must be non-negative. Model paths are static strings:
convenient dotted paths (`camera.yawOffset`, `players.0.score`) or JSON Pointers
(`/odd.key/value`).

Mouse coordinates and wheel deltas are signed 32-bit integer literals, matching
the debug protocol; UI slots are unsigned 32-bit integers. Input release becomes
sampled game state on the next step. For example, a proof that releases `d`
should end with `.keyUp("d").step().expectModel("moveX", 0)`, not assert
`moveX` immediately after `keyUp`.

Only literal strings, finite numbers, booleans, nulls, arrays, and objects are
accepted as arguments. Duplicate object keys are rejected. The grammar cannot
represent imports, declarations/variables, callbacks/functions, loops,
classes/`new`, async/await, `eval`, `Function`, `require`, `process`,
`globalThis`, browser globals, fetch/timers, dynamic properties, or unknown
calls. There is no `eval`, `new Function`, Node `vm`, JavaScript engine, module
resolution, filesystem access, or network access in the parser.

The budgets are 16 KiB of source, 64 logical steps, eight levels of literal
nesting, 10,000 total requested frames, and four captures. `run_automation_code`
returns the normalized plan, passed assertions, labeled state observations, and
fresh final state in its first text block. Each `capture` adds a PNG image block
and therefore requires a `hidden` session; headless sessions still support every
data/input/clock operation. Runtime text responses and individual raw capture
responses are each capped at 8 MiB. An automation run additionally caps its
serialized text summary and error text at 4 MiB, all retained raw captures
together at 16 MiB, and their base64 MCP image content at 24 MiB. The encoded
aggregate is checked before base64 construction, then raw captures are consumed
one at a time as image blocks are built. Declared `Content-Length` is checked
before allocation and the same limit is enforced while chunks stream; an error
reports the used and allowed byte counts.

Execution is ordered but not transactional. Complete parse and static
plan-budget rejection is side-effect free, including when a valid mutating
prefix has an invalid suffix. Once a valid plan begins, a later runtime,
assertion, or runtime-output cap failure can leave earlier steps applied. The
session operation gate is held for the whole plan, so concurrent mutating calls
wait and cannot splice input or clock operations into it; calls for other exact
URLs remain independent. This PoC
intentionally has no variables, branching, loops, retry conditions, arbitrary
JavaScript expressions, or rollback.

The complete architecture, jam-friction evaluation, and productionization
questions are in [the PoC note](mcp-automation-code-poc.md).

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

## Authoring a game with no filesystem

An agent with no filesystem of its own (an MCP client in a chat app, say) can
still go from nothing to a running game to a durable project. `launch_game`
takes `files` — `[path, source]` pairs, the entry `.fun` first — instead of
`dir`:

```jsonc
launch_game { "mode": "headless",
              "files": [["game.fun", "type Model = { n: float }\n\nlet init = …"],
                        ["step.fun", "let amount = 1.0\n"]] }
// → {"session":"s1","dir":"/tmp/functor-mcp-…",…}

get_state     { "session": "s1" }                    // → {"model":{"n":0},…}
reload_source { "session": "s1", "source": "…edited…" }   // model preserved
save_project  { "session": "s1", "dir": "./my-game" }
// → {"dir":"/abs/my-game","files":["game.fun","step.fun","functor.json"]}
```

The server writes the inline files to a scratch directory it owns and launches
them normally, so file-watch hot reload, `reload_source` and `reload_project`
all behave exactly as for a project on disk. That directory is **removed when
the session stops** (or the server shuts down): an inline game has no durable
home until `save_project` gives it one.

`save_project` deliberately does not copy the launch directory. It asks the
runtime for what it is *running* (`GET /project`, debug protocol v5), so a
session edited only over the wire saves the edited source rather than the text
it booted with.

A directory that already holds a project (a `functor.json` or any `.fun` /
`.funi`) is **refused** — nearly every project's entry is named `game.fun`, so
a matching name is no evidence that it is the same project. Pass
`overwrite: true` to replace it; that also **deletes modules the session does
not have**, since `file = module` means a leftover sibling would still load and
the saved copy would not be the program that ran. A `functor.json` is
synthesized only when the directory has none — an existing manifest is never
rewritten, and a multi-entry one is not reconstructed (`/project` reports
modules, not project metadata).

The other direction is `init_game`, which scaffolds the ordinary starter on
disk — `init_game { "dir": "./my-game" }` then
`launch_game { "dir": "./my-game" }`.

## Driving a game deterministically

```jsonc
launch_game { "dir": "examples/counter", "mode": "headless" }
// → {"session":"s1","url":"http://127.0.0.1:53127","port":53127,"owned":true,…}

pause      { "session": "s1" }                       // pin the clock where it is
send_input { "session": "s1",
             "command": {"type":"ui_event","slot":0,"kind":"Clicked"} }
step       { "session": "s1" }                       // one frame, waited for
// → {"frame":9,"tts":0.13,"pending_steps":0,…,"model":{"count":1}}
stop_game  { "session": "s1" }
```

Nothing advanced between the click and the step, and `step` returned only once
the step had actually run — so `model.count` is the click's effect, not a
sample of a still-moving game.

## Debugging a human's live session

`functor develop` serves the debug runtime on **http://127.0.0.1:8077** by default
(`docs/debug-runtime.md`), so there is no port to relay: the human runs their game,
the agent attaches to the well-known port.

```sh
functor -d examples/counter develop        # the human's window, hot-reloading on save
```

```jsonc
connect_game { "url": "http://127.0.0.1:8077" }
// → {"session":"s1","url":"http://127.0.0.1:8077","owned":false,…}
```

The session is **attached**: `get_state`, `get_scene`, `capture_frame`, `pause`,
`step`, and `send_input` all drive the human's live window, and `stop_game` merely
forgets it. Remember that pausing or injecting input affects what they are looking
at — `resume` when done.

If the human is running two develop sessions, only the first holds 8077 (the second
logs that it is running without a debug server); ask them to start the one you need
with an explicit `--debug-port`.

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

- [Driving games with agents](https://functor.games/manual/#agents) — the same story as a
  manual section, for readers who arrive from the site rather than the repo.
- [`docs/debug-runtime.md`](debug-runtime.md) — the HTTP surface these tools wrap,
  with the exact request/response shapes.
- `e2e/mcp-server.mjs` — the end-to-end proof, speaking raw JSON-RPC to the server.
- `.claude/skills/functor-lang/SKILL.md` — the language guide `language_guide`
  serves, and the file to edit when the language changes.
- [`docs/functor-lang.md`](functor-lang.md) — the language roadmap behind it.
- `tools/functor-sdk` — the typed TypeScript SDK over the same endpoints, for
  scripts rather than agents.
