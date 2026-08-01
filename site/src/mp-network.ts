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
// The wire log (click an edge → a pinned, scrub-locked panel) is the NEXT PR.

import type { NetCoordinator, Packet } from "./net-coordinator.js";

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
  destroy(): void;
}

type Direction = "up" | "down";

interface Edge {
  node: NetworkNode;
  path: SVGPathElement;
  chip: HTMLElement;
  chipLabel: HTMLElement;
  chipCount: HTMLElement;
  length: number;
  count: number;
}

interface Dot {
  el: SVGElement;
  edge: Edge;
  dir: Direction;
  start: number;
}

interface Point {
  x: number;
  y: number;
}

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
  svg.append(wires, packets);
  layer.appendChild(svg);

  const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
  const edges = new Map<string, Edge>();
  const dots: Dot[] = [];
  /** Packets seen since the last frame. Routing must stay cheap, so the
   * listener only buffers — every DOM write happens in `step`. */
  let inbox: Packet[] = [];
  let active = false;

  const addEdge = (node: NetworkNode): Edge => {
    const path = document.createElementNS(SVG_NS, "path");
    path.setAttribute("class", "mp-wire");
    // The player color rides on `color`, so the wire's stroke and the dot's
    // fill and glow can all be one `currentColor` in the stylesheet.
    path.style.color = node.color;
    wires.appendChild(path);
    const chip = document.createElement("span");
    chip.className = "mp-edge-chip";
    chip.innerHTML = `<b></b><i></i>`;
    chips.appendChild(chip);
    const edge: Edge = {
      node,
      path,
      chip,
      chipLabel: chip.querySelector("b")!,
      chipCount: chip.querySelector("i")!,
      length: 0,
      count: 0,
    };
    edges.set(node.id, edge);
    return edge;
  };

  const dropEdge = (edge: Edge, id: string) => {
    edge.path.remove();
    edge.chip.remove();
    edges.delete(id);
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
      const box = node.shell.getBoundingClientRect();
      return {
        left: box.left - origin.left,
        top: box.top - origin.top,
        width: box.width,
        height: box.height,
        cx: box.left - origin.left + box.width / 2,
        cy: box.top - origin.top + box.height / 2,
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
      edge.path.setAttribute(
        "d",
        `M${from.x},${from.y} C${c0.x},${c0.y} ${c1.x},${c1.y} ${to.x},${to.y}`
      );
      const mid = midpoint(from, c0, c1, to);
      edge.chip.style.left = `${mid.x}px`;
      edge.chip.style.top = `${mid.y}px`;
      written.push(edge);
    }
    // Lengths last: `getTotalLength` is a geometry READ, and interleaving it
    // with the `d` writes above would flush per edge instead of once.
    for (const edge of written) edge.length = edge.path.getTotalLength();
    paintChips();
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
          `recorded, not applied yet, so every packet crosses at the same pace.`;
      }
      const count = `${edge.count} pkt`;
      if (edge.chipCount.textContent !== count) edge.chipCount.textContent = count;
    }
  };

  const spawn = (edge: Edge, dir: Direction, bytes: number, now: number) => {
    // Both directions are circles; size scales MILDLY with the bytes the dot
    // carries, over a range that differs by direction (FULL_BYTES). The
    // client's own ink rides on `color` for intent; authority takes the
    // server's slate from the stylesheet, since every authority packet on
    // every wire comes from the one pane.
    const weight = Math.min(1, bytes / FULL_BYTES[dir]);
    const el = document.createElementNS(SVG_NS, "circle");
    el.setAttribute("r", (dir === "up" ? 2.6 + weight * 2 : 2 + weight * 0.8).toFixed(2));
    if (dir === "up") el.style.color = edge.node.color;
    el.setAttribute("class", `mp-packet ${dir}`);
    if (dots.length >= MAX_DOTS) {
      const oldest = dots.shift();
      oldest?.el.remove();
    }
    packets.appendChild(el);
    dots.push({ el, edge, dir, start: now });
  };

  const step = () => {
    if (!active) return;
    const now = performance.now();
    const seen = inbox;
    inbox = [];
    // BATCHED PER FRAME (Addendum 5a.5, retiring the old time sampling): one
    // dot per edge per direction per frame that carried traffic, sized by the
    // bytes that frame carried. A 60Hz session broadcasts a snapshot per client
    // per frame, so a dot per packet is a swarm and a time sample is a lie
    // about the rate; a dot per frame is exactly what the wire did, at the rate
    // the screen can show. (Traffic keyed to the TIMELINE — replaying the
    // scrubbed window's packets — arrives with the wire-log PR.)
    const batched = new Map<string, { edge: Edge; dir: Direction; bytes: number }>();
    for (const packet of seen) {
      // Lifecycle events (connected/disconnected) are not traffic — the pane
      // headers already say who is linked. Dots are payload.
      if (packet.kind !== "message") continue;
      const dir: Direction = packet.from === "server" ? "down" : "up";
      const edge = edges.get(dir === "up" ? packet.from : packet.to);
      if (!edge) continue;
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
    // Read every position first, then write every transform: `getPointAtLength`
    // flushes pending layout, so interleaving it with the transform writes
    // would flush once PER DOT.
    const moved: { el: SVGElement; point: DOMPoint }[] = [];
    for (let i = dots.length - 1; i >= 0; i--) {
      const dot = dots[i];
      const t = (now - dot.start) / FLIGHT_MS;
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
  };

  const clearDots = () => {
    for (const dot of dots) dot.el.remove();
    dots.length = 0;
    inbox = [];
  };

  const setActive = (on: boolean) => {
    if (on === active) return;
    active = on;
    if (!on) clearDots();
    else relayout();
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
    },
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
