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

Two facts about the MVU loop drive the whole API shape:

1. **Effects are persisted across hot reload.** `getState`/`setState` bundle
   `(model, effectQueue)` into `OpaqueState`. Per the
   `effects-plain-data-invariant`, an `Effect` must therefore be **plain data** —
   no live sockets, no closures. Outbound network ops carry a `ConnectionId` +
   bytes, never a handle.
2. **Subs are recomputed every frame and are *not* persisted.** So a `Sub` *may*
   carry closures (decoders) and *must* carry identity, so a live socket is
   matched across recomputations instead of being torn down and reopened every
   frame (see the existing comment in `Sub.fs`).

That asymmetry decides the API: **inbound + lifecycle is a `Sub`; outbound + send
is an `Effect`.**

## Architecture

```
        ┌──────────────────────── F# functional core ────────────────────────┐
        │  subscriptions: model -> Sub   (inbound + connection lifecycle)     │
        │  update/tick   -> effect       (outbound, PLAIN DATA only)          │
        └─────────────────────────────────┬───────────────────────────────────┘
                                           │  (thin Emit shims)
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

## API (F#)

Inbound + lifecycle — `Sub` (carries decoders, owns the socket):

```fsharp
// client: declares a desired connection; runtime keeps it open + reconnects
Sub.connect (endpoint: Endpoint) (decode: NetEvent -> 'msg)
// server: accepts many; yields per-client events (native only for TCP/UDP/WS)
Sub.listen  (bind: Endpoint)     (decode: NetEvent -> 'msg)
```

`NetEvent = Connected of ConnectionId | Message of ConnectionId * byte[]
          | Disconnected of ConnectionId | Error of ConnectionId * string`

Outbound + send — `Effect` (plain data only):

```fsharp
Effect.send      (id: ConnectionId) (payload: byte[]) : effect<'msg>
Effect.broadcast (ids)              (payload: byte[]) : effect<'msg>
Effect.close     (id: ConnectionId)                   : effect<'msg>
```

`ConnectionId` is a plain value assigned by the runtime and reported via the
`Connected`/`ClientConnected` event; the game stores it in its model and names it
in send effects. HTTP request/response correlates by a plain token the game picks:
request via `Effect`, response arrives via the connection's inbound `Sub`.

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

Enabling refactor: today there's a single global `currentRunner` per dylib
(`Runtime.fs`), so one process can't host a server + many clients. Generalize the
`no_mangle` / `wasm_bindgen` exports to a **handle-based API** (`create_runner()
-> Handle`, `tick(h)`, `inject_net(h, ...)`, `get_state(h)`). This also subsumes
today's single-instance hot-reload case.

**B. Multi-process integration harness.** Real `functor-runner` processes driven
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
