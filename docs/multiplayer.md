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

Effect.send(connId, text)     // send on an open connection
Effect.sendMsg(connId, msg)   // send a plain-data VALUE; received as Net.Data(id, value)
```

`Net` is a built-in module, always in scope:
`type NetEvent = | Connected(id: Float) | Message(id: Float, text: String) |
Data(id: Float, value: NetData) | Disconnected(id: Float) |
Error(id: Float, text: String)`. The connection id is
assigned by the runtime and reported via `Connected`; the game stores it in its
model and names it in `Effect.send`. `examples/wsdemo` (client) and
`examples/wsserverdemo` (server) are the ports.

**Typed messages.** `Effect.sendMsg(connId, msg)` sends any plain-data value —
usually a variant of an ADT declared in a module BOTH ends load (a shared
sibling under a multi-entry project), so the protocol typechecks identically on
each side. The host converts the payload to the broker's serializable
`EffectValue` at the call site (a closure/host value inside is a teaching
error), frames it as a control-prefixed JSON text on the existing transport,
and the receiving end decodes it back and delivers `Net.Data(id, value)`
through the connection's tagger — the game matches `value` directly against the
shared ADT's constructors. Plain-text `Effect.send` traffic shares the
connection untouched (interop with non-Functor peers); a frame that fails to
decode (version skew, corruption) arrives as `Net.Error`. Typed sends land in
the structured effect log as data (`net.sendMsg` records), so they replay and
introspect like every other effect. `examples/mp` is the full reference — its
client and server exchange the shared `Protocol.Wire` ADT (typed `Move`s up,
typed `Snapshot`s down, full float precision) with no string codec anywhere;
the netsim fixtures (`runtime/functor-netsim/tests/fixtures/typed/`) are the
minimal ping/pong form.

Two sharp edges, by design: (1) constructors match by their **canonical tag**,
which includes the module prefix — `Protocol.Ping` sent from one end only matches
`Protocol.Ping` patterns on the other, so declare the ADT in ONE shared module
loaded identically by both roles (an entry-declared copy would tag bare `Ping`
and fall through the peer's catch-all silently). (2) Non-finite numbers
(NaN/Infinity) are refused at the `sendMsg` call site — JSON cannot carry them.
Note: adding `Data` to `NetEvent` was a check-time **breaking change** — a
pre-existing game matching `Net.NetEvent` without a catch-all needs a
`Net.Data` arm to typecheck again.

**Codec evolution (intent, not built).** The wire codec is a two-function seam
(`encode_typed_msg`/`decode_typed_msg`) over the serde-derived `EffectValue`,
and the `\u{1}fun:` prefix is a frame DISCRIMINATOR, not part of the payload —
a different tag can select a different codec per frame, so JSON and a binary
format (CBOR/postcard/…) can coexist on one connection and be adopted
incrementally. The plan when bandwidth starts to matter (the Phase 4 UDP path
and the netcode epic's snapshot deltas, not the WS lobby flows): negotiate the
codec **per connection** at the handshake — both-Functor peers may agree on a
compact binary format, anything else falls back to JSON (which also preserves
the non-Functor interop story). Games never see the codec: same values in,
same values out, and the effect log stores the structured `EffectValue`, not
wire bytes, so replay/introspection are format-independent. Deliberately NOT
planned: user-authored encoder/decoder surfaces (Elm-style) — `sendMsg` exists
to kill hand-rolled codecs; full wire control stays with the `Effect.send`
text escape hatch (and a future `Effect.sendBytes`). Two prerequisites for a
non-self-describing binary format: the protocol-hash handshake (postcard/
bincode decode drift into wrong VALUES rather than failing loud, unlike
JSON/CBOR), and a bytes-inbound path through the shells (WS binary frames;
`NetEvent` text is `String` today). Cheaper first lever for WS: compression
(permessage-deflate), which changes no formats at all.

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
