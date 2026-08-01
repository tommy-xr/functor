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
// This module is the thing in between. Its routing model deliberately MIRRORS
// the in-process netsim (`runtime/functor-netsim/src/lib.rs`), so a session that
// runs in panes and the same session run headlessly agree on semantics:
//
//   • a listener registry keyed by AUTHORITY (host:port, scheme and path
//     ignored), so a client url "ws://127.0.0.1:9001/play" matches a server
//     bind "127.0.0.1:9001" — `authority()` there, `authorityOf` here;
//   • one connection id per PAIR, shared by both ends (netsim uses the
//     `VirtualNet` id the same way), so a send from either end routes to the
//     other by looking up the same row;
//   • both ends are told `connected` — the client under its connect key, the
//     accepting server under its LISTEN key — because that key is how each
//     side's runtime routes the event back to the right `Sub.connect` /
//     `Sub.listen` tagger;
//   • FIFO per pane: deliveries queue in arrival order and flush as one batch.
//
// PERFECT LINKS ONLY. Everything a link profile would add — latency, jitter,
// loss, partitions — is deliberately absent, and so is the step barrier: a
// routed packet is delivered on the host's NEXT rAF, not on the receiving
// pane's next fixed step. Impairment and the barrier are later PRs; when the
// barrier lands, `flush()` becomes step-time delivery and `Packet.tick` stops
// being null.
//
// The packet log records from day one even though nothing renders it yet: the
// coordinator is the only place that sees every packet, so it owns the log the
// later packet-log rail reads.

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

/** One routed packet, as the (not yet rendered) packet-log rail will read it. */
export interface Packet {
  /** The step it belongs to — always null until the barrier lands (see above). */
  tick: number | null;
  /** Pane id the packet originated from. */
  from: string;
  /** Pane id it was delivered to. */
  to: string;
  conn: number;
  kind: DeliveredEvent["kind"];
  /** Payload bytes for a message; 0 for the lifecycle kinds. */
  size: number;
}

/** Newest entries win: an hour-long session must not grow without bound. The
 * trim runs in blocks so it is not an O(n) memmove per packet at the cap. */
const PACKET_LOG_CAP = 10_000;
const PACKET_LOG_SLACK = 1_000;

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
 * construction here; netsim, being lockstep, has no such window and errors
 * immediately. The window is generous because a cold pane boot is a wasm
 * fetch + compile, and the cost of being wrong is a dead session.
 */
const CONNECT_GRACE_MS = 15_000;

/** Hot path: one decoder for every `Send` that crosses the coordinator. */
const DECODER = new TextDecoder();

/** The authority (host:port) of an endpoint, ignoring scheme and path — the
 * exact rule netsim's `authority()` applies, so both agree on what matches. */
const authorityOf = (endpoint: string): string => {
  const afterScheme = endpoint.split("://").at(-1) ?? endpoint;
  return afterScheme.split("/")[0] ?? afterScheme;
};

interface Pane {
  id: string;
  frame: HTMLIFrameElement;
  /** Events queued for this pane, flushed in order on the next host rAF. */
  outbox: DeliveredEvent[];
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

export class NetCoordinator {
  private readonly panes = new Map<string, Pane>();
  /** authority -> the listening side (pane + its listen key). */
  private readonly listeners = new Map<string, Side>();
  /** Shared connection id -> its two ends. Ids start at 1: 0 is the "no
   * connection" id an unroutable-connect error carries (as in netsim). */
  private readonly conns = new Map<number, Conn>();
  private readonly pending: PendingConnect[] = [];
  private readonly log: Packet[] = [];
  private nextConn = 1;
  private raf = 0;
  private readonly abort = new AbortController();

  constructor() {
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
    this.panes.set(id, { id, frame, outbox: [], scope });
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

  destroy(): void {
    cancelAnimationFrame(this.raf);
    this.abort.abort();
    for (const pane of this.panes.values()) pane.scope.abort();
    this.panes.clear();
    this.listeners.clear();
    this.conns.clear();
    this.pending.length = 0;
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

  private deliver(from: string, to: Side, event: DeliveredEvent, size: number): void {
    const pane = this.panes.get(to.pane);
    if (!pane) return;
    pane.outbox.push(event);
    this.log.push({ tick: null, from, to: to.pane, conn: event.conn, kind: event.kind, size });
    if (this.log.length > PACKET_LOG_CAP + PACKET_LOG_SLACK) {
      this.log.splice(0, this.log.length - PACKET_LOG_CAP);
    }
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
    const pane = this.panes.get(id);
    if (pane) pane.outbox.length = 0;
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
   * One batch per pane, in arrival order. This is the "perfect link": a packet
   * routed during frame N is delivered at the start of the host's frame N+1,
   * with no impairment applied. The barrier PR moves this to the receiving
   * pane's step boundary.
   */
  private flush(): void {
    this.retryPending();
    for (const pane of this.panes.values()) {
      if (pane.outbox.length === 0) continue;
      const events = pane.outbox.splice(0);
      pane.frame.contentWindow?.postMessage(
        { type: "functor-net-deliver", events },
        window.location.origin
      );
    }
  }
}
