// The NETWORK view's graph layer: the wires between the client panes and the
// server hub, and the packets travelling along them (design:
// ~/notes/projects/functor/design-multiplayer-ide-frontend.md, "Addendum 5 —
// the network view").
//
// The panes themselves are laid out by CSS (styles.css, `[data-view=network]`)
// — this module never positions, reorders, or re-parents a pane. It only
// MEASURES them: every edge endpoint comes from the pane cards' boxes, so the
// wires follow whatever the grid decided at the current size and client count.
//
// Two rules from the addendum are load-bearing here:
//
//   1. COLOR carries identity, SIZE carries payload (Addendum 5a.1 — the
//      hollow-cyan-square authority packet is retired). Both directions are
//      circles: client → server is the client's player color, sized by payload;
//      server → client is the server's slate (--scrub-server) in a TIGHT size
//      range, so the per-frame snapshot stream never dominates the picture.
//   2. PACE is a measurement. See FLIGHT_MS.
//
// The third rule is the WIRE LOG: clicking an edge pins a panel, tethered to
// that wire with a dashed leader, showing that link's traffic as DECODED VALUES
// (`Steer {turn:1}`) rather than as bytes — and locked to the chrono rail, so
// scrubbing sweeps the rows the same way it sweeps the dots.

import type { NetCoordinator, Packet } from "./net-coordinator.js";
import {
  buildWireRow,
  indexAfter,
  LIVE_ROW_MS,
  logWindow,
  onLink,
  openTree,
  packetKey,
  shortName,
} from "./wire-rows.js";
import type { WireValue } from "./wire-value.js";

const SVG_NS = "http://www.w3.org/2000/svg";

/**
 * How long a packet takes to cross its edge, in ms.
 *
 * A FIXED value, and that is honest today: the coordinator routes on perfect
 * links (net-coordinator.ts — no latency, no jitter, no loss), so there is no
 * measurement to render and every edge is genuinely the same speed. The link
 * chips are configured profiles, not observations, which is why they are
 * labelled as such.
 *
 * When impairment lands, this becomes the MEASURED latency of the packet's
 * link, per Addendum 5's "pace is a measurement": flight time is never scaled
 * for looks, and if sub-perceptual links ever force a floor or a log scale, the
 * UI must SAY so with a "×N" chip on the edge.
 */
const FLIGHT_MS = 600;

/**
 * Concurrent dots, hard cap. Oldest is dropped first, so a runaway session sheds
 * packets instead of growing the DOM without bound.
 *
 * Sized for the honest steady state rather than for looks: with one dot per
 * edge per direction per frame (see the batching in `step`), a 60Hz session
 * keeps `FLIGHT_MS / 16.7 ≈ 36` dots in the air per edge per direction, so
 * MAX_CLIENTS × 2 directions ≈ 216 at the busiest. A cap below that is not a
 * safety valve, it is a visual bug: every dot is evicted mid-wire and the
 * stream never reaches the far end.
 */
const MAX_DOTS = 260;

/**
 * How wide a window of the packet log the REPLAY renders, in reference-clock
 * frames.
 *
 * The live feed keeps `FLIGHT_MS` of traffic in the air at once; parking the
 * session must show the same picture at the same density, so the window is that
 * flight time expressed in frames (the timeline's fixed 60fps), DERIVED rather
 * than written down so the two cannot drift apart. A packet routed AT the
 * playhead is at the start of its flight and one a full window
 * back is arriving — so scrubbing sweeps the traffic along the wires exactly as
 * running it would, in either direction.
 */
const REPLAY_WINDOW_FRAMES = Math.round((FLIGHT_MS / 1000) * 60);

/**
 * Rows in the pinned wire log's viewport.
 *
 * Not a scroll buffer: the panel is a WINDOW onto the log at the playhead, and
 * the rail is how you move it. A dozen rows is what a 16:9 stage has room for
 * beside the cards without the panel becoming a second layout.
 */
const WIRE_LOG_ROWS = 12;

/** Rows of context kept AFTER the playhead's row, so the parked frame does not
 * sit on the panel's last line with nothing following it. */
const WIRE_LOG_LEAD = 3;

/** The panel's margin from the stage's edge when it parks in a free quadrant. */
const PANEL_MARGIN = 10;

/**
 * Payload bytes at which a dot reaches its largest size, per direction.
 *
 * INTENT is a keypress-sized message, so it saturates early and its range is
 * wide — a snapshot-sized intent should look heavy. AUTHORITY is a whole world
 * snapshot every frame, so it saturates late and its range is tight
 * (Addendum 5a.1): the stream is constant, and a constant stream of big dots
 * is the thing that drowned the view out.
 */
const FULL_BYTES: Record<Direction, number> = { up: 128, down: 512 };

/**
 * Buffered packets awaiting the next frame, hard cap. rAF is throttled (or
 * stopped) in a background tab while the panes keep routing, so the buffer
 * needs the same discipline the coordinator's own log has: oldest out first,
 * never unbounded.
 */
const INBOX_CAP = 512;

/** A pane as the graph needs it: an identity, a box to measure, and its ink. */
export interface NetworkNode {
  /** The coordinator's routing id ("client 2", "server"). */
  id: string;
  /** The pane card whose box the edge anchors to. */
  shell: HTMLElement;
  /** The pane's player color, as a CSS value. */
  color: string;
  /** The client's configured link profile, as a label. Read live: the chip is a
   * label of the CONFIGURED profile, never a claim about the wire. */
  linkLabel: () => string;
}

export interface NetworkGraphOptions {
  /** The stage the graph shares with the pane grid — its positioning context. */
  stage: HTMLElement;
  /** The pane grid inside it. The wires are inserted BEFORE it and the chips
   * AFTER it, so wires run behind the cards and a chip is never buried under
   * one. */
  grid: HTMLElement;
  net: NetCoordinator;
  clients: () => NetworkNode[];
  server: () => NetworkNode | null;
  /**
   * The session clock, in reference-clock frames (mp-panes owns it). While
   * `parked` — paused, or mid-scrub — the graph stops animating the live feed
   * and REPLAYS the log around `frame` instead: the traffic belongs to the
   * timeline, so the rail decides which packets are on the wires.
   */
  clock: () => { parked: boolean; frame: number | null };
  /**
   * A wire was pinned (or unpinned, with null) — the one place a reader says
   * "THIS link is the interesting one".
   *
   * The wire tab in the bottom panel follows the non-null half of it (see
   * wire-tab.ts's note on the selection model): pinning a wire here focuses the
   * same link there, so one click aims both surfaces. Unpinning is deliberately
   * NOT propagated — "no pin" and "every link" are different statements, and
   * this panel unpins itself whenever the network view closes.
   */
  onSelect?: (id: string | null) => void;
}

export interface NetworkGraph {
  /** Re-measure every edge against the panes' current boxes. */
  relayout(): void;
  /** One frame of the graph: spawn sampled packets, advance the dots. Driven by
   * the host's existing rAF loop rather than a second one. */
  step(): void;
  /** Network view on/off. Off is inert: no listeners fire work, no dots live. */
  setActive(on: boolean): void;
  /** Zero the per-edge packet totals — a new program is a new session, and the
   * reduced-motion badge must not carry the previous example's traffic. */
  resetCounts(): void;
  /** Pin the wire log to a link (a client pane id), or clear it with null. */
  selectEdge(id: string | null): void;
  destroy(): void;
}

type Direction = "up" | "down";

interface Edge {
  node: NetworkNode;
  path: SVGPathElement;
  /** The same curve, drawn invisibly and fat: a 1.4px wire is not a click
   * target, and widening the visible one would make every wire shout. */
  hit: SVGPathElement;
  chip: HTMLButtonElement;
  chipLabel: HTMLElement;
  chipCount: HTMLElement;
  length: number;
  count: number;
  /** The curve's midpoint, in layer coordinates — where the chip sits, and
   * where the pinned panel's tether attaches. */
  mid: Point;
}

interface Dot {
  el: SVGElement;
  edge: Edge;
  dir: Direction;
  /** When the dot was spawned, on the host clock — the LIVE feed's animation. */
  start: number;
  /** Fixed position along the wire (0..1), for a REPLAYED dot: a parked session
   * has no host-clock flight, its position is decided by the playhead. */
  held: number | null;
}

interface Point {
  x: number;
  y: number;
}

/** A measured box in the WIRE LAYER's coordinates — what every geometry in this
 * module is expressed in (the layer shares the stage's box). */
const toLocal = (box: DOMRect, origin: DOMRect): DOMRect =>
  new DOMRect(box.left - origin.left, box.top - origin.top, box.width, box.height);

/** A cubic's point at t = 0.5 — where the edge chip sits. */
const midpoint = (p0: Point, c0: Point, c1: Point, p1: Point): Point => ({
  x: (p0.x + 3 * c0.x + 3 * c1.x + p1.x) / 8,
  y: (p0.y + 3 * c0.y + 3 * c1.y + p1.y) / 8,
});

export function initNetworkGraph({
  stage,
  grid,
  net,
  clients,
  server,
  clock,
  onSelect,
}: NetworkGraphOptions): NetworkGraph {
  const layer = document.createElement("div");
  layer.className = "mp-net-layer";
  stage.insertBefore(layer, grid);
  const chips = document.createElement("div");
  chips.className = "mp-net-chips";
  stage.appendChild(chips);

  const svg = document.createElementNS(SVG_NS, "svg");
  svg.setAttribute("class", "mp-wires");
  svg.setAttribute("aria-hidden", "true");
  const wires = document.createElementNS(SVG_NS, "g");
  const packets = document.createElementNS(SVG_NS, "g");
  // The pinned panel's leader line. Last, so it draws over the wires it points
  // at rather than under them.
  const tether = document.createElementNS(SVG_NS, "g");
  tether.setAttribute("class", "mp-tether");
  const tetherLine = document.createElementNS(SVG_NS, "path");
  const tetherNub = document.createElementNS(SVG_NS, "circle");
  tetherNub.setAttribute("r", "3");
  tether.append(tetherLine, tetherNub);
  svg.append(wires, packets, tether);
  layer.appendChild(svg);

  // The pinned wire log. It rides the CHIPS layer, which sits after the grid in
  // the DOM: a panel of text under a pane card is a panel you cannot read.
  const panel = document.createElement("aside");
  panel.className = "mp-wirelog";
  panel.hidden = true;
  panel.innerHTML = `
    <header class="mp-wl-hd">
      <b class="mp-wl-link"></b>
      <span class="mp-wl-what">wire log</span>
      <button type="button" class="mp-wl-x" title="Clear the pinned wire log">esc to clear ✕</button>
    </header>
    <div class="mp-wl-rows" role="log" aria-label="Wire log"></div>
    <footer class="mp-wl-ft"></footer>`;
  chips.appendChild(panel);
  const panelLink = panel.querySelector(".mp-wl-link") as HTMLElement;
  const panelRows = panel.querySelector(".mp-wl-rows") as HTMLElement;
  const panelFoot = panel.querySelector(".mp-wl-ft") as HTMLElement;

  const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
  const edges = new Map<string, Edge>();
  const dots: Dot[] = [];
  /** Packets seen since the last frame. Routing must stay cheap, so the
   * listener only buffers — every DOM write happens in `step`. */
  let inbox: Packet[] = [];
  let active = false;
  /** Which feed the dots on screen came from. A mode change clears them: a live
   * dot mid-flight and a replayed one mean different things, and letting the
   * two coexist would show traffic at the playhead that never crossed there. */
  let replaying = false;
  /** The frame the replay is currently drawn for, so a held playhead rebuilds
   * nothing. `null` = nothing drawn yet. */
  let replayFrame: number | null = null;
  /** The link whose wire log is pinned (a client pane id), or null. */
  let selected: string | null = null;
  /** What the panel's rows currently say, so a steady session rewrites nothing. */
  let rowKey = "";
  /** When the rows last repainted — the live tail's throttle (LIVE_ROW_MS). */
  let lastRowPaint = 0;
  /**
   * The row opened into a value tree, if any: which packet it is, the tree's
   * own DOM (kept across repaints, so the nodes the reader opened stay open),
   * and — when it opened on a LIVE tail — the viewport to hold there.
   *
   * The packet is held BY IDENTITY, not by a hash: the panel and the log share
   * the same `Packet` objects, and the row hash (frame/sender/size/arrival) can
   * genuinely collide — `at` is a clamped `performance.now()`, so two
   * same-sized intents routed in one turn of the loop can carry the same stamp,
   * and two rows would then both claim the one tree.
   *
   * Holding the viewport is the point of `held`. A live tail turns a twelve-row
   * window over in a fraction of a second at snapshot rates, so a row that
   * opened and kept tailing would scroll off before it could be read: opening a
   * row IS the pause. Parked, the rail stays in charge (scrub-lock is the
   * panel's promise), `held` is null, and the tree simply travels with its row
   * until the row leaves the window.
   */
  let opened: { packet: Packet; host: HTMLElement; held: Packet[] | null } | null = null;
  /** The packet whose payload button should take focus after the next repaint —
   * a repaint rebuilds every row, and the button the reader just pressed with
   * the keyboard is one of the ones it destroys. */
  let refocus: Packet | null = null;

  const addEdge = (node: NetworkNode): Edge => {
    const path = document.createElementNS(SVG_NS, "path");
    path.setAttribute("class", "mp-wire");
    // The player color rides on `color`, so the wire's stroke and the dot's
    // fill and glow can all be one `currentColor` in the stylesheet.
    path.style.color = node.color;
    const hit = document.createElementNS(SVG_NS, "path");
    hit.setAttribute("class", "mp-wire-hit");
    hit.addEventListener("click", () => selectEdge(node.id === selected ? null : node.id));
    wires.append(path, hit);
    // A real button: the chip is the wire's label AND its accessible click
    // target — the SVG curve is reachable by pointer only.
    const chip = document.createElement("button");
    chip.type = "button";
    chip.className = "mp-edge-chip";
    chip.innerHTML = `<b></b><i></i>`;
    chip.addEventListener("click", () => selectEdge(node.id === selected ? null : node.id));
    chips.appendChild(chip);
    const edge: Edge = {
      node,
      path,
      hit,
      chip,
      chipLabel: chip.querySelector("b")!,
      chipCount: chip.querySelector("i")!,
      length: 0,
      count: 0,
      mid: { x: 0, y: 0 },
    };
    edges.set(node.id, edge);
    return edge;
  };

  const dropEdge = (edge: Edge, id: string) => {
    edge.path.remove();
    edge.hit.remove();
    edge.chip.remove();
    edges.delete(id);
    // A link that is gone cannot stay pinned — its rows would freeze on a
    // client that is no longer in the session.
    if (selected === id) selectEdge(null);
    for (let i = dots.length - 1; i >= 0; i--) {
      if (dots[i].edge === edge) {
        dots[i].el.remove();
        dots.splice(i, 1);
      }
    }
  };

  const relayout = () => {
    const hub = server();
    const live = hub ? clients() : [];
    // Reconcile the edge set with the current panes (a count change, or an
    // example that mounted/removed its authority).
    for (const [id, edge] of [...edges]) {
      if (!live.some((node) => node.id === id)) dropEdge(edge, id);
    }
    if (!hub || live.length === 0) return;

    // ONE read pass, then one write pass: the boxes are all measured before
    // anything is written back, so a re-measure never thrashes layout.
    const origin = layer.getBoundingClientRect();
    if (origin.width === 0 || origin.height === 0) return;
    svg.setAttribute("viewBox", `0 0 ${origin.width} ${origin.height}`);
    const boxOf = (node: NetworkNode) => {
      const box = toLocal(node.shell.getBoundingClientRect(), origin);
      return {
        left: box.left,
        top: box.top,
        width: box.width,
        height: box.height,
        cx: box.left + box.width / 2,
        cy: box.top + box.height / 2,
      };
    };
    const hubBox = boxOf(hub);
    const geometry = live.map((node) => {
      const box = boxOf(node);
      const dx = hubBox.cx - box.cx;
      const dy = hubBox.cy - box.cy;
      // Leave from whichever pair of sides actually faces the hub. Pick the
      // axis the two cards are SEPARATED on, not simply the larger centre
      // delta: a tall stage can make |dy| win between cards that overlap
      // vertically, and the wire would then leave a card's bottom for a hub
      // edge above it — running backwards behind both.
      const apartX = Math.max(0, hubBox.left - (box.left + box.width), box.left - (hubBox.left + hubBox.width));
      const apartY = Math.max(0, hubBox.top - (box.top + box.height), box.top - (hubBox.top + hubBox.height));
      const horizontal = apartX === apartY ? Math.abs(dx) >= Math.abs(dy) : apartX > apartY;
      let from: Point;
      let to: Point;
      let control: Point;
      // The bow is a fraction of the GAP the wire actually crosses, not of the
      // distance between the cards' centres: the cards nearly touch, and a
      // bow sized off their centres would swing the whole curve out behind
      // them — a wire (and its packets) you cannot see.
      if (horizontal) {
        from = { x: box.left + (dx > 0 ? box.width : 0), y: box.cy };
        to = { x: hubBox.left + (dx > 0 ? 0 : hubBox.width), y: hubBox.cy };
        control = { x: (to.x - from.x) * 0.5, y: 0 };
      } else {
        from = { x: box.cx, y: box.top + (dy > 0 ? box.height : 0) };
        to = { x: hubBox.cx, y: hubBox.top + (dy > 0 ? 0 : hubBox.height) };
        control = { x: 0, y: (to.y - from.y) * 0.5 };
      }
      const c0 = { x: from.x + control.x, y: from.y + control.y };
      const c1 = { x: to.x - control.x, y: to.y - control.y };
      return { node, from, c0, c1, to };
    });

    const written: Edge[] = [];
    for (const { node, from, c0, c1, to } of geometry) {
      const edge = edges.get(node.id) ?? addEdge(node);
      edge.node = node;
      edge.path.style.color = node.color;
      edge.hit.style.color = node.color;
      const d = `M${from.x},${from.y} C${c0.x},${c0.y} ${c1.x},${c1.y} ${to.x},${to.y}`;
      edge.path.setAttribute("d", d);
      edge.hit.setAttribute("d", d);
      const mid = midpoint(from, c0, c1, to);
      edge.mid = mid;
      edge.chip.style.left = `${mid.x}px`;
      edge.chip.style.top = `${mid.y}px`;
      written.push(edge);
    }
    // Lengths last: `getTotalLength` is a geometry READ, and interleaving it
    // with the `d` writes above would flush per edge instead of once.
    for (const edge of written) edge.length = edge.path.getTotalLength();
    paintChips();
    placePanel();
  };

  // Every write is change-guarded, so this is a couple of string compares per
  // frame — cheaper than the bookkeeping a throttle would need.
  const paintChips = () => {
    for (const edge of edges.values()) {
      const label = edge.node.linkLabel();
      if (edge.chipLabel.textContent !== label) {
        edge.chipLabel.textContent = label;
        // The chip labels a CONFIGURED profile — the links themselves are still
        // perfect (net-coordinator.ts). Say that, rather than implying a
        // measurement the coordinator cannot make yet.
        edge.chip.title =
          `${label} — this client's configured link profile. Impairment is ` +
          `recorded, not applied yet, so every packet crosses at the same pace. ` +
          `Click to pin this link's wire log.`;
      }
      const count = `${edge.count} pkt`;
      if (edge.chipCount.textContent !== count) edge.chipCount.textContent = count;
    }
  };

  // ---------------------------------------------------------- the wire log
  // Clicking a wire pins this panel to it (design addendum 5, rule 3): that
  // link's traffic, both directions interleaved by frame, as the VALUES the
  // game sent. It is a panel rather than a hover tooltip precisely because it
  // has to stay readable while the other hand is scrubbing.

  // The row grammar itself — `#f · direction · payload · bytes`, plus the log
  // walks that place a viewport on the playhead — is shared with the wire tab
  // (wire-rows.ts), so the two surfaces read identically.

  const selectEdge = (id: string | null) => {
    if (id !== null && !edges.has(id)) return;
    if (selected === id) return;
    selected = id;
    rowKey = "";
    // Another link is other traffic: the open row is not in it.
    opened = null;
    for (const [edgeId, edge] of edges) {
      const on = edgeId === id;
      edge.path.classList.toggle("selected", on);
      // The fat hit path doubles as the selection HALO — see the stylesheet.
      edge.hit.classList.toggle("selected", on);
      edge.chip.classList.toggle("selected", on);
      edge.chip.setAttribute("aria-pressed", String(on));
    }
    renderPanel();
    // A new link is a new tether and a new free quadrant to find, even when the
    // row count happens to match (which is what renderPanel re-places on).
    placePanel();
    onSelect?.(id);
  };

  /**
   * The panel's viewport onto the log: this link's message traffic, oldest
   * first, both directions in one list (which is what "interleaved by frame"
   * means when the log is already in routing order), plus which row the
   * playhead is on.
   *
   * SCRUB-LOCKED, and the lock itself is shared with the wire tab
   * (`logWindow`): one time axis, two representations (design §5), so the two
   * surfaces cannot answer the rail differently. All this adds is WHICH packets
   * count as this panel's — the pinned link's.
   */
  const viewportRows = (
    id: string,
    at: number | null
  ): { rows: Packet[]; highlight: number } =>
    logWindow(net.packets(), at, {
      match: (packet) => onLink(packet, id),
      back: WIRE_LOG_ROWS,
      lead: WIRE_LOG_LEAD,
    });

  /**
   * Open a row into a value tree — or close the one that is open.
   *
   * One row at a time: the panel is a narrow column beside a running game, and
   * two open snapshots would be a scroll, not a reading. Opening rebuilds the
   * rows immediately (bypassing the live throttle) so the aria state of every
   * payload button matches the DOM in the same tick, and re-places the panel,
   * whose height just changed.
   */
  const openRow = (packet: Packet, value: WireValue, shown: readonly Packet[]) => {
    if (opened?.packet === packet) {
      opened = null;
    } else {
      // The panel positions itself against its own height, so a node opening
      // inside the tree has to re-place it.
      opened = {
        packet,
        host: openTree(value, placePanel),
        held: clock().parked ? null : [...shown],
      };
    }
    // The reader may have pressed this button with the keyboard, and the
    // repaint below destroys it — hand the focus to its replacement.
    refocus = packet;
    rowKey = "";
    lastRowPaint = 0;
    renderPanel();
    placePanel();
  };

  const renderPanel = () => {
    if (!selected || !active) {
      panel.hidden = true;
      tether.setAttribute("visibility", "hidden");
      return;
    }
    const edge = edges.get(selected);
    if (!edge) return;
    const { parked, frame } = clock();
    // LIVE the rows are a tail, and a tail that turns over sixty times a second
    // is not readable — so it repaints at a rate a reader can follow, and the
    // work (decode + rebuild) happens at that rate too. PARKED there is no
    // throttle: the panel must answer the rail immediately.
    const now = performance.now();
    if (!parked && now - lastRowPaint < LIVE_ROW_MS) return;
    const live = viewportRows(selected, parked && frame !== null ? frame : null);
    // An open row holds the LIVE viewport (see `opened`); parked, the rail
    // keeps deciding the window and the tree travels with its row.
    const held = opened?.held;
    const shown = held && !parked ? held : live.rows;
    const { highlight } = live;
    // A row that scrubbed out of the window takes its tree with it.
    if (opened && !shown.includes(opened.packet)) opened = null;
    const key =
      `${selected}|${highlight}|${opened ? packetKey(opened.packet) : ""}|` +
      // `at` disambiguates two packets that share a frame, a direction and a
      // size — a fixed-width intent (`Steer {turn:0}` vs `{turn:1}`) otherwise
      // hashes the same and the rows would go stale.
      shown.map(packetKey).join(",");
    if (key === rowKey) return;
    const rowCount = panelRows.childElementCount;
    lastRowPaint = now;
    rowKey = key;

    panel.hidden = false;
    panelLink.textContent = `${shortName(selected)} ↔ srv`;
    panel.style.setProperty("--pc", edge.node.color);
    let anyPlain = false;
    let focusTarget: HTMLElement | null = null;
    panelRows.replaceChildren(
      ...shown.map((packet, index) => {
        const isOpen = opened?.packet === packet;
        const built = buildWireRow({
          packet,
          at: index === highlight,
          tree: isOpen && opened ? opened.host : null,
          onOpen: (value) => openRow(packet, value, shown),
        });
        if (!built.typed) anyPlain = true;
        if (refocus === packet) focusTarget = built.button;
        return built.el;
      })
    );
    // The rebuild threw away the element the keyboard was on; put the focus on
    // its replacement, and only for the repaint the press caused.
    if (focusTarget) (focusTarget as HTMLElement).focus();
    refocus = null;
    if (shown.length === 0) {
      const empty = document.createElement("p");
      empty.className = "mp-wl-empty";
      empty.textContent = parked
        ? "nothing had crossed this link yet at this frame"
        : "no traffic on this link yet";
      panelRows.appendChild(empty);
    }
    panelFoot.textContent =
      opened && !parked
        ? "the tail is held while a row is open — close it to resume"
        : anyPlain
          ? "Effect.send text — shown exactly as sent"
          : "Effect.sendMsg — decoded as values, never bytes";
    // Only a change in the panel's SIZE can move it, and only the row count
    // changes that. Re-placing on every repaint would mean five forced layouts
    // per repaint (the panel's box, the cards' boxes, the wire's bbox) directly
    // after writing the rows — the read-after-write flush this file avoids
    // everywhere else.
    if (panelRows.childElementCount !== rowCount) placePanel();
  };

  /** Overlap area of two boxes, in px². Zero when they miss each other. */
  const overlap = (a: DOMRect, b: DOMRect): number =>
    Math.max(0, Math.min(a.right, b.right) - Math.max(a.left, b.left)) *
    Math.max(0, Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top));

  /**
   * Park the panel in the free quadrant, and run its tether to the wire.
   *
   * MEASURED, not placed: the stage's layout changes with the client count and
   * the window size, so the panel scores the four corners against what is
   * actually on the stage — every pane card, plus the selected wire's own
   * bounding box — and takes the one that covers the least. Distance to the
   * wire breaks ties, so among clear corners it sits near what it describes.
   */
  const placePanel = () => {
    if (!selected || panel.hidden) {
      tether.setAttribute("visibility", "hidden");
      return;
    }
    const edge = edges.get(selected);
    if (!edge) return;
    const origin = layer.getBoundingClientRect();
    if (origin.width === 0 || origin.height === 0) return;
    const size = panel.getBoundingClientRect();
    const hub = server();
    const obstacles = [...clients(), ...(hub ? [hub] : [])].map((node) =>
      toLocal(node.shell.getBoundingClientRect(), origin)
    );
    const bbox = edge.path.getBBox();
    obstacles.push(new DOMRect(bbox.x, bbox.y, bbox.width, bbox.height));
    const maxX = Math.max(PANEL_MARGIN, origin.width - size.width - PANEL_MARGIN);
    const maxY = Math.max(PANEL_MARGIN, origin.height - size.height - PANEL_MARGIN);
    let best: { x: number; y: number; score: number } | null = null;
    for (const x of [PANEL_MARGIN, maxX]) {
      for (const y of [PANEL_MARGIN, maxY]) {
        const box = new DOMRect(x, y, size.width, size.height);
        const covered = obstacles.reduce((total, other) => total + overlap(box, other), 0);
        const reach = Math.hypot(x + size.width / 2 - edge.mid.x, y + size.height / 2 - edge.mid.y);
        // Area dominates: being clear of the cards and the wire is the rule,
        // being near the wire is only the tie-break.
        const score = covered * 1000 + reach;
        if (!best || score < best.score) best = { x, y, score };
      }
    }
    if (!best) return;
    panel.style.left = `${best.x}px`;
    panel.style.top = `${best.y}px`;
    // The tether lands on the panel's nearest edge, so it never crosses the
    // panel to reach it.
    const anchor = {
      x: Math.max(best.x, Math.min(edge.mid.x, best.x + size.width)),
      y: Math.max(best.y, Math.min(edge.mid.y, best.y + size.height)),
    };
    tether.removeAttribute("visibility");
    tether.style.color = edge.node.color;
    tetherLine.setAttribute("d", `M${edge.mid.x},${edge.mid.y} L${anchor.x},${anchor.y}`);
    tetherNub.setAttribute("cx", String(edge.mid.x));
    tetherNub.setAttribute("cy", String(edge.mid.y));
  };

  (panel.querySelector(".mp-wl-x") as HTMLButtonElement).addEventListener("click", () =>
    selectEdge(null)
  );

  const spawn = (
    edge: Edge,
    dir: Direction,
    bytes: number,
    now: number,
    held: number | null = null
  ) => {
    // Both directions are circles; size scales MILDLY with the bytes the dot
    // carries, over a range that differs by direction (FULL_BYTES). The
    // client's own ink rides on `color` for intent; authority takes the
    // server's slate from the stylesheet, since every authority packet on
    // every wire comes from the one pane.
    const weight = Math.min(1, bytes / FULL_BYTES[dir]);
    const el = document.createElementNS(SVG_NS, "circle");
    el.setAttribute("r", (dir === "up" ? 2.6 + weight * 2 : 2 + weight * 0.8).toFixed(2));
    if (dir === "up") el.style.color = edge.node.color;
    // `replay` is the same shape and the same ink — it is the same traffic, read
    // from the log instead of from the live feed. The class exists so the
    // provenance is assertable (and stylable) rather than inferred.
    el.setAttribute("class", `mp-packet ${dir}${held === null ? "" : " replay"}`);
    if (dots.length >= MAX_DOTS) {
      const oldest = dots.shift();
      oldest?.el.remove();
    }
    packets.appendChild(el);
    dots.push({ el, edge, dir, start: now, held });
  };

  /**
   * Which wire a packet travelled, and which way — the one rule the live feed
   * and the replay share, so the two can never disagree about what a packet is.
   *
   * Null for a packet that is not traffic (the connected/disconnected lifecycle
   * events — the pane headers already say who is linked; dots are payload) or
   * whose wire is not on the stage.
   */
  const linkOf = (packet: Packet): { edge: Edge; dir: Direction } | null => {
    if (packet.kind !== "message") return null;
    const dir: Direction = packet.from === "server" ? "down" : "up";
    const edge = edges.get(dir === "up" ? packet.from : packet.to);
    return edge ? { edge, dir } : null;
  };

  /**
   * Draw the log's window at `frame`: one dot per edge per direction per frame
   * of recorded traffic, held at the position that frame's age gives it.
   *
   * The same batching rule the live feed uses (Addendum 5a.5), applied to the
   * record rather than to the moment — so a parked frame and the live frame it
   * was show the same picture.
   */
  const renderReplay = (frame: number, now: number) => {
    clearDots();
    replayFrame = frame;
    if (reduceMotion.matches) return;
    const batched = new Map<string, { edge: Edge; dir: Direction; bytes: number; age: number }>();
    // Straight to the playhead by binary search, then back one window — the
    // walk costs the window, never the session (same rule as viewportRows).
    const log = net.packets();
    for (let i = indexAfter(log, frame) - 1; i >= 0; i--) {
      const packet = log[i];
      if (packet.frame === null) break;
      const age = frame - packet.frame;
      if (age >= REPLAY_WINDOW_FRAMES) break;
      const link = linkOf(packet);
      if (!link) continue;
      const key = `${link.edge.node.id}|${link.dir}|${packet.frame}`;
      const carried = batched.get(key);
      if (carried) carried.bytes += packet.size;
      else batched.set(key, { edge: link.edge, dir: link.dir, bytes: packet.size, age });
    }
    for (const { edge, dir, bytes, age } of batched.values()) {
      spawn(edge, dir, bytes, now, age / REPLAY_WINDOW_FRAMES);
    }
  };

  const step = () => {
    if (!active) return;
    const now = performance.now();
    const seen = inbox;
    inbox = [];
    // PARKED: the traffic belongs to the timeline, so the rail decides what is
    // on the wires — the live feed is dropped (`seen` is discarded above) and
    // the window around the playhead is drawn from the coordinator's log
    // instead. Live/unparked keeps the per-frame batched feed below.
    const { parked, frame } = clock();
    const replay = parked && frame !== null;
    if (replay !== replaying) {
      replaying = replay;
      clearDots();
      replayFrame = null;
    }
    // A held playhead rebuilds nothing; the advance pass below still runs, so a
    // relayout re-places the held dots on the wires' new geometry.
    if (replay && frame !== replayFrame) renderReplay(frame, now);
    // BATCHED PER FRAME (Addendum 5a.5, retiring the old time sampling): one
    // dot per edge per direction per frame that carried traffic, sized by the
    // bytes that frame carried. A 60Hz session broadcasts a snapshot per client
    // per frame, so a dot per packet is a swarm and a time sample is a lie
    // about the rate; a dot per frame is exactly what the wire did, at the rate
    // the screen can show.
    if (!replay) {
      const batched = new Map<string, { edge: Edge; dir: Direction; bytes: number }>();
      for (const packet of seen) {
        const link = linkOf(packet);
        if (!link) continue;
        const { edge, dir } = link;
        edge.count += 1;
        // Reduced motion: no flying dots at all. The edge keeps a static packet
        // COUNT badge instead, so the traffic is still legible without animation.
        if (reduceMotion.matches) continue;
        const key = `${edge.node.id}|${dir}`;
        const carried = batched.get(key);
        if (carried) carried.bytes += packet.size;
        else batched.set(key, { edge, dir, bytes: packet.size });
      }
      for (const { edge, dir, bytes } of batched.values()) spawn(edge, dir, bytes, now);
      // The preference can flip while the view is open; whatever is still in the
      // air stops immediately rather than finishing its flight.
      if (reduceMotion.matches && dots.length > 0) clearDots();
    }
    // Read every position first, then write every transform: `getPointAtLength`
    // flushes pending layout, so interleaving it with the transform writes
    // would flush once PER DOT.
    const moved: { el: SVGElement; point: DOMPoint }[] = [];
    for (let i = dots.length - 1; i >= 0; i--) {
      const dot = dots[i];
      // A replayed dot is HELD where the playhead put it; a live one flies on
      // the host clock. Both then travel the same wire the same way.
      const t = dot.held ?? (now - dot.start) / FLIGHT_MS;
      if (t >= 1 || dot.edge.length === 0) {
        dot.el.remove();
        dots.splice(i, 1);
        continue;
      }
      // Authority travels the wire backwards: the path is drawn client → hub.
      const at = dot.dir === "up" ? t : 1 - t;
      moved.push({ el: dot.el, point: dot.edge.path.getPointAtLength(at * dot.edge.length) });
    }
    for (const { el, point } of moved) {
      el.setAttribute("transform", `translate(${point.x.toFixed(2)},${point.y.toFixed(2)})`);
    }
    paintChips();
    // The pinned log follows the same clock the dots do — live it tails, parked
    // it sits at the playhead. Both are change-guarded (`rowKey`).
    renderPanel();
  };

  const clearDots = () => {
    for (const dot of dots) dot.el.remove();
    dots.length = 0;
    inbox = [];
    // Nothing is drawn, so no frame is drawn FOR: the next parked step rebuilds
    // (renderReplay re-stamps this immediately after clearing).
    replayFrame = null;
  };

  const setActive = (on: boolean) => {
    if (on === active) return;
    active = on;
    if (!on) {
      clearDots();
      // The pin belongs to the view: leaving it and coming back should show the
      // topology, not a panel pinned to whatever was interesting minutes ago.
      selectEdge(null);
    } else relayout();
  };

  const unsubscribe = net.onPacket((packet) => {
    if (!active) return;
    if (inbox.length >= INBOX_CAP) inbox.shift();
    inbox.push(packet);
  });

  // The clean re-measure seam: the panes are sized by the grid, so whatever
  // changes their boxes (a window resize, the editor column dragging, a layout
  // switch, a client arriving) resizes this layer with them.
  const observer = new ResizeObserver(() => {
    if (active) relayout();
  });
  observer.observe(layer);

  return {
    relayout,
    step,
    setActive,
    resetCounts() {
      for (const edge of edges.values()) edge.count = 0;
      paintChips();
      // A new program is a new protocol: the pinned rows are the old one's.
      selectEdge(null);
    },
    selectEdge,
    destroy() {
      observer.disconnect();
      unsubscribe();
      clearDots();
      for (const [id, edge] of [...edges]) dropEdge(edge, id);
      layer.remove();
      chips.remove();
    },
  };
}
