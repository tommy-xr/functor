// The HOST-PAGE net coordinator: it routes game packets between player iframes,
// so a "server" pane and N "client" panes can talk to each other entirely
// inside one browser page — no sockets, no server process.
//
// Each pane boots `player.html?net=embedder`, which routes the runtime's
// networking to its embedder (runtime/functor-runtime-web/src/lib.rs): the pane
// posts its drained `ConnCommand`s outward as
//
//     { type: "functor-net-commands", commands: [ …ConnCommand… ] }
//
// and takes inbound events back as
//
//     { type: "functor-net-deliver", events: [ …DeliveredEvent… ] }
//
// This module is the thing in between. Its ROUTING model mirrors
// `functor_runtime_common::net`'s `VirtualNet` — the same properties, written
// again in TypeScript rather than called into:
//
//   • a listener registry keyed by AUTHORITY (host:port, scheme and path
//     ignored), so a client url "ws://127.0.0.1:9001/play" matches a server
//     bind "127.0.0.1:9001" — `authorityOf` here;
//   • one connection id per PAIR, shared by both ends (a `VirtualNet`
//     `ConnectionId` is used the same way), so a send from either end routes to
//     the other by looking up the same row;
//   • both ends are told `connected` — the client under its connect key, the
//     accepting server under its LISTEN key — because that key is how each
//     side's runtime routes the event back to the right `Sub.connect` /
//     `Sub.listen` tagger;
//   • FIFO per pane: deliveries queue in arrival order and flush as one batch.
//
// IMPAIRMENT (design Addendum 8.2) — LATENCY AND JITTER ONLY, and that is a
// semantic decision, not a missing feature: `Sub.connect` promises RELIABLE,
// ORDERED delivery, so a coordinator that dropped or reordered its packets
// would be lying about the API the game is written against. Loss and reorder
// belong to unreliable channels and arrive with `Net.Udp` (Addendum 6a); the
// link chips keep showing their loss numbers, labelled as applying to
// datagrams.
//
// So a routed packet is no longer delivered on the host's next rAF. It is
// SCHEDULED — `sentFrame + latency ± jitter`, in reference-clock frames — and
// flushed when the reference clock reaches that frame, exactly the way
// `VirtualNet::send` schedules a `deliver_tick` (see
// `runtime/functor-runtime-common/src/net/virtual_net.rs`, mirrored here rather
// than called into). Its FIFO discipline comes with it: a later-sent packet
// never overtakes an earlier one on the same connection, whatever the jitter
// draw says, because its delivery is clamped to the previous packet's.
//
// The step barrier is still a later PR: the schedule is keyed to the SESSION's
// reference clock, not to the receiving pane's own fixed step, so a delivery
// lands on the host frame that reaches it rather than inside the receiver's
// step. That is what stands between this and cross-run determinism — the jitter
// DRAWS are already reproducible (a seeded SplitMix64 per session), but which
// frame a pane happened to send in is still wall-clock.
//
// The packet log is the record every traffic view reads. Each row is keyed to
// the REFERENCE CLOCK (`Packet.frame`) so traffic lives on the same axis the
// chrono rail draws — scrubbing back replays the window's packets instead of
// only ever showing the live present.

/** The commands a pane's runtime emits (`functor_runtime_common::net::ConnCommand`,
 * serialized by serde's default externally-tagged representation). */
type ConnCommand =
  | { Connect: { key: string; url: string } }
  | { Listen: { key: string; addr: string } }
  | { Send: { conn: number; payload: number[] } }
  | { CloseConn: { conn: number } }
  | { CloseKey: { key: string } };

/** One inbound event handed back to a pane
 * (`functor_runtime_common::net::DeliveredEvent`). The wire is TEXT here: a
 * `Send`'s payload bytes are UTF-8 decoded on the way through, matching what a
 * real WebSocket hands the runtime. */
type DeliveredEvent =
  | { kind: "connected"; key: string; conn: number }
  | { kind: "message"; key: string; conn: number; text: string }
  | { kind: "disconnected"; key: string; conn: number }
  | { kind: "error"; key: string; conn: number; message: string };

/** One routed packet, as the packet-log rail and the network view read it. */
export interface Packet {
  /**
   * The REFERENCE-CLOCK frame the packet was routed in — the session's own time
   * axis, the one the chrono rail labels (mp-panes' `referenceFrame`). This is
   * what makes traffic scrubbable: park the rail at frame N and the view can
   * replay exactly the packets that crossed around N.
   *
   * MEASURED, not scheduled: there is no step barrier yet, so this is the
   * reference pane's live head at route time, ±a frame. When the barrier lands
   * it becomes the step the packet is SENT in, by construction.
   *
   * `null` when there is no reference clock to key against — a host that passed
   * no `referenceFrame`, or a session whose reference pane has not recorded a
   * frame yet (a packet from the boot handshake).
   */
  frame: number | null;
  /**
   * The frame the packet is DELIVERED on — `frame` plus its link's latency and
   * jitter draw, clamped so it never overtakes the previous packet on the same
   * connection (see `schedule`). Scheduled at route time, which is what makes
   * an in-flight packet queryable: a packet is on the wire at playhead `p` when
   * `frame <= p < deliveredFrame`, and its flight LASTS `deliveredFrame -
   * frame` frames — the number the wire rows print and the dots fly.
   *
   * Equal to `frame` on a link fast enough to round to zero frames (LAN).
   * `null` in the two cases where there is no crossing to describe: no
   * reference clock to schedule against (the boot handshake — it delivers on
   * the next flush), or a delivery ABANDONED because the destination document
   * navigated away before its frame came up.
   */
  deliveredFrame: number | null;
  /** When the coordinator routed it (`performance.now()`). Host-clock time, not
   * game time: the wall-clock sibling of `frame`, kept because a live view
   * animates on the host's clock while `frame` places the packet in the
   * session's. */
  at: number;
  /** Pane id the packet originated from. */
  from: string;
  /** Pane id it was delivered to. */
  to: string;
  conn: number;
  kind: DeliveredEvent["kind"];
  /** Payload bytes for a message; 0 for the lifecycle kinds. */
  size: number;
  /**
   * The message's wire TEXT, exactly as the receiving runtime sees it — kept so
   * a reader can decode it into the value the game actually sent (wire-value.ts)
   * rather than showing bytes. Undefined for the lifecycle kinds.
   *
   * It costs nothing to route (the coordinator already decodes the payload for
   * delivery, so this is the same string) and it is what bounds the log's
   * memory: the cap is a packet count, so a chatty snapshot protocol holds
   * roughly `PACKET_LOG_CAP × payload` bytes of text. Decoding is deliberately
   * NOT done here — only the handful of rows a panel shows are ever parsed.
   */
  text?: string;
}

/** Newest entries win: an hour-long session must not grow without bound. The
 * trim runs in blocks so it is not an O(n) memmove per packet at the cap. */
const PACKET_LOG_CAP = 10_000;
const PACKET_LOG_SLACK = 1_000;

/**
 * …and a second bound, on the payload TEXT the log retains: a count alone is
 * not a memory bound, because a game chooses how big a message is. 8 MB holds
 * the whole 10k-packet cap for an ordinary protocol (orbs' snapshots are ~800
 * bytes) and sheds oldest-first for a chatty one.
 */
const PACKET_LOG_BYTES = 8 << 20;

/**
 * How long a `Connect` with no listener waits for its server pane before it
 * errors.
 *
 * It cannot fail fast: a pane's runtime reconciles `Sub.connect` against its
 * DECLARED keys, so a key that has been emitted once is never re-emitted
 * (functor_lang_producer::reconcile_connections). An error delivered to a
 * client that booted a few hundred ms before its server pane would therefore
 * be permanent, and so would one delivered while the server pane is RELOADING
 * (which is why `close` re-queues the client end — see there). Panes race by
 * construction here; a lockstep in-process harness has no such window and
 * errors immediately. The window is generous because a cold pane boot is a wasm
 * fetch + compile, and the cost of being wrong is a dead session.
 */
const CONNECT_GRACE_MS = 15_000;

/** Hot path: one decoder for every `Send` that crosses the coordinator. */
const DECODER = new TextDecoder();

/**
 * The timeline's fixed tick rate (timeline-model.js TIMELINE_FPS) — how a link
 * profile's milliseconds become the frames the schedule is expressed in.
 *
 * Exported because the schedule is what everything downstream now measures in:
 * the pane grid's rail (mp-panes) and the packet dots' flight (mp-network) have
 * to mean the same frame this file scheduled against, and three private 60s
 * would be three things to keep in step.
 */
export const TIMELINE_FPS = 60;

/** A link profile's delay, in reference-clock frames. Rounded, so a link faster
 * than half a frame (LAN's 8ms) is honestly zero: the frame is the finest
 * resolution the schedule has. */
const framesOf = (ms: number): number =>
  Math.max(0, Math.round((ms * TIMELINE_FPS) / 1000));

/**
 * Delivery is never abandoned. If the reference clock stalls (a paused session
 * whose panes somehow keep sending, a background tab) the queue would otherwise
 * grow without bound — so past this many packets the oldest are delivered
 * EARLY. Dropping them is the one thing a reliable-ordered channel may not do
 * (Addendum 8.2), so the pressure valve gives up the impairment instead.
 */
const MAX_IN_FLIGHT = 2_000;

/**
 * Deterministic PRNG (SplitMix64), the same algorithm and constants as
 * `functor_runtime_common::net::Rng` — the jitter draws must be reproducible
 * from a seed, so nothing here touches `Math.random`.
 *
 * BigInt because the algorithm is defined on 64-bit wrapping arithmetic: a
 * 32-bit stand-in would be a different generator wearing its name. It costs a
 * handful of BigInt ops per IMPAIRED packet (none at all on a jitter-free
 * link), against a JSON decode per packet on the same path.
 */
const MASK64 = (1n << 64n) - 1n;

class Rng {
  private state: bigint;

  constructor(seed: bigint) {
    this.state = seed & MASK64;
  }

  next(): bigint {
    this.state = (this.state + 0x9e3779b97f4a7c15n) & MASK64;
    let z = this.state;
    z = ((z ^ (z >> 30n)) * 0xbf58476d1ce4e5b9n) & MASK64;
    z = ((z ^ (z >> 27n)) * 0x94d049bb133111ebn) & MASK64;
    return (z ^ (z >> 31n)) & MASK64;
  }

  /** An integer in `[0, hi]` inclusive (Rust's `range_u32(0, hi)`). */
  upTo(hi: number): number {
    if (hi <= 0) return 0;
    return Number(this.next() % BigInt(hi + 1));
  }
}

/**
 * Seeds run 1, 2, 3… per page, so a session's jitter is stable while it lives
 * (a rebuilt coordinator is a new session and gets the next seed). Not a
 * wall-clock seed: "stable within a session" is what makes two reads of the
 * same log agree, and a fixed sequence is what will make cross-run replay
 * possible once barrier stepping pins WHEN a pane sends.
 */
let nextSessionSeed = 1n;

/** The authority (host:port) of an endpoint, ignoring scheme and path — the
 * same rule the Rust `authority()` applies, so both agree on what matches. */
const authorityOf = (endpoint: string): string => {
  const afterScheme = endpoint.split("://").at(-1) ?? endpoint;
  return afterScheme.split("/")[0] ?? afterScheme;
};

interface Pane {
  id: string;
  frame: HTMLIFrameElement;
  /** Everything this pane registered outside the coordinator's own scope
   * (the iframe `load` hook and the current document's `pagehide`). */
  scope: AbortController;
}

/** One end of a connection: the pane, and the key ITS runtime routes by. */
interface Side {
  pane: string;
  key: string;
}

interface Conn {
  client: Side;
  server: Side;
}

interface PendingConnect {
  pane: string;
  key: string;
  since: number;
}

/**
 * What a link does to the packets crossing it. LATENCY AND JITTER ONLY — the
 * chips' loss/reorder fields are not here, and deliberately so (Addendum 8.2:
 * dropping a packet on a reliable-ordered channel would be a lie about
 * `Sub.connect`).
 *
 * `jitter` is EXTRA delay, drawn uniformly from `[0, jitter]` — the same
 * one-sided shape `VirtualNet` uses. A packet can arrive late, never early:
 * arriving early would mean the link beat its own latency.
 */
export interface LinkImpairment {
  /** One-way latency, in ms. */
  ms: number;
  /** Jitter, in ms — the width of the extra-delay draw. */
  jitter: number;
}

export interface NetCoordinatorOptions {
  /**
   * The session's REFERENCE CLOCK, as a frame number — what every routed packet
   * is keyed to (`Packet.frame`).
   *
   * A getter rather than a pushed value: the coordinator is constructed before
   * any pane exists, and the clock is a live measurement of whichever pane is
   * currently the reference (the server pane, or client 1 without one). The
   * host owns that definition — the coordinator only asks, per packet, what
   * time it is. `null` while nothing has recorded a frame yet.
   */
  referenceFrame?: () => number | null;
  /**
   * The impairment on a client's link, by its pane id — the link chip, read
   * live at ROUTE time so a profile change takes effect on the next packet.
   * Packets already in flight keep the schedule they were given (re-timing them
   * could reorder a reliable channel, which is the one thing this may not do).
   *
   * The pane asked about is always the CLIENT end of the pair: a link belongs
   * to a client, and the authority has none. `null`/omitted is a perfect link.
   */
  link?: (clientPane: string) => LinkImpairment | null;
}

/** One packet waiting for its delivery frame. */
interface InFlight {
  /** The destination pane. */
  pane: string;
  event: DeliveredEvent;
  /** The reference frame it is due on; `null` delivers on the next flush. */
  due: number | null;
  /** Its row in the log, so the record can be CORRECTED when the schedule is
   * not what happens: an early flush under the pressure valve, or a delivery
   * abandoned because the destination document navigated away. Without this the
   * log would keep claiming a crossing that never took place. */
  packet: Packet;
}

export class NetCoordinator {
  private readonly panes = new Map<string, Pane>();
  /** authority -> the listening side (pane + its listen key). */
  private readonly listeners = new Map<string, Side>();
  /** Shared connection id -> its two ends. Ids start at 1: 0 is the "no
   * connection" id an unroutable-connect error carries. */
  private readonly conns = new Map<number, Conn>();
  private readonly pending: PendingConnect[] = [];
  private readonly log: Packet[] = [];
  /** Payload text the log is holding, in characters (see PACKET_LOG_BYTES). */
  private logBytes = 0;
  private readonly watchers = new Set<(packet: Packet) => void>();
  /** Scheduled packets, in send order — which is also delivery order within a
   * connection, because the clamp in `schedule` keeps it so. */
  private readonly inFlight: InFlight[] = [];
  /** `conn|destination pane` -> the last delivery frame scheduled that way: the
   * FIFO clamp's state, keyed by where a packet is GOING — which is one entry
   * per direction for every connection between two panes, and one shared entry
   * for the loopback row `open()` documents (a pane that dials its own
   * authority). Sharing it there over-clamps and never reorders, and keying by
   * destination is also what keeps a `disconnected` clamped behind the messages
   * still in flight, after its connection row is already gone. */
  private readonly lastDue = new Map<string, number>();
  private readonly rng = new Rng(nextSessionSeed++);
  /** The reference frame the last flush saw — how a clock that went backwards
   * (a reloaded or newly promoted reference pane) is noticed. */
  private lastNow: number | null = null;
  private nextConn = 1;
  private raf = 0;
  private readonly abort = new AbortController();

  private readonly options: NetCoordinatorOptions;

  constructor(options: NetCoordinatorOptions = {}) {
    this.options = options;
    window.addEventListener("message", (event) => this.onMessage(event), {
      signal: this.abort.signal,
    });
    const loop = () => {
      this.flush();
      this.raf = requestAnimationFrame(loop);
    };
    this.raf = requestAnimationFrame(loop);
  }

  /**
   * Register a pane. `id` is the routing identity in the packet log, so it
   * should be the label the UI shows ("client 2", "server").
   *
   * A pane that navigates (reload, or a new `src` for another example) is a
   * NEW game with a new model, so whatever the previous document had open is
   * closed. That happens on the outgoing document's `pagehide` rather than on
   * the incoming one's `load`: `contentWindow` survives the navigation, and
   * player.html arms its delivery listener BEFORE `load` fires, so resetting
   * only on `load` leaves a window in which a queued event is posted into the
   * replacement — which queues it and hands the fresh game a connection id
   * that belongs to a dead one. `load` re-arms the hook for the next
   * navigation (and covers the first document, which has none).
   *
   * `signal` (mp-panes' per-pane `AbortController`) removes the pane entirely.
   */
  addPane(id: string, frame: HTMLIFrameElement, signal?: AbortSignal): void {
    const scope = new AbortController();
    this.panes.set(id, { id, frame, scope });
    const armUnload = () => {
      frame.contentWindow?.addEventListener("pagehide", () => this.resetPane(id), {
        once: true,
        signal: scope.signal,
      });
    };
    frame.addEventListener(
      "load",
      () => {
        this.resetPane(id);
        armUnload();
      },
      { signal: scope.signal }
    );
    armUnload();
    signal?.addEventListener("abort", () => this.removePane(id), { once: true });
  }

  /** Drop a pane and close every connection it held (both ends notified). */
  removePane(id: string): void {
    this.resetPane(id);
    this.panes.get(id)?.scope.abort();
    this.panes.delete(id);
  }

  /** Every packet routed so far, oldest first (capped at the last 10k). */
  packets(): readonly Packet[] {
    return this.log;
  }

  /**
   * Forget every routed packet. A new PROGRAM is a new session: its panes
   * restart their clocks from zero, so the previous program's rows would
   * otherwise share frame numbers with the new one's and reappear under the
   * playhead as traffic that never happened (and, on a quiet link, as another
   * protocol's decoded messages).
   */
  clearLog(): void {
    this.log.length = 0;
    this.logBytes = 0;
  }

  /**
   * Watch packets as they route. The log is the record; this is the live feed a
   * view animates from, so it does not have to poll a 10k array every frame.
   *
   * Called synchronously from the routing path, so a watcher must be cheap and
   * must not route anything itself — the network view buffers and spends its
   * work on the next frame. Returns an unsubscribe.
   */
  onPacket(watcher: (packet: Packet) => void): () => void {
    this.watchers.add(watcher);
    return () => this.watchers.delete(watcher);
  }

  /**
   * How many live connections each pane holds. The coordinator is the only
   * place that knows who is actually talking to whom, so this is what a pane's
   * link indicator reads — a pane absent from the map is still waiting.
   */
  connectionCounts(): Map<string, number> {
    const counts = new Map<string, number>();
    for (const row of this.conns.values()) {
      for (const side of [row.client, row.server]) {
        counts.set(side.pane, (counts.get(side.pane) ?? 0) + 1);
      }
    }
    return counts;
  }

  destroy(): void {
    cancelAnimationFrame(this.raf);
    this.abort.abort();
    for (const pane of this.panes.values()) pane.scope.abort();
    this.panes.clear();
    this.listeners.clear();
    this.conns.clear();
    this.pending.length = 0;
    this.inFlight.length = 0;
    this.lastDue.clear();
    this.watchers.clear();
  }

  // ------------------------------------------------------------------ ingress

  private onMessage(event: MessageEvent): void {
    // Same-origin panes only: these commands carry game payloads, and a
    // delivery is data the receiving game trusts as its network.
    if (event.origin !== window.location.origin) return;
    const data = event.data as { type?: unknown; commands?: unknown } | null;
    if (typeof data !== "object" || data === null) return;
    if (data.type !== "functor-net-commands") return;
    // `source` is null for a discarded window — and so is a detached iframe's
    // `contentWindow`, so the two would otherwise "match".
    if (!event.source) return;
    const pane = [...this.panes.values()].find((p) => p.frame.contentWindow === event.source);
    if (!pane) return;
    if (!Array.isArray(data.commands)) return;
    for (const command of data.commands as ConnCommand[]) this.perform(pane, command);
  }

  private perform(pane: Pane, command: ConnCommand): void {
    if ("Listen" in command) {
      // Idempotent by authority, like every host treats `Listen` (a hot reload
      // re-declares it every time). The newest declaration wins.
      this.listeners.set(authorityOf(command.Listen.key), {
        pane: pane.id,
        key: command.Listen.key,
      });
      // A client that connected before this pane booted is waiting on it.
      this.retryPending();
      return;
    }
    if ("Connect" in command) {
      const { key } = command.Connect;
      // Idempotent by key: a re-declare while the connection is already up
      // must not open a second one.
      if (this.connFor(pane.id, key) !== null) return;
      if (this.pending.some((p) => p.pane === pane.id && p.key === key)) return;
      if (!this.open(pane.id, key)) {
        this.pending.push({ pane: pane.id, key, since: performance.now() });
      }
      return;
    }
    if ("Send" in command) {
      const { conn, payload } = command.Send;
      // A send on a connection this pane is not an end of (a closed one, or a
      // stale id a model carried across a reload/rewind) is DROPPED, not
      // misrouted — `VirtualNet::send` refuses the same case via `peer_of`.
      const peer = this.peerOf(conn, pane.id);
      if (!peer) return;
      const text = DECODER.decode(Uint8Array.from(payload));
      this.deliver(pane.id, peer, { kind: "message", key: peer.key, conn, text }, payload.length);
      return;
    }
    if ("CloseConn" in command) {
      // Same ownership rule: only an end of a connection may close it.
      if (this.peerOf(command.CloseConn.conn, pane.id)) this.close(command.CloseConn.conn, pane.id);
      return;
    }
    if ("CloseKey" in command) {
      const { key } = command.CloseKey;
      const listener = this.listeners.get(authorityOf(key));
      if (listener?.pane === pane.id && listener.key === key) {
        this.listeners.delete(authorityOf(key));
      }
      for (const [id, row] of [...this.conns]) {
        if (
          (row.client.pane === pane.id && row.client.key === key) ||
          (row.server.pane === pane.id && row.server.key === key)
        ) {
          this.close(id, pane.id);
        }
      }
      this.dropPending((p) => p.pane === pane.id && p.key === key);
    }
  }

  // ------------------------------------------------------------------ routing

  /** Resolve a client key to its listening pane and wire the pair up. Returns
   * false when nobody is listening on that authority (the caller waits).
   *
   * KNOWN, inherited from `VirtualNet` (whose `peer_of` has the same
   * degeneracy): a pane that both listens on and connects to its OWN authority
   * — a host that also plays — gets both ends of one row, so its own sends
   * come back under the server key. Real per-end id spaces are the fix, and
   * belong with the PR that introduces a host-and-play pane. */
  private open(paneId: string, key: string): boolean {
    const server = this.listeners.get(authorityOf(key));
    if (!server || !this.panes.has(server.pane)) return false;
    const conn = this.nextConn++;
    this.conns.set(conn, { client: { pane: paneId, key }, server: { ...server } });
    // BOTH ends learn about the connection, each under its own routing key —
    // the client under the url it connected to, the server under its bind.
    // Each is logged as coming FROM its peer, so the log reads as traffic
    // between two panes rather than as a pane talking to itself.
    this.deliver(server.pane, { pane: paneId, key }, { kind: "connected", key, conn }, 0);
    this.deliver(paneId, server, { kind: "connected", key: server.key, conn }, 0);
    return true;
  }

  private close(conn: number, byPane: string): void {
    const row = this.conns.get(conn);
    if (!row) return;
    this.conns.delete(conn);
    for (const side of [row.client, row.server]) {
      this.deliver(byPane, side, { kind: "disconnected", key: side.key, conn }, 0);
    }
    // The clamp's state dies with the connection: a new connection reusing this
    // id is a new stream, and inheriting a delivery frame from the old one
    // would delay its first packets by whatever the last one was carrying.
    for (const side of [row.client, row.server]) this.lastDue.delete(`${conn}|${side.pane}`);
    // A client whose peer went away (the server pane reloaded, or closed the
    // connection) can never ask again: its runtime only emits `Connect` for a
    // key it has not already declared, and a `disconnected` does not clear
    // that. So the coordinator re-queues the client end on its behalf, and the
    // server's next `Listen` re-opens it. Without this, reloading the server
    // pane kills every client for the rest of the session.
    if (
      byPane !== row.client.pane &&
      this.panes.has(row.client.pane) &&
      !this.pending.some((p) => p.pane === row.client.pane && p.key === row.client.key)
    ) {
      this.pending.push({ ...row.client, since: performance.now() });
    }
  }

  /** The peer end of `conn` as seen from `paneId`, or null when that pane is
   * not on the connection (or it does not exist). */
  private peerOf(conn: number, paneId: string): Side | null {
    const row = this.conns.get(conn);
    if (!row) return null;
    if (row.client.pane === paneId) return row.server;
    if (row.server.pane === paneId) return row.client;
    return null;
  }

  /** The live connection this pane holds for `key`, if any. */
  private connFor(paneId: string, key: string): number | null {
    for (const [id, row] of this.conns) {
      if (row.client.pane === paneId && row.client.key === key) return id;
    }
    return null;
  }

  /**
   * When this event lands, in reference-clock frames.
   *
   * Only MESSAGES are impaired: the lifecycle events are the coordinator's own
   * bookkeeping, not traffic the game put on the wire (they carry no payload
   * and the pane headers, not the log, are what read them). They still take the
   * FIFO clamp, so a `disconnected` can never overtake a message still in
   * flight on the connection it closes.
   *
   * `null` sent frame — no reference clock yet — schedules nothing: an
   * unscheduled packet flushes immediately, which is exactly the boot
   * handshake's old behaviour.
   */
  private schedule(sent: number | null, to: Side, event: DeliveredEvent): number | null {
    if (sent === null) return null;
    const conn = this.conns.get(event.conn);
    const profile =
      event.kind === "message" && conn ? this.options.link?.(conn.client.pane) : null;
    const jitter = profile ? framesOf(profile.jitter) : 0;
    let due = sent + (profile ? framesOf(profile.ms) : 0) + this.rng.upTo(jitter);
    // FIFO per connection per direction (VirtualNet's rule): a later-sent
    // packet never overtakes an earlier one, whatever the jitter drew. Clamped
    // to the previous delivery rather than PAST it — packets that land on the
    // same frame still arrive in send order (one flush, one ordered batch), and
    // a burst must not be spread a frame apart per packet.
    const key = `${event.conn}|${to.pane}`;
    const previous = this.lastDue.get(key);
    if (previous !== undefined && due < previous) due = previous;
    this.lastDue.set(key, due);
    return due;
  }

  private deliver(from: string, to: Side, event: DeliveredEvent, size: number): void {
    const pane = this.panes.get(to.pane);
    if (!pane) return;
    const frame = this.options.referenceFrame?.() ?? null;
    const due = this.schedule(frame, to, event);
    const packet: Packet = {
      frame,
      deliveredFrame: due,
      at: performance.now(),
      from,
      to: to.pane,
      conn: event.conn,
      kind: event.kind,
      size,
      ...(event.kind === "message" ? { text: event.text } : {}),
    };
    this.inFlight.push({ pane: to.pane, event, due, packet });
    this.log.push(packet);
    this.logBytes += packet.text?.length ?? 0;
    if (this.log.length > PACKET_LOG_CAP + PACKET_LOG_SLACK) {
      for (const dropped of this.log.splice(0, this.log.length - PACKET_LOG_CAP)) {
        this.logBytes -= dropped.text?.length ?? 0;
      }
    }
    // The byte bound sheds one packet at a time: it only bites for a protocol
    // whose messages are large, and there the newest few are what a reader is
    // looking at anyway.
    while (this.logBytes > PACKET_LOG_BYTES && this.log.length > 1) {
      this.logBytes -= this.log.shift()?.text?.length ?? 0;
    }
    for (const watcher of this.watchers) watcher(packet);
  }

  /** Close everything a pane owns without deregistering the pane itself. */
  private resetPane(id: string): void {
    for (const [conn, row] of [...this.conns]) {
      if (row.client.pane === id || row.server.pane === id) this.close(conn, id);
    }
    for (const [authority, side] of [...this.listeners]) {
      if (side.pane === id) this.listeners.delete(authority);
    }
    this.dropPending((p) => p.pane === id);
    // A navigating pane is a NEW game: everything addressed to the document
    // that is going away goes with it. (The other end of each connection keeps
    // its `disconnected` — that is how it learns.) The log is told: an
    // abandoned packet has no delivery frame, so no view claims it landed.
    for (let i = this.inFlight.length - 1; i >= 0; i--) {
      if (this.inFlight[i].pane !== id) continue;
      this.inFlight[i].packet.deliveredFrame = null;
      this.inFlight.splice(i, 1);
    }
  }

  private dropPending(match: (p: PendingConnect) => boolean): void {
    for (let i = this.pending.length - 1; i >= 0; i--) {
      if (match(this.pending[i])) this.pending.splice(i, 1);
    }
  }

  /** Retry connects whose listener hadn't booted yet; give up after the grace
   * window with a teaching error on the CALLER's key.
   *
   * In ARRIVAL order, so connection ids (and therefore the server's player
   * order) follow the order the clients asked, not the reverse. */
  private retryPending(): void {
    const now = performance.now();
    const requests = this.pending.splice(0);
    for (const request of requests) {
      // Expiry first: a listener that appears after the window has passed must
      // not silently revive a request already deemed dead (rAF is throttled in
      // a background tab, so "the window passed" and "we noticed" differ).
      if (now - request.since > CONNECT_GRACE_MS) {
        this.deliver(
          request.pane,
          { pane: request.pane, key: request.key },
          {
            kind: "error",
            key: request.key,
            conn: 0,
            message:
              `no pane is listening on ${authorityOf(request.key)} — a pane must ` +
              `declare Sub.listen("${authorityOf(request.key)}", …) before a client can connect`,
          },
          0
        );
        continue;
      }
      // Still routable? Otherwise keep waiting.
      if (!this.open(request.pane, request.key)) this.pending.push(request);
    }
  }

  // ------------------------------------------------------------------- egress

  /**
   * Everything whose delivery frame has arrived, one batch per pane, in send
   * order — the impaired link's egress.
   *
   * A single pass over the queue, which is in send order and (per destination)
   * therefore in delivery order too, so "due" is decided packet by packet and
   * the survivors keep their relative order by construction. An unscheduled
   * packet (routed before there was a reference clock) is due immediately; a
   * SCHEDULED one waits for its frame even while the clock is momentarily
   * unreadable, or the impairment would be silently skipped exactly when a pane
   * is between documents.
   *
   * Two escapes, and both DELIVER rather than drop — the channel is reliable:
   *
   *   • the clock went BACKWARDS. It is a live measurement of whichever pane is
   *     currently the reference, so a reloaded (or newly promoted) reference
   *     pane restarts it near zero. Every schedule and every FIFO watermark
   *     belongs to the old timeline, and holding them would stall the session
   *     for as long as the clock was ahead — so a regression opens a new epoch:
   *     drain the queue in order, forget the watermarks.
   *   • more than `MAX_IN_FLIGHT` waiting. A clock that has stopped advancing
   *     must not grow the queue without bound, so the oldest are delivered
   *     early. `forced` is a PREFIX of a send-ordered queue, so this cannot
   *     reorder anything.
   *
   * Both correct the log on the way past: a packet delivered off its schedule
   * says so, because `deliveredFrame` is read as what happened.
   */
  private flush(): void {
    this.retryPending();
    const now = this.options.referenceFrame?.() ?? null;
    const rewound = now !== null && this.lastNow !== null && now < this.lastNow;
    if (rewound) this.lastDue.clear();
    this.lastNow = now ?? this.lastNow;
    if (this.inFlight.length > 0) {
      // One outbox per pane, per flush: a delivery is never held across frames,
      // so this is the whole batch a pane receives.
      const outboxes = new Map<string, DeliveredEvent[]>();
      const forced = Math.max(0, this.inFlight.length - MAX_IN_FLIGHT);
      let kept = 0;
      for (let i = 0; i < this.inFlight.length; i++) {
        const waiting = this.inFlight[i];
        const early = rewound || i < forced;
        if (early || waiting.due === null || (now !== null && waiting.due <= now)) {
          if (early) waiting.packet.deliveredFrame = now;
          const outbox = outboxes.get(waiting.pane);
          if (outbox) outbox.push(waiting.event);
          else outboxes.set(waiting.pane, [waiting.event]);
        } else {
          this.inFlight[kept++] = waiting;
        }
      }
      this.inFlight.length = kept;
      for (const [id, events] of outboxes) {
        this.panes.get(id)?.frame.contentWindow?.postMessage(
          { type: "functor-net-deliver", events },
          window.location.origin
        );
      }
    }
  }
}
