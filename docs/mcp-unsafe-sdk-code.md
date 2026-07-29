# Unsafe SDK code over MCP — Node child architecture

## Question

Can Functor give an MCP client Playwright-like “run SDK code” ergonomics, so an
agent can use ordinary variables, loops, callbacks, assertions, and bounded
polling instead of spelling every debug-runtime request as a separate tool
call?

Yes. `run_game_code_unsafe` invokes one submitted JavaScript function in a
Node.js child process:

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

This is the recommended composition surface for trusted local coding agents.
The individual MCP tools and debug-runtime HTTP endpoints remain the underlying
primitives and escape hatches, not the workflow an agent should normally have
to orchestrate request by request.

The MCP request is still JSON. The program is simply its `code` string:

```json
{
  "session": "s1",
  "code": "async (game) => { ... }",
  "timeout_ms": 120000,
  "include_final_state": false
}
```

`include_final_state` defaults to `true` for compatibility. Set it to `false`
when the program already selects its evidence into `return_value`. The parent
still takes the final `/state` snapshot after the Node child exits, but discards
the structured model and `model_debug` after producing a small
`final_state_summary`. This keeps large models from dominating an otherwise
compact automation result.

## Architecture

```text
MCP client
  │ run_game_code_unsafe { session, code }
  ▼
Rust MCP parent ──spawns──► Node.js child (arbitrary JavaScript)
  ▲                              │
  │ newline-delimited RPC        │ game.state(), game.step(), …
  └──────────────────────────────┘
  │
  ├── exact-URL operation gate
  ├── debug-protocol version checks
  ├── bounded HTTP responses and captures
  ├── waited/cancellable clock advances
  ├── input snapshot + failure restoration
  └── structured SDK-call trace
```

The child never receives a privileged in-process Rust object. Its injected
`game` methods send newline-delimited RPC calls to the parent over reserved
stdout. Ordinary `process.stdout.write` is captured as a log rather than
allowed to corrupt that channel. The parent performs the existing debug-runtime
HTTP operations, preserving one authority for lifecycle, cancellation, byte
limits, and quarantine.

This is deliberately different from the earlier restricted-plan prototype.
There is no custom TypeScript-shaped parser, duplicated Rust/TypeScript plan
schema, canonical-source round trip, or serialized predicate language.
`stepUntil` takes an ordinary JavaScript callback because the program itself is
the serialization boundary.

## Result and validation trace

The code's return value must be JSON-serializable. The tool also returns what
actually happened:

```json
{
  "ok": true,
  "unsafe": {
    "rce_equivalent": true,
    "security_sandbox": false,
    "execution": "local_node_child_process"
  },
  "node_version": "v22.20.0",
  "calls_executed": 6,
  "return_value": {
    "frame": 42,
    "enemies": 1
  },
  "trace": [
    {
      "seq": 1,
      "method": "pause",
      "args": [],
      "ok": true,
      "elapsed_ms": 3,
      "result": null
    }
  ],
  "logs": [
    {
      "level": "log",
      "text": "spawned 1"
    }
  ],
  "captures": [],
  "final_state": null,
  "final_state_summary": {
    "frame": 42,
    "tts": 0.7,
    "pending_steps": 0,
    "held_keys": [],
    "held_mouse_buttons": [],
    "model_json_bytes": 12345
  }
}
```

The trace is an execution record, not a static plan. It naturally represents
branches and polling because it records only SDK RPC that the parent received.
Large results such as state and scene data are summarized in trace entries; the
submitted program can select the important fields into `return_value`. Every
`game.capture()` call adds metadata to the text result and a PNG MCP image
block. Because submitted code is RCE-equivalent, it can deliberately bypass
the stdout guard and spoof protocol messages. The trace is useful observability
for trusted automation, not a security attestation of hostile code.

With the default `include_final_state: true`, `final_state` remains the complete
fresh `/state` object and `final_state_summary` is absent. With `false`,
`final_state` is `null`; the summary reports frame, game time, pending clock
steps, held keyboard/mouse-button levels, and the encoded byte size of
`model`. It includes `paused` only when an attached runtime actually supplies
that field; it never guesses clock state from `pending_steps`. Neither `model`
nor `model_debug` is retained in compact mode.

## Injected SDK

The child-side object mirrors the standalone `@functor/sdk` `FunctorClient`
method surface within the runner's byte and call bounds:

- observe: `state`, `scene`, `trace`, `capture`, `heldKeys`, `isKeyDown`,
  `xrInput`;
- deterministic clock: `pause`, `step`, `stepFrames`, `stepUntil`, `resume`;
- input: `input`, `key`, `keyDown`, `keyUp`, `pressKey`, `mouseMove`,
  `mouseWheel`, `mouseButton`, `mouseDown`, `mouseUp`, `xr`, `xrClear`,
  `uiClick`;
- lifecycle/data: `rewind`, `reloadSource`, `reloadProject`, `loadProject`,
  `reloadAsset`, `reloadAssets`;
- asynchronous observation: `waitForState`.

`stepUntil(predicate, options)` checks current state first, then advances and
observes one fixed frame at a time until the callback returns true. It requires
a bounded `maxFrames` (default 600, maximum 10,000), uses a fixed `dts`
(default 1/60 second), and throws a teaching error when exhausted.

Standalone TypeScript exposes the same `stepUntil`, `pressKey`, and `uiClick`
helpers. Its binary HTTP transport can upload larger assets; the submitted-code
JSON protocol caps one decoded `reloadAsset` payload at 8 MiB. Submitted MCP
code is JavaScript accepted by Node; agents can author it with TypeScript SDK
types, but the string itself must not contain TypeScript-only syntax such as
type annotations.

## Trust boundary

The `unsafe` suffix is part of the contract:

- submitted code can import Node modules, read/write files, inspect environment
  variables, access the network, and start processes;
- the Node child has the same operating-system authority as `functor mcp`;
- the parent can contain a Node crash and kill the direct child on timeout, but
  it is not a security or process-tree sandbox;
- subprocesses deliberately started by submitted code can outlive the direct
  Node child and its timeout;
- this is suitable for local stdio use where the MCP client is already trusted
  with comparable developer-machine authority;
- do not expose it to untrusted network clients.

Node.js 20 or newer must be installed. If `node` is absent, the tool returns an
installation error before submitted code or game mutation runs. `FUNCTOR_NODE`
can name a non-default executable.

A hosted or multi-tenant version needs a real container/VM boundary, separate
filesystem/process/network namespaces, resource quotas, and a narrow
credential-free proxy back to the game runtime. Node's `vm` API and the local
child-process boundary are not substitutes.

## Bounds and failure behavior

- source: 64 KiB;
- SDK calls: 25,000;
- active submitted-code deadline, including initial/final snapshots:
  120 seconds maximum;
- direct Node-child shutdown confirmation: 2 seconds;
- aggregate failed-run safety cleanup: 30 seconds, then quarantine;
- one child protocol message: 16 MiB;
- retained console text: 64 KiB;
- returned JSON text: 4 MiB;
- one runtime text response or PNG: 8 MiB;
- one decoded asset sent through the child JSON protocol: 8 MiB;
- all raw captures: 16 MiB;
- all base64 MCP image content: 24 MiB.

The exact-URL mutation gate is held for the whole function. Other mutations on
that runtime wait without interleaving; different runtimes continue
independently; state/scene/trace reads remain available for observation.

On syntax failure, the function never starts. Once it starts, execution is not
transactional: model, physics, UI, and effects from landed steps are not rolled
back. If code throws, times out, is cancelled, or is interrupted by
`stop_game`, the parent kills the direct Node child, cancels any accepted
unlanded clock work, snapshots current input, and transitions only SDK-touched
key/mouse-button levels that differ from their pre-run snapshot. Child shutdown
may add at most 2 seconds and the whole safety cleanup at most 30 seconds beyond
the active deadline. Processes deliberately spawned by submitted code are not
tracked or killed.

If direct-child termination, queued-step cancellation, or input restoration
cannot be confirmed, the shared exact URL is quarantined. Every alias rejects
further mutations. Stopping an owned session clears the tombstone only after
confirming runtime termination; an attached runtime requires restarting both
the runtime and MCP server.

## Game-jam fit

The July 2026 game jam exposed one universal workflow gap: participants fell
back to repository-only launch/input/time/state/capture scripts. The
Playwright-style runner keeps the improvements demonstrated by the first plan
prototype while removing its expression ceiling:

1. one MCP call owns the complete gameplay proof;
2. SDK methods replace raw HTTP payloads and pending-step polling;
3. callbacks express state-dependent progress such as spawn/load/phase waits;
4. local variables and loops keep repeated actions compact;
5. return values, logs, call traces, and captures keep evidence with the run;
6. the same concepts work in standalone TypeScript.

The Node-child iteration was rerun against the four jam branches, with no raw
input/time/state HTTP choreography:

| Entry | Calls | Evidence returned by one code run |
| --- | ---: | --- |
| Photo vignette | 4 | two mouse samples produced `yawOffset = -0.6` and `pitchOffset = 0.3` |
| Swarm Survival | 13 | selected 144 enemies, moved the player from `x = 0` to `x = 0.0867`, then observed `moveX = 0` after release |
| Marble Golf | 19 | waited for physics readiness, changed aim/power, observed a moving first stroke, then reset to `(-2.4, -8)` |
| Tower Defense | 23 | placed one 25-credit tower, started wave 1, then used `stepUntil` to observe the first spawned enemy |

These are concrete execution traces rather than canonical plan round trips.
The most useful change was not fewer primitive calls inside the parent; it was
that one agent-facing call could express and return the entire state-dependent
proof with local variables, assertions, and callbacks.

This tool improves the agent iteration loop. It does not fix engine/API gaps
found by the games themselves: picking/orientation, colliders, procedural
terrain collision, lines, instancing, richer UI, or phase telemetry.
