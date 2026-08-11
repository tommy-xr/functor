# `functor mcp` — MCP over the debug runtime

`functor mcp` is an [MCP](https://modelcontextprotocol.io) server: it exposes the
[debug runtime](debug-runtime.md) — launch a game, read its model, pause it,
inject input, step the clock, capture a frame — as standard tools, over stdio.

The capability is not new; the *interface* is. A trusted local MCP agent can
launch or attach to a game, then use `run_game_code_unsafe` to observe, drive,
wait, assert, and return evidence in one ordinary JavaScript function. The
individual tools expose the same lower-level operations for one-off use and
clients without Node.js.

It is a plain HTTP client of the runtimes and lives in the CLI. Its runtime-side
additions are `GET /project` (the read half used by `save_project`) plus the
protocol-v8 `cancel` command that lets bounded stepping or submitted SDK code
abort without leaving queued steps behind. The code runner snapshots input and
restores only the key/button levels touched by an unsuccessful run.

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

For trusted local coding-agent automation, use `launch_game` or `connect_game`
once and make `run_game_code_unsafe` the default composition surface. One code
call can observe, drive, wait, assert, and return evidence without a long
sequence of MCP round trips. The individual observation and driving tools
remain useful for one-off operations, clients without Node.js, and debugging the
lower-level protocol. Do not expose the unsafe code runner to untrusted callers.

**Sessions.** The server manages N concurrent games at once. A session is a base
URL plus, when the server launched it, the child process. State-changing calls
on one session share an async operation gate: an overlapping call waits for the
active call to finish, then runs without interleaving. This is mutual exclusion,
not a promised FIFO sequencer; relative waiter order is unspecified. Cancellation
while queued stops waiting and prevents the mutation. Once a mutation acquires
the gate, it runs to its operation boundary even if its MCP response is
cancelled. Acquired `step` and submitted-code calls have a 120-second active
deadline that progress does not extend; it is checked between operations and
step polls, while an in-flight request retains its 30-second request timeout.
Direct Node-child shutdown has a separate 2-second bound, and failed-code safety
cleanup has one aggregate 30-second bound, so confirmation may extend the tool
response beyond the active deadline but cannot hold the gate indefinitely. An
owned stop also ends an active step/code run at its next polling boundary before
taking the gate. Before an aborted operation releases the gate, it cancels the
accepted step queue; a code failure snapshots current input and transitions only
SDK-touched key and mouse-button levels that differ from their pre-run baseline.
If either cleanup cannot be confirmed before its bound, the exact URL is
quarantined: every alias rejects further mutations. Stopping an owned session
waits at most 5 seconds to confirm process termination and clears its tombstone
only on success. An unconfirmed termination removes the unusable closing
session records but preserves the URL tombstone. Merely detaching an attached
id cannot prove cleanup; restart both that runtime and this MCP server before
reconnecting. `list_sessions` exposes
session flags plus quarantined URL tombstones with no remaining id; read-only
state/scene/trace calls remain available for diagnosis.
Exact normalized base-URL aliases created by this MCP server share the same
gate, while different exact URLs continue concurrently. `connect_game` reserves
that gate before discovery, so it cannot race stop and insert a dead alias
afterward. Normalization currently removes trailing `/` only, so hostname aliases
such as `localhost` versus `127.0.0.1` are not recognized, and direct HTTP
clients outside this MCP process are not coordinated. Read-only
`get_state`/`get_scene`/`get_trace` calls do not take the gate.

`stop_game` marks closing before it waits for the gate and completes cleanup
after that mark even if its MCP response is cancelled. Stopping an attached
session closes/removes only that id and leaves other aliases valid. Stopping an
owned session closes the owner, every exact-URL alias, and any pending connect;
keeps those closing records through child kill/wait; then removes the group.
New and already-queued mutations on a closing id reject without runtime I/O.

| Tool | What it does |
| --- | --- |
| `launch_game` | Spawn a game as a child on a free port and return its session id. The project comes from `dir` **or** from `files` — the whole project inline (see below). `entry` names the ROLE, for a project whose `functor.json` declares `entries` (`"server"`, `"client"`; default `client`, or the sole entry) — one session per role. `mode` is `hidden` (default) or `headless`. The response carries `protocol_version`; pass `discovery: true` for the full endpoint index. |
| `connect_game` | Attach to a runtime this server does **not** own (a human's `functor develop` on port 8077, someone else's `--debug-port`, or an adb-forwarded Quest). |
| `list_sessions` | Every session: id, url, owned/attached, and whether it currently answers. |
| `stop_game` | Kill a launched game and close its exact-URL aliases/pending connects; merely forget one attached id. |

**Observing.**

| Tool | What it does |
| --- | --- |
| `get_state` | `frame`, `tts`, `pending_steps`, `model_revision`, `pending_net`, viewports, sampled `input`, and the model — the structured JSON view (the default read; `model_debug` is the Debug text). |
| `get_scene` | The camera, scene graph, and lights `draw` produced. Pure data, so it works headlessly. |
| `get_trace` | The paused inspector trace: every entry point's binder and variable values for the last real frame. Pause first. |
| `capture_frame` | A PNG of the next rendered frame, returned as an MCP image block. |
| `wire_log` | Every packet the coordinator routed for a session group, as data: `{seq, frame, at_ms, from, to, conn, kind, size, payload_text}`, with `since` / `limit` / `link` / `direction` filters. Group sessions only. |

**Driving.**

| Tool | What it does |
| --- | --- |
| `pause` | Pin the clock (defaults to the current `tts`), so nothing advances on its own. |
| `step` | Run 1–10,000 `frames` of finite positive `dts` each, **wait for them to land**, and return the fresh state. |
| `step_all` | Step several sessions one round, strictly sequentially, in the order given — the multiplayer lockstep primitive. See [Running a multiplayer session](#running-a-multiplayer-session). |
| `launch_session_group` | Launch every role of a multi-entry project at once, wired to each other by **this process** — no sockets. See [The coordinator: this process is the network](#the-coordinator-this-process-is-the-network). |
| `resume` | Follow wall-clock time again. |
| `send_input` | Inject one `POST /input` command verbatim — key, mouse move/wheel/button, `ui_event`, or an `xr` sample. |
| `rewind` | Restore model + physics to a recorded frame (it pins the clock first, as `/rewind` requires). |
| `reload_source` / `reload_project` | Hot-reload the entry, or every sibling module, with the live model preserved. |
| `run_game_code_unsafe` | Run a JavaScript function against an injected game SDK in a local Node child, returning its JSON value, SDK-call trace, logs, captures, and final state. This is deliberately RCE-equivalent; see below. |

**Authoring.**

| Tool | What it does |
| --- | --- |
| `init_game` | Scaffold a starter project on disk — the same `functor.json` + `game.fun` `functor init` writes. `template` is `"3d"` (default), `"fps"`, or `"multiplayer"`. Never overwrites; its `dir` goes straight into `launch_game`. |
| `save_project` | Write a session's **current** source to a directory, with the `functor.json` it booted with — so a multi-entry (multiplayer) project keeps its roles. The sources come from the RUNTIME (`GET /project`), so they include every wire-only edit. Refuses a directory that already holds a project unless `overwrite`. |

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
- **Order is the semantics for a networked group.** `step_all` steps sessions
  sequentially in the order given — producer → authority → observer — because
  stepping them concurrently makes packet arrival a race. See
  [Running a multiplayer session](#running-a-multiplayer-session).
- **Input is level state.** A key, a held mouse button, and an injected XR sample
  stay in force across steps until released or replaced. That is how a paused
  session is scripted: press, step a few frames, release.

Runtime errors come back as tool errors carrying the runtime's own message — a
`/input` 400 explaining a misspelled field, a `/reload-source` 400 with the
rendered load error, a `/time` 409 naming a `--fixed-time` pin.

`launch_game` and `connect_game` both require **debug protocol v8 or newer**
(`docs/debug-runtime.md`), and say so if the runtime is older. Earlier runtimes
lack at least one guarantee the tools rely on: waited batched steps, structured
model state, running-project reads, or safe queued-step cleanup. This
matters most for a device APK, which versions independently of the CLI — rebuild
it from the same Functor version.

Launched games are killed when the server stops, whether its client closes
stdin or signals it (SIGTERM/Ctrl-C). Attached ones are always left running.

## Unsafe SDK code in a Node child

`run_game_code_unsafe` follows the same model as Playwright's code-running
tools: submit one ordinary JavaScript function, and the server injects a
session-bound `game` object:

```js
async (game) => {
  await game.pause();
  await game.pressKey("3");

  const settled = await game.stepUntil(
    (state) => state.model.enemies.length > 0,
    {
      maxFrames: 120,
      dts: 0.016,
      description: "the first enemy to spawn",
    },
  );

  console.log("spawned", settled.model.enemies.length);
  return {
    frame: settled.frame,
    enemies: settled.model.enemies.length,
  };
}
```

The request puts that function in the `code` string and may set `timeout_ms` up
to 120,000. `include_final_state` defaults to `true`; set it to `false` when
the function already selects its evidence into `return_value`. The parent still
takes the final `/state` snapshot, but returns only a
`final_state_summary`—frame/time, pending steps, held input, and model JSON byte
size—instead of retaining the structured model and `model_debug`. The string is
JavaScript evaluated by Node, not a TypeScript source file, so omit type
annotations. Node.js 20 or newer must be installed; `FUNCTOR_NODE` can point to
a non-default executable. If Node is missing or too old, the tool fails before
running submitted code or mutating the game.

The injected object mirrors the standalone `@functor/sdk` method surface within
the runner's documented bounds: observation, clock, input, project reload,
asset reload, rewind, and capture methods are available. Its
`stepUntil(predicate, options)` helper checks current state, then advances one
fixed frame at a time until an ordinary sync or async callback returns true. It
defaults to 600 frames, caps `maxFrames` at 10,000, and throws a teaching error
on exhaustion. `waitForState` polls a running game without advancing it. The
child JSON protocol caps one decoded asset at 8 MiB; standalone SDK binary
uploads may be larger.

The child sends each SDK call to the Rust parent over reserved line-delimited
stdout; ordinary stdout writes become captured logs. The parent performs the
existing typed debug-runtime requests while holding the session's operation
gate. The tool returns the function's
JSON-serializable value, captured console logs, a structured trace of every SDK
call that actually ran, capture metadata plus PNG image blocks, and either the
fresh final state or its compact summary. For trusted automation, the trace is
useful validation evidence: loops, branches, and dynamic polling appear as
their concrete calls without inventing a second serialized plan language. It is
not an attestation against hostile submitted code, which already has
RCE-equivalent authority and can deliberately spoof child protocol output.

The `unsafe` suffix is literal. Submitted code is arbitrary local Node code:
it can import modules, access files and environment variables, use the network,
and start processes with the same operating-system authority as
`functor mcp`. The child-process boundary contains a Node crash and lets the
parent kill that direct child on timeout, cancellation, or `stop_game`; **it is
not a security or process-tree sandbox**. A subprocess deliberately started by
submitted code can outlive the Node child. Use this only with MCP clients
already trusted with equivalent developer-machine access. Never expose it to
untrusted network clients. A hosted or multi-tenant service needs a real
container/VM boundary and a narrow, credential-free game proxy.

Execution is ordered but not transactional. Syntax failure happens before code
starts, but a later throw cannot roll back model, physics, UI, or effects from
steps that already landed. On throw, timeout, MCP cancellation, or stop, the
parent kills the direct child, cancels accepted clock work, and restores only
key/mouse-button levels touched through the injected SDK that differ from their
baseline. Direct-child shutdown is bounded to 2 seconds and the whole safety
cleanup to 30 seconds; failed or expired cleanup quarantines the exact runtime
URL under the same rules as a failed `step`.

The complete architecture, limits, threat boundary, and game-jam evaluation are
in [the code-runner note](mcp-unsafe-sdk-code.md).

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
the saved copy would not be the program that ran.

The `functor.json` is written only when the directory has none — an existing
manifest is never rewritten — and it is the one the session **booted with**,
verbatim. `/project` reports modules, not project metadata, so a manifest
cannot be reconstructed from it: a multi-entry (multiplayer) project would come
back as `{"entry": "game.fun"}`, a directory that builds green and then runs the
wrong role. A remembered manifest that no longer describes the file set (a
module the session dropped) is REFUSED rather than adjusted. An ATTACHED
session is the one case with no answer — nothing on the wire reports a
runtime's manifest — so it still gets a synthesized single-entry one.

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

## Running a multiplayer session

A multi-entry project (`functor.json` with `entries`, like `examples/orbs`) is
one directory holding several ROLES — often, as in orbs, several inline
`module` blocks of one file. Each role is its own runtime, so it is its own
session: launch one per role and name it with `entry`, since the path cannot
say which role a shared file is being run as.

```jsonc
launch_game { "dir": "examples/orbs", "entry": "server", "mode": "headless" }  // → s1
launch_game { "dir": "examples/orbs", "entry": "client", "mode": "headless" }  // → s2
launch_game { "dir": "examples/orbs", "entry": "client", "mode": "headless" }  // → s3

pause    { "session": "s2" }                     // pin every role's clock
pause    { "session": "s1" }
pause    { "session": "s3" }
step_all { "sessions": ["s2", "s1", "s3"] }      // one lockstep round
```

**The ordering law: step producer → authority → observer.** `step_all` steps the
sessions strictly sequentially in the order given — each session's steps land
fully (`pending_steps` back to 0) before the next session starts, and each
session's already-received network events are drained (`pending_net` back to 0)
before *it* steps — and returns every session's post-step summary in that order.
The order is the semantics, not a convenience: a client's input has to reach the
authority *before* the authority steps, and the authority's broadcast has to
exist *before* an observer steps, or every round lands its work an arbitrary
number of rounds late. **Concurrent stepping is not reproducible**: whether a
packet arrives before or after its receiver's step is a race, so two identical
runs disagree. Sessions are resolved and de-duplicated — by runtime URL, so two
ids aliasing one game are refused — before anything steps, so a typo cannot
leave the group half-advanced; a failure *partway through* means earlier
sessions already advanced, with no rollback. The whole call shares one
120-second deadline.

**What it does not promise.** `step_all` orders the *stepping* and drains what
has already arrived. It is **not a transport barrier**: a packet still on the
wire between two processes is invisible to every process (which is exactly what
`pending_net` says it cannot see), so nothing here can force delivery to happen
within a given round. A game whose correctness depends on that needs its own
convergence check between rounds — poll `get_state` for the game-level fact
("the authority has seated both clients", "the observer's world matches"), the
same way you would wait for any distributed condition.

**Pause freezes the CLOCK, not the network.** A paused role keeps accepting
inbound messages and folding them through `update`, so its model changes while
`frame` and `tts` stand still. Two consequences for a driver:

- `frame` is **not** a version label for a networked model. `model_revision`
  is: it counts every replacement of the model **by game logic** — a tick, an
  injected input, an effect fold, a network delivery — so comparing it against
  an earlier read answers "did anything land" even with the clock stopped.
  (Operations that replace the model *from outside* the game — a hot reload, a
  `load_project`, a `rewind` — deliberately do not count. Those are things the
  driver itself just did, and each returns fresh state to re-baseline from; a
  counter that also moved for them would not distinguish "the network changed
  my model" from "I rewound it".)
- Before snapshotting a baseline, wait for **`pending_net` to reach 0** on
  every role. That is the shell's inbound queue depth; zero means nothing
  already received is still unprocessed. (It cannot see a packet still on the
  wire, so it is a lower bound — pair it with a game-level condition, e.g.
  "the server seated both clients".)

Both fields need debug protocol v10 (`docs/debug-runtime.md`).

**Hot-reload keeps connections.** Reloading the authority with `reload_source`
or `reload_project` preserves the live model *and* the live sockets — connections
are owned by the shell, which does not reload — so clients stay joined across an
edit to the server's rules.

Launch the authority first and let its listener bind before a client dials.
`Sub.connect` is a desired connection: a failed attempt reports `Net.Error` and
retries forever on the deterministic game-time backoff documented by the
prelude, so a boot race recovers rather than leaving the client dead.

`e2e/mcp-step-all.mjs` is the end-to-end proof: it runs the three roles of
`examples/orbs` twice from scratch and asserts the ordered lockstep produces
the identical world trace both times.

## The coordinator: this process is the network

Everything above runs the roles over **real localhost sockets**, which is why
`step_all` cannot promise anything about delivery: the packets are the kernel's.
`launch_session_group` runs the same project the other way round — every role
starts on the **embedder transport** (`--net-transport embedder`,
`docs/debug-runtime.md`), so **no runtime opens a socket at all**. Each one's
`Sub.listen` / `Sub.connect` / `Effect.send` traffic is drained by this server,
routed, and delivered back. The MCP host *is* the network.

```jsonc
launch_session_group { "dir": "examples/orbs",
                       "roles": ["server", "client", "client"],
                       "mode": "headless" }
// → { "group": "g1",
//     "sessions": [ {"session":"s1","label":"server","role":"server", …},
//                   {"session":"s2","label":"client1","role":"client", …},
//                   {"session":"s3","label":"client2","role":"client", …} ],
//     "step_order": ["s1","s2","s3"] }

pause    { "session": "s2" }                          // pin every role
pause    { "session": "s1" }
pause    { "session": "s3" }
step_all { "sessions": ["s2", "s1", "s3"] }           // one lockstep round
wire_log { "group": "g1", "since": 41 }               // …and what crossed in it
```

`roles` is the launch order; repeats are how a group gets two clients. Omit it
for one session per declared entry, with a role named `server` first. Choose the
later `step_all` order by the producer → authority → observer law rather than by
blindly copying launch order. Labels (`server`, `client1`, `client2`) are the
routing identities the wire log reads by.

**Routing (what the coordinator does).** It mirrors the browser coordinator
(`site/src/net-coordinator.ts`) and, through it, `VirtualNet`: a listener
registry keyed by **authority** (`host:port` — so a client's
`ws://127.0.0.1:9101/orbs` matches a server's `127.0.0.1:9101` bind), **one
connection id per pair** shared by both ends, both ends told `connected` under
their own routing key, and **FIFO per session**. A client that dials before its
authority exists waits and connects the moment the listener appears (15 s
grace). A client whose peer is REMOVED from the group (its session stopped) is
re-queued, because a runtime never re-dials on its own; a plain hot RELOAD of
the authority is not that case — the model, and with it the connection ids,
survives, so the group keeps routing straight across it.

**Cadence.** Routing runs on this server's own loop, in two places:

- **continuously in the background** — a round is scheduled every 8 ms, though
  its real period is the round trip to each member (a headless runtime services
  the debug channel once per ~16 ms loop, so a three-role group settles nearer
  50–100 ms) — so a group running live on wall-clock time still moves its
  packets between tool calls;
- **at every session boundary inside a `step_all` round**, with the background
  pump held off for the whole round. So a packet client 1 sends in round *N*
  is delivered — and folded through `update`, since `POST /net/deliver` answers
  only after the fold — before the authority steps.

That second half is what `step_all` could not do over sockets. It is still not
a *barrier*: the group runs freely between rounds, and nothing schedules a
packet into a named step.

**Delivery in v1 is IMMEDIATE and in order** — a perfect, reliable link: the
router never drops or reorders a packet, and a delivery that fails in transit
is re-queued at the front of its queue and retried. The one honest gap is that
`GET /net/outbound` is take-and-consume, so a drain whose RESPONSE is lost
takes those commands with it, and a delivery whose outcome is UNKNOWN (a
timeout) is reported and not retried rather than risked twice — on a reliable
ordered channel, a duplicate is worse than a gap. A session that stops
consuming has its queue bounded and the shed count reported. All of these land
on stderr, which is where this server logs; none of them are silent. Latency, jitter, and step-time scheduling ("departs step
610, deliver at 618") arrive with barrier stepping, which is also what will
give `wire_log`'s `frame` a value; today it is always `null` rather than a
measurement dressed up as a schedule.

**Reading the wire.** `wire_log` returns rows oldest-first:

```jsonc
{ "seq": 42, "frame": null, "at_ms": 8123.44,
  "from": "client1", "to": "server", "conn": 1,
  "kind": "message", "size": 118,
  "payload_text": "\u0001fun:{\"Variant\":[\"Steer\",[…]]}" }
```

`payload_text` is the message **exactly as the receiving runtime sees it**, and
is deliberately **not** decoded server-side: the framing belongs to the game
(`Effect.sendMsg` writes U+0001 + `fun:` + the payload as JSON; plain
`Effect.send` text is unframed), so the reader decodes it. `seq` is the stable
cursor — read the last row's `seq`, run a round, pass it back as `since`, and
you have exactly that round's traffic. `link` narrows to one label
(`"client1"`) or an unordered pair (`"client1:server"`), and `direction`
(`"sent"` / `"received"`) narrows a single label further. A label the group does
not have is an error, not an empty result. Rows share a payload byte budget
spent newest-first, so a chatty protocol's oldest rows come back with
`payload_elided: true` (and `payloads_elided` counted) rather than flooding the
answer — narrow the window or the `link` to read them.

**What reproduces, and what does not.** With immediate in-order delivery and
`step_all`'s boundary pumping, a steady-state round routes the same packets, in
the same direction, every time. Absolute frame numbers do not: a group runs on
wall-clock time between rounds, so how many frames elapse before you pause is
not controlled. Whole-environment reproducibility waits on barrier stepping.

Groups are launched from a **directory** (`dir`), because the roles come from
that project's `functor.json` `entries`; use `launch_game` for an inline
`files` project. Stopping a group's last session retires its coordinator, and
`wire_log` then reports the group as unknown.

`e2e/mcp-session-group.mjs` is the proof: orbs as a group, clients converging
with **nothing ever listening on `127.0.0.1:9101`**, `wire_log` rows that decode
agent-side, `step_all` + `wire_log` composing per round, and a mid-group reload
of the authority.

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
- `e2e/mcp-step-all.mjs` — the multiplayer proof over real sockets: three roles
  of `examples/orbs`, stepped in order, run twice, asserted identical.
- `e2e/mcp-session-group.mjs` — the coordinator proof: the same roles wired to
  each other by the MCP process, with zero sockets, and their wire read back.
- `.claude/skills/functor-lang/SKILL.md` — the language guide `language_guide`
  serves, and the file to edit when the language changes.
- [`docs/functor-lang.md`](functor-lang.md) — the language roadmap behind it.
- `tools/functor-sdk` — the typed TypeScript SDK over the same endpoints, for
  scripts rather than agents.
