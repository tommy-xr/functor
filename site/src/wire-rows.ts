// The ROW GRAMMAR of the traffic monitor: `#f · direction · payload · bytes`,
// and the two log walks that decide which packets a viewport holds.
//
// It exists because the same grammar is now read in two places — the pinned
// panel tethered to a wire in the network view (mp-network.ts) and the wire tab
// in the bottom panel (wire-tab.ts) — and a reader who learns to scan one must
// not have to re-learn the other. One builder, one set of class names, one
// stylesheet: the two surfaces differ in what they SHOW (a window on one link
// versus a filterable session-wide monitor), never in how a row reads.
//
// DOM-imperative, like both of its hosts: these rows are rebuilt on a throttled
// repaint beside three running games, and a row's only state (the open tree) is
// held by the host that owns the disclosure.

import type { Packet } from "./net-coordinator.js";
import { hasTree, mountValueTree } from "./value-tree.js";
import { decodeWire, fullWire, type WireValue } from "./wire-value.js";

/**
 * How often a LIVE tail repaints, in ms — shared, because both surfaces make
 * the same promise about it.
 *
 * A tail that turns over sixty times a second is not a log, it is a blur — and
 * each repaint decodes a screenful of payloads and rebuilds the rows, on a page
 * already running three games. Ten a second reads as live and costs a sixth as
 * much. PARKED there is no throttle at all: the rows must answer the rail on
 * the frame the playhead moves.
 */
export const LIVE_ROW_MS = 100;

/** `client 2` → `c2`, `server` → `srv`: a direction column has to fit. */
export const shortName = (id: string): string =>
  id === "server" ? "srv" : id.startsWith("client ") ? `c${id.slice(7)}` : id;

/**
 * Is this packet traffic on `id`'s link?
 *
 * A link is a PANE PAIR, not a connection id: a game that opened two
 * connections to the same authority would interleave both here. Nothing in the
 * samples does, and the pane pair is what the wire on screen represents.
 */
export const onLink = (packet: Packet, id: string): boolean =>
  packet.kind === "message" &&
  ((packet.from === id && packet.to === "server") ||
    (packet.from === "server" && packet.to === id));

/** Which link a packet crossed — the client end of the pair, whichever way it
 * went. Null for anything that is not client↔server message traffic. */
export const linkIdOf = (packet: Packet): string | null => {
  if (packet.kind !== "message") return null;
  if (packet.from === "server") return packet.to;
  if (packet.to === "server") return packet.from;
  return null;
};

/**
 * The first index in the log whose frame is after `at` — the playhead's place
 * in the record, by binary search.
 *
 * The log is sorted by frame because the reference clock only ever advances; an
 * unframed packet (the boot handshake, routed before the reference pane had
 * recorded a frame) sorts before everything, which is also where it belongs on
 * the timeline.
 */
export const indexAfter = (log: readonly Packet[], at: number): number => {
  let lo = 0;
  let hi = log.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if ((log[mid].frame ?? -Infinity) <= at) lo = mid + 1;
    else hi = mid;
  }
  return lo;
};

/** A row's cache key — the fields that make two rows different. Identity is the
 * `Packet` itself (the hosts hold the open row by reference); this is only for
 * the change hash that decides whether a repaint has anything to say. */
export const packetKey = (packet: Packet): string =>
  `${packet.frame}:${packet.from}:${packet.size}:${packet.at}`;

/**
 * The scrub-lock itself: a window of matching packets around `at`, and which of
 * its rows the playhead is on (-1 while live).
 *
 * Parked, the window ends `lead` matching rows PAST the playhead so the parked
 * frame is not the last line on screen, and the rest of it is the `back` rows
 * before — so scrubbing sweeps the list exactly as running it would. Live
 * (`at === null`) it is simply the newest `back` rows.
 *
 * Costs a screenful, not a session: the log is ordered by frame (the reference
 * clock only advances), so the playhead's place is found by BINARY SEARCH and
 * both walks start there. A linear scan from the end would traverse all ten
 * thousand packets every time the rail is parked in the past — exactly the
 * state this feature exists for.
 */
export const logWindow = (
  log: readonly Packet[],
  at: number | null,
  { match, back, lead }: { match: (packet: Packet) => boolean; back: number; lead: number }
): { rows: Packet[]; highlight: number } => {
  const before: Packet[] = [];
  const after: Packet[] = [];
  // The lead rows first, walking FORWARD from the playhead: the ones nearest
  // it, not the newest in the session. They also decide how many rows are left
  // for the walk backwards, which is why they are collected first.
  const pivot = at === null ? log.length : indexAfter(log, at);
  for (let i = pivot; i < log.length && after.length < lead; i++) {
    if (match(log[i])) after.push(log[i]);
  }
  for (let i = pivot - 1; i >= 0 && before.length < back - after.length; i--) {
    if (match(log[i])) before.push(log[i]);
  }
  before.reverse();
  const highlight = at === null || before.length === 0 ? -1 : before.length - 1;
  return { rows: [...before, ...after], highlight };
};

/**
 * A value tree ready to hang under its row (`value-tree.ts`).
 *
 * Shared because the details are the ones that must not drift between the two
 * surfaces: the rows are a `role="log"` live region, and a tree opening inside
 * one would otherwise be read out IN FULL on every repaint that re-parents it.
 * It is a thing to explore, not an announcement.
 *
 * `onResize` fires when the tree's height changes — a host that positions
 * itself against it (the pinned panel) re-places from there.
 */
export const openTree = (value: WireValue, onResize: () => void = () => {}): HTMLElement => {
  const host = document.createElement("div");
  host.className = "mp-wl-tree";
  host.setAttribute("aria-live", "off");
  mountValueTree(host, value, onResize);
  return host;
};

export interface WireRowOptions {
  packet: Packet;
  /** The playhead's row, while the session is parked. */
  at?: boolean;
  /** The open row's value tree, moved in by its host — present only on the one
   * row that is open. */
  tree?: HTMLElement | null;
  /** The link this packet crossed (`c2`), for a list that interleaves several.
   * Omitted by a single-link viewport, whose whole panel says the link once. */
  link?: string | null;
  /** The ink the row's direction cell takes (a client's player color). Omitted
   * leaves the host's own `--pc`. */
  color?: string | undefined;
  /** Opens the payload. Absent — or a payload with nothing to disclose — leaves
   * the cell a plain span rather than a control that reveals the line you are
   * already looking at. */
  onOpen?: (value: WireValue) => void;
}

export interface WireRow {
  el: HTMLElement;
  /** The payload's disclosure, when it has one — what a host re-focuses after a
   * repaint destroys the button the keyboard was on. */
  button: HTMLButtonElement | null;
  /** False for a plain `Effect.send` text payload: the footer says so. */
  typed: boolean;
}

/**
 * One row of traffic.
 *
 * textContent, never innerHTML: every cell here is a running game's own data.
 */
export const buildWireRow = ({
  packet,
  at = false,
  tree = null,
  link = null,
  color,
  onOpen,
}: WireRowOptions): WireRow => {
  const up = packet.from !== "server";
  const decoded = decodeWire(packet.text);
  const open = tree !== null;
  const row = document.createElement("div");
  row.className = `mp-wl${link === null ? "" : " linked"}${at ? " at" : ""}${open ? " open" : ""}`;
  if (color) row.style.setProperty("--pc", color);
  // The full rendering is built ON HOVER, once: rendering every row's whole
  // payload unelided (a world snapshot, per row, per repaint) to fill a tooltip
  // nobody has asked for yet is the most expensive thing this list could do. An
  // OPEN row skips it: the tooltip would cover the tree that already says the
  // same thing, better.
  if (!open) {
    row.addEventListener(
      "mouseenter",
      () => {
        row.title = fullWire(packet.text);
      },
      { once: true }
    );
  }
  const cell = (className: string, text: string): HTMLSpanElement => {
    const el = document.createElement("span");
    el.className = className;
    el.textContent = text;
    return el;
  };
  const cells: HTMLElement[] = [cell("f", `#f ${packet.frame ?? "—"}`)];
  if (link !== null) cells.push(cell("l", link));
  cells.push(
    cell(`d ${up ? "up" : "dn"}`, up ? `${shortName(packet.from)}→srv` : `srv→${shortName(packet.to)}`)
  );

  // The payload cell is the disclosure control when there is something to
  // disclose: a typed payload WITH structure, whose whole value the page already
  // holds (the frame is decoded client-side, so opening it costs nothing but the
  // DOM). A bare `Welcome(2)`, which the row already shows completely, stays a
  // plain cell.
  const value = decoded.value && onOpen && hasTree(decoded.value) ? decoded.value : null;
  let payload: HTMLElement;
  let button: HTMLButtonElement | null = null;
  if (value && onOpen) {
    button = document.createElement("button");
    button.type = "button";
    button.setAttribute("aria-expanded", String(open));
    button.addEventListener("click", () => onOpen(value));
    payload = button;
  } else {
    payload = document.createElement("span");
  }
  payload.className = "payload";
  if (decoded.head) {
    const ctor = document.createElement("b");
    ctor.textContent = decoded.head;
    const args = document.createElement("em");
    args.textContent = decoded.body;
    payload.append(ctor, args);
  } else {
    payload.textContent = decoded.body;
  }
  cells.push(payload, cell("n", `${packet.size} B`));
  row.append(...cells);
  // The tree is the SAME element across repaints, moved into the rebuilt row:
  // every node the reader opened inside it stays open, because its state never
  // left the DOM.
  if (tree) row.append(tree);
  return { el: row, button, typed: decoded.typed };
};
