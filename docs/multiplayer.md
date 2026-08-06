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
        │  transports         — TcpDirect | Udp | WebSocket | Http | WebRTC      │
        │                       + VirtualNet (in-memory, deterministic)          │
        └───────────────────────────────────────────────────────────────────────┘
            native: tokio tasks            │   wasm: web-sys / wasm-bindgen-futures
```

- **The `ConnCommand` / `NetEvent` vocabulary** (`functor-runtime-common`'s `net`
  module) is the seam. The Sub/Effect API and `ConnectionManager` speak only that
  vocabulary; real sockets, the embedder seam, and the in-memory `VirtualNet` are
  swapped underneath. This is what lets the same game run over real I/O *or* a
  simulated, deterministic network.
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
`type NetEvent = | Connected(id: float) | Message(id: float, text: string) |
Data(id: float, value: unknown) | Disconnected(id: float) |
Error(id: float, text: string)`. `Data`'s payload is `unknown` — the explicit
gradual seam, since its real type is whatever ADT the two ends share. The
connection id is
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
shared ADT's constructors. Lists, immutable Maps, tuples, records, and variants
all cross this seam structurally; Map entries retain their canonical key order.
Plain-text `Effect.send` traffic shares the
connection untouched (interop with non-Functor peers); a frame that fails to
decode (version skew, corruption) arrives as `Net.Error`. Typed sends land in
the structured effect log as data (`net.sendMsg` records), so they replay and
introspect like every other effect. `examples/orbs` is the full reference — its
`module Client` and `module Server` exchange the shared `Wire` ADT (typed
`Steer`s and `Claim`s up, typed `Snapshot`s down, full float precision) with no
string codec anywhere; `e2e/net-coordinator.mjs` drives it as a
whole hosted session.

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

**A. Hosted panes + a host coordinator (primary SDK).** A whole session runs in
one browser page as N independent runtimes — a "server" pane and K "client"
panes, each an ordinary `player.html?net=embedder` with its own model and its
own render loop. `"embedder"` routes a runtime's networking to the page that
embeds it instead of to real sockets: the pane posts its drained `ConnCommand`s
outward (`functor-net-commands`) and takes inbound events back
(`functor-net-deliver`). The **host coordinator** (`site/src/net-coordinator.ts`)
is the thing in between — a listener registry keyed by authority, one connection
id per pair, both ends told `connected`, FIFO per pane. No sockets are opened
and no server process runs.

Its routing properties mirror `functor_runtime_common::net::VirtualNet`, which
it does not call — including its scheduling: each routed packet is given a
delivery FRAME (`sent + latency + jitter`, drawn from a seeded SplitMix64 and
clamped so it can never overtake an earlier packet on the same connection) and
flushed when the session's reference clock reaches it. Every pane's link chip
sets that latency and jitter; the wire rows print `sent → delivered`.

**Latency and jitter only, and that is a decision** (design Addendum 8.2):
`Sub.connect` promises reliable, ordered delivery, so the coordinator must never
drop or reorder a packet. The chips keep their loss numbers, labelled as
applying to datagrams, and those activate with `Net.Udp`. Still to come:
partitions, and the step-time delivery barrier — the schedule is keyed to the
session's reference clock rather than to each receiving pane's own step, which
is what stands between the reproducible jitter DRAWS and reproducible runs.

`e2e/net-coordinator.mjs` (`npm run test:net-coordinator`) drives `examples/orbs`
as a server plus two clients in headless Chromium and asserts the handshake,
two-way traffic, convergence across clients, and input propagation.

An earlier in-process variant of this SDK — `functor-netsim`, N producers
stepped in lockstep over `VirtualNet` inside ONE process — was built and then
removed: it duplicated the protocol the coordinator now owns while running the
games in a shape (shared command queues, one thread, one clock) that the panes
do not. `VirtualNet` itself survives as the semantics above.

**B. Multi-process integration harness.** Real `functor` game processes driven
over an extended debug-server API (add `/net` inject + `/tick` step to the
existing `/input`, `/time`, `/state`, `/scene`). Slower, less deterministic;
validates the real I/O + serialization path. Smoke/integration only.

## Whole-environment time travel

The goal: seek the WHOLE environment to a past frame — every pane's model *and*
the packets that were genuinely in flight between them — so a rewound frame
shows each client's own lagging view beside the server's authoritative one.

It splits along a line the code already draws: each producer records and
restores its own model (`SceneRecorder` runs inside the frame body `tick`
executes, and `seek_scene_to` restores it), so the coordinator need only
snapshot what lives *outside* the producers — the network and its routing
tables.

The **snapshot cut** is load-bearing. Only delivery touches a model outside
`tick` (an inbound message folds through `update` on the spot), so the snapshot
must be taken after a frame's sends are routed and the network has advanced but
*before* delivery — the one instant where every model still equals its recorded
frame. A seek then replays that frame's pending deliveries, so a parked frame
shows exactly what a viewer saw live and seeking is idempotent.

Semantics match the single-game scrubber: a seek is non-destructive **while
parked**, and stepping on **commits the branch**. The commit is rebuilt from the
recorded frame, so stepping on from an untouched scrub reproduces the original
timeline byte-for-byte. Because a scrub-back is a plain restore and never a
re-step, none of this needs determinism — the property `History` relies on for
one game holds for N games plus the network.

Two rules to enforce loudly rather than silently skew: every instance must join
before the environment's first step (frame alignment), and a seek must preflight
every instance's recorded range before mutating anything (a producer's own
`seek_scene_to` *clamps* rather than refusing). Link impairment is configuration,
not recorded state, so it survives a restore — "rewind, worsen the link, watch it
again" works.

This is **not built on the coordinator yet**: the removed in-process harness is
where these rules were first implemented and regression-tested, and the
coordinator-seek PR re-establishes them over the panes.

## Roadmap (small, stacked PRs; each protocol ships with a test)

| Phase | Scope | Targets |
| --- | --- | --- |
| **0. Spine** | the `ConnCommand`/`NetEvent` vocabulary + `AsyncInbox` + `VirtualNet`, Rust-only unit tests (latency/loss/reorder/partition). No game yet. | n/a |
| **1. HTTP** | `Effect` request + inbound `Sub` response (correlate by token); reqwest/hyper (native) + fetch (wasm). | wasm+native |
| **2. WebSocket** | `Sub.connect` + `Effect.send`; sub identity/reconciliation. Client first, then `Sub.listen` (server, native). | wasm+native |
| **3. Multi-instance SDK** | runner handle refactor + the embedder seam + the host coordinator + first sync/latency/disconnect suite. | both |
| **4. TCP/UDP direct** | raw TCP + UDP `listen`/`connect` (UDP matters most for the real-time game). | native only |
| **5. WebRTC** | data channels + signaling. Deferred. | wasm+native |

## Netcode epic (Phase 6+, scoped separately)

For the battle-royale target, on top of the transport layer: server-authoritative
sim, client-side prediction + server reconciliation, snapshot/delta entity sync,
interpolation / lag compensation, area-of-interest culling for ~100-player scale.
The Phase 3 multi-instance SDK is precisely the tool to test this — predicted
vs. authoritative divergence under controlled latency/loss is exactly what a
`LinkProfile`-impaired coordinator session asserts on.
