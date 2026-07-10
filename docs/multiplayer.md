# Multiplayer / networking design

Status: **active** (Phase 0 in progress). This is the design doc and roadmap for
networking in Functor. The backlog stubs in `docs/todo.md` ("Async inbox",
"Keyed resource registry", `Sub.Net.*`) are the first concrete steps and are
expanded here.

## Goal

Support real multiplayer games — protocols **HTTP(S)**, **WebSockets**, **TCP/UDP
direct sockets**, and (later) **WebRTC** — across both the native and wasm
runtimes, and make every one of them **drivable and testable headlessly** the way
rendering already is (`--fixed-time`, frame capture, the debug server).

Long-term north star: a multiplayer VR battle-royale ("all-must-fall"). That
raises the bar on latency (UDP + client prediction) and scale (~100 players), but
those are a *netcode* epic built **on top of** the transport layer described here,
not part of it.

## Design constraints (from the architecture)

The MVU loop's hot-reload behavior shapes the API:

- **The effect queue is not persisted across hot reload.** Functor Lang reload preserves
  **only the model** (a plain value the host holds); the queue is reset to empty.
  An `Effect` may carry a closure — which is what lets HTTP use the Elm `expect`
  shape (the request carries a `tagger : Result -> Msg`).
- **An in-flight request's tagger cannot survive a reload.** The request→response
  tagger is held in a token-keyed registry, not in the model. On hot reload the
  model is preserved (and closures stored *inside* it rebind), but a pending
  request loses its tagger and the response is dropped with a warning (a
  deliberate, dev-only trade).
- **Subs are recomputed every frame and not persisted.** For *persistent
  connections* (WebSocket/TCP/UDP, Phase 2+), a `Sub` still carries the inbound
  decoder and the connection identity, so a live socket is matched across
  recomputations instead of being reopened every frame.

So: **one-shot request/response (HTTP) is a single `Effect` carrying its tagger,
Elm-style; persistent connections are a `Sub` (inbound/identity) + `Effect`
(send), per the WebSocket/TCP/UDP phases.**

## Architecture

```
        ┌──────────────────────── Functor Lang functional core ───────────────────────┐
        │  subscriptions: model -> Sub   (inbound + connection lifecycle)     │
        │  update/tick   -> effect       (outbound, PLAIN DATA only)          │
        └─────────────────────────────────┬───────────────────────────────────┘
                                           │  (the Functor Lang producer + prelude)
        ┌──────────────────────────────────▼──────────────── imperative shell ─┐
        │  ConnectionManager  — owns live connections, keyed by sub identity     │
        │  AsyncInbox         — thread-safe queue; drained ONCE per frame        │
        │  Transport (trait)  — TcpDirect | Udp | WebSocket | Http | WebRTC      │
        │                       + VirtualTransport (in-memory, deterministic)    │
        └───────────────────────────────────────────────────────────────────────┘
            native: tokio tasks            │   wasm: web-sys / wasm-bindgen-futures
```

- **`Transport` trait** (in `functor-runtime-common`) is the seam. The Sub/Effect
  API and `ConnectionManager` talk only to the trait; real sockets vs. an
  in-memory `VirtualTransport` are swapped underneath. This is what lets the same
  game run over real I/O *or* a simulated, deterministic network.
- **AsyncInbox + once-per-frame drain** is the determinism seam. I/O happens
  whenever on background tasks; the game only *observes* inbound messages at frame
  boundaries, when the runtime drains the inbox into the `EffectQueue` and feeds
  messages through `update`. (Same shape as the debug server's per-frame request
  drain.)
- **ConnectionManager** reconciles the declared sub set against live connections
  each frame: open newly-declared connections, tear down removed ones, keyed by a
  stable identity (endpoint / user key), not the generic msg.

## API (Functor Lang)

**HTTP — Elm `Http.get { expect = ... }` style (shipped).** A single `Effect`
carries the tagger; the response comes back as a message through `update`. No
subscription.

```functor
Effect.httpGet(url, tagger)        // tagger: (HttpResponse) => Msg
Effect.httpPost(url, body, tagger) // the response record is handed to the tagger
```

Under the hood: the request gets an auto token; running the effect registers the
tagger (keyed by token) and queues a plain-data command for the host to perform;
when the response lands, the broker applies the tagger and delivers the message.
`examples/netdemo` is the port.

**Persistent connections — `Sub` (inbound/identity) + `Effect` (send)**
(WebSockets shipped):

```functor
// client: declares a desired connection; runtime keeps it open + reconnects
Sub.connect(url, tagger)   // tagger: (Net.NetEvent) => Msg
// server: accepts many; yields per-client events (native only for TCP/UDP/WS)
Sub.listen(addr, tagger)   // tagger: (Net.NetEvent) => Msg

Effect.send(connId, text)  // send on an open connection
```

`Net` is a built-in module, always in scope:
`type NetEvent = | Connected(id: Float) | Message(id: Float, text: String) |
Disconnected(id: Float) | Error(id: Float, text: String)`. The connection id is
assigned by the runtime and reported via `Connected`; the game stores it in its
model and names it in `Effect.send`. `examples/wsdemo` (client) and
`examples/wsserverdemo` (server) are the ports.

## Test harness / SDK

Both layers, in-process first:

**A. In-process deterministic netsim (primary SDK).** A `functor-netsim` crate
holds N game instances + a `VirtualTransport` bus and steps them in controlled
lockstep. Because the harness *is* the network, tests of entity sync, high
latency, loss, reordering, partitions, and disconnect/reconnect are byte-for-byte
reproducible — fast, no sockets, no GPU.

```
sim.add_server(game); sim.add_client(game) x K
sim.set_link(client2, { latency, jitter, loss, reorder })
sim.advance_ticks(server, 1); sim.deliver()   // you control when bytes cross
sim.partition([clientA], [server]); sim.heal(...)
sim.kill(clientB); ...; sim.restart(clientB)
assert_eq!(sim.state(clientA).entities, sim.state(server).entities)
```

This shipped as the `functor-netsim` crate: many `functor_lang::Session`-backed game
instances share a `VirtualTransport` bus in one process, stepped in lockstep. (An
`functor_lang::Session` is a plain owned value, so hosting a server + many clients in one
process is natural — there is no per-process global runner to work around.)

**B. Multi-process integration harness.** Real `functor` game processes driven
over an extended debug-server API (add `/net` inject + `/tick` step to the
existing `/input`, `/time`, `/state`, `/scene`). Slower, less deterministic;
validates the real I/O + serialization path. Smoke/integration only.

## Roadmap (small, stacked PRs; each protocol ships with a netsim test)

| Phase | Scope | Targets |
| --- | --- | --- |
| **0. Spine** | `Transport` trait + `AsyncInbox` + `VirtualTransport`, Rust-only unit tests (latency/loss/reorder/partition). No game yet. | n/a |
| **1. HTTP** | `Effect` request + inbound `Sub` response (correlate by token); reqwest/hyper (native) + fetch (wasm). | wasm+native |
| **2. WebSocket** | `Sub.connect` + `Effect.send`; sub identity/reconciliation. Client first, then `Sub.listen` (server, native). | wasm+native |
| **3. Multi-instance + netsim SDK** | runner handle refactor + `functor-netsim` crate + first sync/latency/disconnect suite. | both |
| **4. TCP/UDP direct** | raw TCP + UDP `listen`/`connect` (UDP matters most for the real-time game). | native only |
| **5. WebRTC** | data channels + signaling. Deferred. | wasm+native |

## Netcode epic (Phase 6+, scoped separately)

For the battle-royale target, on top of the transport layer: server-authoritative
sim, client-side prediction + server reconciliation, snapshot/delta entity sync,
interpolation / lag compensation, area-of-interest culling for ~100-player scale.
The Phase 3 deterministic netsim is precisely the tool to test this — predicted
vs. authoritative divergence under controlled latency/loss is exactly what
`VirtualTransport` asserts on.
