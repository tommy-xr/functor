// THE WIRE TAB — the traffic monitor as a permanent tab in the sandbox's
// bottom panel, beside problems / output / executions (design:
// ~/notes/projects/functor/design-multiplayer-ide-frontend.md, Addendum 8.2 —
// "an in-editor wireshark").
//
// WHY A SECOND SURFACE. The pinned panel (mp-network.ts) is the IN-CONTEXT
// reading: it hangs off the wire you clicked, in the one view that draws wires,
// and it answers "what is crossing THIS link?". That is the wrong instrument
// for "the protocol did something odd four seconds ago" — a question you ask
// while looking at the editor, in whatever layout you were already in. So the
// monitor also lives where a monitor belongs: docked, always reachable, with a
// packet count on the tab, showing every link at once and filterable down.
//
// Both surfaces read the SAME rows (wire-rows.ts) against the SAME log
// (net-coordinator.ts) on the SAME clock — live they tail at 10Hz, parked they
// window around the playhead and mark the nearest row. A reader who learns one
// has learned the other; only the framing differs.
//
// THE SELECTION MODEL. One value, propagated one way. Pinning a wire in the
// network view focuses that link's chip here (mp-network's `onSelect`), so a
// single click aims both surfaces at the same link. UNPINNING is not
// propagated, because the two surfaces mean different things by "nothing
// selected": the panel means "no panel", and this tab means "every link" — and
// the panel unpins itself every time the network view closes, which must not
// silently widen a filter the reader chose here. Picking a chip here likewise
// stays here: this tab is the session-wide view, and reaching back to re-pin a
// panel in a view that may not even be open would be a change you did not make.
//
// DOM-imperative, like the view strip it sits beside: it repaints off the pane
// grid's rAF beside three running games, and routing a live packet count
// through React state would re-render the whole status bar (output list
// included) every frame. React mounts these two nodes and never looks inside
// them — see `status-bar-store.ts`'s `WireSlot`.

import type { NetCoordinator, Packet } from "./net-coordinator.js";
import type { WireSlot } from "./status-bar-store.js";
import {
  buildWireRow,
  inFlightAt,
  LIVE_ROW_MS,
  linkIdOf,
  logWindow,
  onLink,
  openTree,
  packetKey,
  shortName,
} from "./wire-rows.js";
import type { WireValue } from "./wire-value.js";

/**
 * Rows the monitor renders at once.
 *
 * A cap, because the log holds ten thousand packets and a snapshot protocol
 * fills it in under a minute: rendering all of it would rebuild ten thousand
 * rows ten times a second for a reader looking at eleven. The footer says the
 * number it is showing AND the number it has, so the cap is a stated bound
 * rather than a silent one — and the rail is how you reach the rest, which is
 * the whole point of a scrub-locked log.
 *
 * Sixty is five screenfuls of the 180px panel — enough scrollback to read back
 * through a burst — and the price is paid ten times a second in full: every row
 * is rebuilt AND its payload re-decoded on each repaint, so this number is a
 * `JSON.parse` budget as much as a DOM one. (At 780-byte snapshots, sixty rows
 * is ~47KB parsed per repaint; two hundred was three times that, for scrollback
 * that scrolls away while you read it.)
 */
const MAX_ROWS = 60;

/** Rows of context kept AFTER the playhead's row, so the parked frame is not
 * the last line in the list. */
const LEAD_ROWS = 8;

/** Which half of the conversation to show. `intent` is client → server, and
 * `authority` is the snapshot stream coming back. */
type DirFilter = "all" | "intent" | "authority";

const DIR_FILTERS: { id: DirFilter; label: string; title: string }[] = [
  { id: "all", label: "both", title: "Every packet on the selected links" },
  { id: "intent", label: "intent", title: "Client → server only" },
  { id: "authority", label: "authority", title: "Server → client only" },
];

/** A link the monitor can filter to: the client end of a client↔server pair,
 * and the ink that client is drawn in everywhere else. */
export interface WireLink {
  id: string;
  color: string;
}

export interface WireTabOptions {
  /** The status bar's two nodes: the tab's panel body and its count badge. */
  slot: WireSlot;
  net: NetCoordinator;
  /** The session's links, in pane order. Read live: clients come and go. */
  links: () => WireLink[];
  /** The session clock, in reference-clock frames — the same one the pinned
   * panel and the wires read (mp-panes' `sessionClock`). */
  clock: () => { parked: boolean; frame: number | null };
}

export interface WireTab {
  /** One frame: the badge always, the rows only while the panel is open. */
  step(): void;
  /** Focus a link's filter — what pinning a wire in the network view does. */
  focus(id: string): void;
  /** A new program is a new session: the count and the open row go with it. */
  reset(): void;
  destroy(): void;
}

export function initWireTab({ slot, net, links, clock }: WireTabOptions): WireTab {
  const root = document.createElement("div");
  root.className = "wire-tab";
  root.innerHTML = `
    <div class="wire-filters">
      <span class="wire-group" role="group" aria-label="Link filter"></span>
      <span class="wire-group wire-dirs" role="group" aria-label="Direction filter"></span>
    </div>
    <div class="wire-rows" role="log" aria-label="Wire traffic"></div>
    <footer class="wire-ft"></footer>`;
  const linkChips = root.querySelector(".wire-group") as HTMLElement;
  const dirChips = root.querySelector(".wire-dirs") as HTMLElement;
  const rowsHost = root.querySelector(".wire-rows") as HTMLElement;
  const foot = root.querySelector(".wire-ft") as HTMLElement;
  slot.panel.replaceChildren(root);

  /** The focused link, or null for ALL (every link interleaved by frame, with a
   * link column added). */
  let link: string | null = null;
  let dir: DirFilter = "all";
  /** The link chips currently built, as their ids — the rebuild guard. */
  let chipKey = "";
  /** What the rows currently say, so a steady session rewrites nothing. */
  let rowKey = "";
  /** When the rows last repainted — the live tail's throttle. */
  let lastPaint = 0;
  /**
   * What the last pass was built from: the parked frame (null while live) and
   * the log's length. PARKED there is no throttle — the panel must answer the
   * rail on the frame the playhead moves — so this is what keeps a held
   * playhead from re-walking the log sixty times a second for a window that
   * cannot have moved. `dirty` forces a pass through it after a change the log
   * cannot express (a filter, a disclosure).
   */
  let lastAt: number | null = null;
  let lastLength = -1;
  let dirty = true;
  /**
   * The row opened into a value tree: which packet (BY IDENTITY — the row hash
   * can genuinely collide, see mp-network's note), the tree's own DOM (kept
   * across repaints so opened nodes stay open), and, when it opened on a live
   * tail, the viewport to hold there. Opening a row IS the pause: a tail that
   * kept moving would scroll the row away before it could be read.
   */
  let opened: { packet: Packet; host: HTMLElement; held: Packet[] | null } | null = null;
  /** The packet whose payload button takes focus after the next repaint — a
   * repaint destroys the button the keyboard was on. */
  let refocus: Packet | null = null;
  /** Messages routed this session — the tab's badge. Counted on the way past
   * rather than by scanning the log, which is capped (and 10k long). */
  let routed = 0;
  let badgeText = "";
  let lastBadge = 0;
  /** Whether the panel was open on the previous frame — the reopen edge. */
  let wasOpen = false;

  const dirOf = (packet: Packet): DirFilter =>
    packet.from === "server" ? "authority" : "intent";

  const matches = (packet: Packet): boolean => {
    if (packet.kind !== "message") return false;
    if (link === null ? linkIdOf(packet) === null : !onLink(packet, link)) return false;
    return dir === "all" || dirOf(packet) === dir;
  };

  /** The chips on screen, each with the question "am I the current filter?" —
   * so a press can be reflected IN PLACE (see `syncChips`). */
  const chips: { button: HTMLButtonElement; on: () => boolean }[] = [];

  const chip = (
    label: string,
    on: () => boolean,
    title: string,
    pick: () => void
  ): HTMLButtonElement => {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "wire-chip";
    button.textContent = label;
    button.title = title;
    button.addEventListener("click", () => {
      pick();
      // A different filter is other traffic, so the open row is not in it — and
      // its HELD viewport is a frozen list from the OLD filter, which would
      // otherwise keep showing rows the pressed chip says are excluded.
      opened = null;
      // A filter change is a different list: repaint now rather than on the
      // tail's next tick, so the press and the rows agree in one turn.
      rowKey = "";
      lastPaint = 0;
      dirty = true;
      syncChips();
      render();
    });
    chips.push({ button, on });
    return button;
  };

  /**
   * Reflect the current filter on the chips that already exist.
   *
   * In place, never a rebuild: the reader may have pressed this chip with the
   * KEYBOARD, and replacing the button under them drops focus to `<body>` and
   * restarts Tab at the top of the document. (The rows solve the same problem
   * the other way — they must be rebuilt, so they carry `refocus`.)
   */
  const syncChips = () => {
    for (const one of chips) {
      one.button.setAttribute("aria-pressed", String(one.on()));
    }
  };

  /**
   * Rebuild the chip row when the SESSION's links change — a client arriving or
   * leaving. A mere filter press never gets here; `syncChips` handles that.
   */
  const buildChips = () => {
    const live = links();
    const key = live.map((one) => one.id).join(",");
    if (key === chipKey) return;
    chipKey = key;
    // A link that left the session cannot stay focused — its rows would freeze
    // on a client that is no longer here.
    if (link !== null && !live.some((one) => one.id === link)) link = null;
    chips.length = 0;
    // The ALL chip wears no ink dot: it stands for no one client.
    const all = chip("all links", () => link === null, "Every link, interleaved by frame", () => {
      link = null;
    });
    all.classList.add("all");
    linkChips.replaceChildren(
      all,
      ...live.map((one) => {
        const button = chip(
          `${shortName(one.id)} ↔ srv`,
          () => link === one.id,
          `Only traffic between ${one.id} and the authority`,
          () => {
            link = one.id;
          }
        );
        button.style.setProperty("--pc", one.color);
        return button;
      })
    );
    dirChips.replaceChildren(
      ...DIR_FILTERS.map((one) =>
        chip(one.label, () => dir === one.id, one.title, () => {
          dir = one.id;
        })
      )
    );
    syncChips();
  };

  /**
   * How many rows the filter matches in the whole log — the footer's honesty
   * about what the cap is hiding.
   *
   * The one pass that cannot avoid the whole log, so it runs only on a repaint
   * that is actually going to write the footer, never on the guard path.
   */
  const countMatches = (): number => {
    let total = 0;
    for (const packet of net.packets()) if (matches(packet)) total += 1;
    return total;
  };

  /** The monitor's viewport onto the log — the SAME scrub-lock the pinned panel
   * uses (`logWindow`), over this tab's filter instead of one link. */
  const viewport = (at: number | null) =>
    logWindow(net.packets(), at, { match: matches, back: MAX_ROWS, lead: LEAD_ROWS });

  /** Open a row into a value tree — or close the one that is open. One row at a
   * time: two open snapshots in a 180px panel is a scroll, not a reading. */
  const openRow = (packet: Packet, value: WireValue, shown: readonly Packet[]) => {
    if (opened?.packet === packet) {
      opened = null;
    } else {
      // No re-place callback: this panel is docked and its own height is fixed,
      // so a node opening inside the tree scrolls the list rather than moving
      // anything (unlike the pinned panel, which is positioned).
      opened = { packet, host: openTree(value), held: clock().parked ? null : [...shown] };
    }
    refocus = packet;
    rowKey = "";
    lastPaint = 0;
    dirty = true;
    render();
  };

  const render = () => {
    const { parked, frame } = clock();
    const at = parked && frame !== null ? frame : null;
    const length = net.packets().length;
    // Parked on the same frame with nothing new routed: the window cannot have
    // moved, and there is no throttle here to catch it (see `lastAt`).
    if (!dirty && at !== null && at === lastAt && length === lastLength) return;
    const now = performance.now();
    if (!dirty && at === null && now - lastPaint < LIVE_ROW_MS) return;
    dirty = false;
    lastAt = at;
    lastLength = length;
    const live = viewport(at);
    // An open row holds the LIVE viewport; parked, the rail keeps deciding the
    // window and the tree travels with its row until the row leaves it.
    const held = opened?.held;
    const shown = held && !parked ? held : live.rows;
    const { highlight } = live;
    if (opened && !shown.includes(opened.packet)) opened = null;
    // The change hash is the window's ENDS plus its length, not every row: at
    // two hundred rows, ten times a second, hashing the whole list costs more
    // than the rebuild it is there to avoid. Any shift of the window moves an
    // end (the log only ever grows at the tail, and a scrub moves both).
    // `at` is in the hash because which rows are IN FLIGHT changes as the
    // playhead crosses a delivery, even when the window has not moved.
    const key =
      `${link}|${dir}|${at}|${highlight}|${shown.length}|` +
      `${shown[0] ? packetKey(shown[0]) : ""}|${shown.at(-1) ? packetKey(shown.at(-1)!) : ""}|` +
      `${opened ? packetKey(opened.packet) : ""}`;
    if (key === rowKey) return;
    rowKey = key;
    lastPaint = now;

    const inks = new Map(links().map((one) => [one.id, one.color]));
    let anyPlain = false;
    let focusTarget: HTMLButtonElement | null = null;
    rowsHost.replaceChildren(
      ...shown.map((packet, index) => {
        const isOpen = opened?.packet === packet;
        const id = linkIdOf(packet);
        const built = buildWireRow({
          packet,
          at: index === highlight,
          inFlight: inFlightAt(packet, at),
          tree: isOpen && opened ? opened.host : null,
          // ALL interleaves several links, so each row says which one it
          // crossed and wears that client's ink; a focused link says it once,
          // on its chip, and keeps the narrower row.
          link: link === null && id ? shortName(id) : null,
          color: id ? inks.get(id) : undefined,
          onOpen: (value) => openRow(packet, value, shown),
        });
        if (!built.typed) anyPlain = true;
        if (refocus === packet) focusTarget = built.button;
        return built.el;
      })
    );
    if (shown.length === 0) {
      const empty = document.createElement("p");
      empty.className = "mp-wl-empty";
      empty.textContent = parked
        ? "nothing matching had crossed yet at this frame"
        : "no traffic matches this filter yet";
      rowsHost.appendChild(empty);
    }
    // Live, the newest row is the one being read, so the list stays pinned to
    // the bottom. Parked, the playhead's row is — and it can be anywhere in the
    // window, so centre the list on it. Written as scrollTop rather than
    // `scrollIntoView`, which also scrolls every ANCESTOR: this panel sits in
    // the page's chrome, and moving the page to show a log row is not something
    // the reader asked for. (`.wire-rows` is positioned, so a row's `offsetTop`
    // is its offset within the list.)
    const marked = rowsHost.children[highlight] as HTMLElement | undefined;
    if (parked && marked) {
      rowsHost.scrollTop = Math.max(
        0,
        marked.offsetTop - rowsHost.clientHeight / 2 + marked.offsetHeight / 2
      );
    } else if (!opened) {
      rowsHost.scrollTop = rowsHost.scrollHeight;
    }
    // The rebuild threw away the element the keyboard was on; put the focus on
    // its replacement, and only for the repaint the press caused. (The cast is
    // load-bearing: the assignment happens inside the map callback, which
    // control-flow analysis cannot see, so the checker narrows this to `never`.)
    if (focusTarget) (focusTarget as HTMLButtonElement).focus();
    refocus = null;
    // The footer says WHICH rows these are, not only how many: "the last N" is
    // true of a live tail and false of everything else — parked, the window is
    // centred on the playhead, and a held one is wherever the tail stopped. A
    // count that is right while the sentence around it is wrong is the kind of
    // honesty this panel exists to have.
    const total = countMatches();
    const showing =
      total <= shown.length
        ? `${total} row${total === 1 ? "" : "s"}`
        : at !== null
          ? `showing ${shown.length} of ${total} rows around #f ${at}`
          : opened
            ? `showing ${shown.length} of ${total} rows`
            : `showing the last ${shown.length} of ${total} rows`;
    foot.textContent = [
      showing,
      opened && !parked ? "the tail is held while a row is open" : null,
      anyPlain ? "Effect.send text — shown exactly as sent" : null,
    ]
      .filter(Boolean)
      .join(" · ");
  };

  /**
   * The tab's packet count.
   *
   * Change-guarded AND rate-limited: a snapshot protocol routes a packet per
   * client per frame, so "has it changed?" is true on every single frame — the
   * guard alone would still write the DOM sixty times a second for a number
   * nobody can read at that rate. It ticks at the tail's rate, like the rows.
   */
  const paintBadge = (now: number) => {
    const text = `${routed}`;
    if (text === badgeText || now - lastBadge < LIVE_ROW_MS) return;
    lastBadge = now;
    badgeText = text;
    slot.badge.textContent = text;
    slot.badge.title = `${routed} packets routed this session`;
  };

  const unsubscribe = net.onPacket((packet) => {
    if (packet.kind === "message") routed += 1;
  });

  return {
    step() {
      paintBadge(performance.now());
      // Closed, the tab costs a change-guarded badge write and nothing else —
      // no decode, no rows, no layout.
      if (!slot.open) {
        wasOpen = false;
        return;
      }
      // Reopening is a repaint even when nothing changed: `display:none` threw
      // the scroll box away, so the rows come back at the top — and PARKED
      // (nothing routing, the window fixed) no later frame would correct it.
      if (!wasOpen) {
        wasOpen = true;
        dirty = true;
      }
      buildChips();
      render();
    },
    focus(id) {
      if (link === id) return;
      link = id;
      opened = null;
      rowKey = "";
      lastPaint = 0;
      dirty = true;
      buildChips();
      syncChips();
      if (slot.open) render();
    },
    reset() {
      routed = 0;
      link = null;
      dir = "all";
      opened = null;
      refocus = null;
      rowKey = "";
      chipKey = "";
      lastPaint = 0;
      lastBadge = 0;
      dirty = true;
      paintBadge(performance.now());
    },
    destroy() {
      unsubscribe();
      slot.panel.replaceChildren();
      slot.badge.textContent = "";
      // The slot outlives this monitor (the store owns it), and React only
      // rewrites `open` when the bar re-renders — which a torn-down page may
      // never do. A stale `true` would have the NEXT monitor rendering into a
      // panel nobody opened.
      slot.open = false;
    },
  };
}
