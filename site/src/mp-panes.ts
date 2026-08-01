// Multiplayer pane-grid PROTOTYPE for the sandbox (design:
// ~/notes/projects/functor/design-multiplayer-ide-frontend.md — this is the
// LIVE-mode skeleton of Addendum 2: hard per-client boundaries, iframe panes).
//
// The preview column becomes:
//
//   ┌ chrono bar ─ ⏸ ⏭ ══rail══╪═ 🔮 | ⊞ tiled ▦ grid ▤ tabs ┐ (game side only —
//   ├ pane grid ──────────────────────────────────────────────┤    the editor keeps
//   │  ╔ 1 client · cyan  ⇅ Wi-Fi ▾   #f 841 ● ╗  ┌ 2 client ┐│    its full height)
//   │  ║             iframe                    ║  │  iframe   ││
//   └──╚══════════════════════════════════════╝──└───────────┘┘
//
// Each pane is a real `player.html` iframe running the same program — N
// independent sims, hot-reloaded together with each pane's model preserved.
// Panes grow/shrink LIVE (pane 1 wraps the page's #player once and never
// moves again, so its model survives every count change).
//
// Prototype seams (all same-origin, so no postMessage transport yet):
//  - the host chrono bar drives every pane through its `window.__scrub` seam
//    (pause/step/seek/extrapolate broadcast; rail + markers mirror the
//    focused pane);
//  - every pane (the sandbox player included) boots with `?scrubber=hidden`
//    — the seam without the bar — and the chrono bar is the ONE transport
//    instrument at every client count, single included;
//  - the per-pane link chip (LAN / Wi-Fi / mobile / awful) records a
//    LinkProfile per client but impairs nothing until the transport carries
//    it. It is labelled as such.
//
// This module is imperative DOM on purpose: it owns the preview column
// OUTSIDE the React islands (like CodeMirror owns #editor), and publishes to
// the page through the injected `setPill` / `statusBar` seams.

import { PlayerBridge } from "./player-bridge.js";
import { asPlayerMessage } from "./protocol.js";
import type { StatusBar } from "./status-bar-store.js";
import type { PillState } from "./components/StatusPill.js";
import { NetCoordinator } from "./net-coordinator.js";
import { initNetworkGraph, type NetworkNode } from "./mp-network.js";

/** The shared scrubber's `window.__scrub` seam (runtime/functor-runtime-web/scrubber.js). */
interface ScrubSeam {
  paused(): boolean;
  frame(): number;
  /** The recorded window `[lo, hi]`, empty until anything is recorded. `hi` is
   * the pane's LIVE HEAD — the newest frame it has simulated — independent of
   * where its scrubber is currently parked (which is what `frame()` reports). */
  range(): number[];
  seek(frame: number): void;
  togglePause(): void;
  canDetach(): boolean;
  detached(): boolean;
  detachedGeneration(): number;
  ownsDetachedInput(): boolean;
  setDetachedPointerLockPending(pending: boolean): void;
  toggleDetached(): void;
  debugCamera(): {
    mode: number;
    material: number;
    physics: boolean;
    authoredFrustum: boolean;
    gameUi: boolean;
    fov: number;
    zoom2d: number;
  };
  setDebugCamera(settings: {
    mode?: number;
    fov?: number;
    material?: number;
    physics?: boolean;
    authoredFrustum?: boolean;
    gameUi?: boolean;
    reset?: boolean;
  }): void;
  step(): void;
  model(): { preview: { enabled: boolean; seconds: number; rate: number } };
  selectEvent(id: number | null): void;
  view(): TimelineView | null;
  setPreview(preview: {
    enabled?: boolean;
    seconds?: number;
    rate?: number;
    /** 1 trail / 2 strobe / 3 both (scrubber.js seam extension). */
    mode?: number;
    /** Pin the overlay's fade (0..1); negative resumes the runtime's easing. */
    presence?: number;
  }): void;
}

/** The slice of timeline-model's derived view the chrono bar renders. */
interface TimelineView {
  viewport: { lo: number; hi: number };
  recorded: { lo: number; hi: number };
  recordingAvailable: boolean;
  playheadUnit: number;
  playheadClippedBefore: boolean;
  playheadClippedAfter: boolean;
  recordedStartUnit: number;
  recordedEndUnit: number;
  hasUnavailableHistory: boolean;
  unavailableEndUnit: number;
  unavailableAfterStartUnit: number;
  previewEndUnit: number;
  previewFrames: number;
  previewClippedFrames: number;
  selectedFrame: number;
  paused: boolean;
  eventMarkers: {
    id: number;
    unit: number;
    frame: number;
    category: string;
    kind: string;
    labels: string[];
    count: number;
  }[];
}

interface LinkProfile {
  name: string;
  ms: number;
  jitter: number;
  loss: number;
}

type PaneState = PillState["state"];

/**
 * How the panes are laid out. `tiled` is one row of equal tiles; `grid` wraps
 * the CLIENTS into two columns and gives the server its own full-width strip
 * along the bottom (design-multiplayer-ide-frontend.md §5 — the authority reads
 * differently from the players); `network` puts the server in the middle as the
 * hub with its clients around it and draws the wires between them (Addendum 5);
 * `tabs` shows one pane at a time.
 *
 * Declaration order is the segmented control's order AND the `f` key's cycle.
 */
const VIEWS = ["tiled", "grid", "network", "tabs"] as const;
type PaneView = (typeof VIEWS)[number];

/**
 * What a pane IS in the session. A `client` pane is a player: numbered, player
 * colored, focusable, and the thing the CLIENTS control counts. The `server`
 * pane is the authority: one per session, unnumbered, subdued, and outside the
 * client count — see the server-pane block further down for the full contract.
 */
type PaneRole = "client" | "server";

interface Pane {
  role: PaneRole;
  /** The pane's routing identity in the coordinator ("client 2", "server"). */
  id: string;
  /** The pane's ink — a player color, or the server's slate. */
  color: string;
  /** Aborts every listener this pane registered outside its own subtree. */
  scope: AbortController;
  iframe: HTMLIFrameElement;
  shell: HTMLElement;
  tab: HTMLButtonElement;
  state: PaneState;
  link: LinkProfile;
  frameLabels: HTMLElement[];
  dots: HTMLElement[];
  conn: HTMLElement;
  /** The link indicator's two halves. The DOT survives every header clip (it
   * is the pane's whole connection state at a glance); the WORD goes with the
   * rest of the prose when the header is a thumbnail's. */
  connDot: HTMLElement;
  connText: HTMLElement;
  /** This pane's frame readout on the network status strip, while that strip
   * exists (rebuilt whenever the pane set changes). */
  stripFrame: HTMLElement | null;
  errStrip: HTMLElement;
}

export interface MultiplayerPanesOptions {
  frame: HTMLIFrameElement;
  count: number;
  statusBar: StatusBar;
  /** Writes the header pill (the page's pill store). */
  setPill: (state: PaneState, text: string, detail: string) => void;
  /** The current editor buffer, so a mirror added mid-session catches up. */
  getSource: () => string;
  /**
   * Ask the PAGE for a client count — the "+" tile's one job. It routes through
   * the host rather than calling `setCount` directly so that the spatial
   * affordance and the CLIENTS dropdown are the same path: the same clamp, the
   * same `#clients=` hash write, the same control re-render.
   */
  requestCount: (n: number) => void;
}

export interface MultiplayerPanes {
  /**
   * Load a fresh program into every pane. `serverSrc` is the player URL of the
   * example's SERVER role (examples.ts `server`), or null for a sample that
   * declares none — passing it mounts the server pane, passing null removes it.
   */
  setSrc(src: string, serverSrc?: string | null): void;
  push(source: string): void;
  reset(): void;
  aggregateStatus(state: PaneState, text: string, detail: string): void;
  setCount(n: number): void;
  count(): number;
  destroy(): void;
}

const PLAYER_COLORS = ["var(--scrub-p1)", "var(--scrub-p2)", "var(--scrub-p3)", "var(--scrub-p4)"];

/** One clamp for every entry point (the select offers the same range). */
export const MAX_CLIENTS = 3;

/** The timeline's fixed tick rate (timeline-model.js TIMELINE_FPS). */
const TIMELINE_FPS = 60;

const LINK_PRESETS: LinkProfile[] = [
  { name: "LAN", ms: 8, jitter: 2, loss: 0 },
  { name: "Wi-Fi", ms: 45, jitter: 12, loss: 1.2 },
  { name: "mobile", ms: 132, jitter: 38, loss: 3.4 },
  { name: "awful", ms: 400, jitter: 120, loss: 12 },
];

const seamOf = (iframe: HTMLIFrameElement): ScrubSeam | null => {
  try {
    return (iframe.contentWindow as (Window & { __scrub?: ScrubSeam }) | null)?.__scrub ?? null;
  } catch {
    return null;
  }
};

const exitPanePointerLock = (iframe: HTMLIFrameElement) => {
  try {
    iframe.contentDocument?.exitPointerLock();
  } catch {
    // A navigated or removed iframe has already lost pointer lock.
  }
};

// The chrono bar lives in the HOST document. Clicking its camera button leaves
// keyboard focus there even after pointer lock moves relative mouse events to
// a pane's canvas, so WASD/QE would keep going to the editor/host instead of
// the player's window. Explicitly focus the accepted pane and a programmatic
// canvas focus target; direct player/IDE activation already gets this naturally
// because its camera button lives inside the player document.
const focusPaneCameraInput = (iframe: HTMLIFrameElement, canvas: HTMLCanvasElement) => {
  try {
    iframe.contentWindow?.focus();
    if (!canvas.hasAttribute("tabindex")) canvas.tabIndex = -1;
    canvas.focus({ preventScroll: true });
  } catch {
    // A pane can navigate between pointer-lock acceptance and focus transfer.
  }
};

export function initMultiplayerPanes({
  frame,
  count,
  statusBar,
  setPill,
  getSource,
  requestCount,
}: MultiplayerPanesOptions): MultiplayerPanes {
  const previewPane = frame.closest(".preview-pane") as HTMLElement;
  previewPane.classList.add("mp");

  // ------------------------------------------------------------- chrono bar
  const chrono = document.createElement("div");
  chrono.className = "mp-chrono";
  chrono.innerHTML = `
    <button class="mp-sbtn" id="mp-pause" title="Pause / resume every client">⏸</button>
    <button class="mp-sbtn" id="mp-step" title="Step every client one frame">⏭</button>
    <span class="mp-rail" id="mp-rail" title="Drag to seek every client">
      <span class="mp-track"></span>
      <span class="mp-unavailable" id="mp-unavailable"></span>
      <span class="mp-unavailable mp-unavailable-after" id="mp-unavailable-after"></span>
      <span class="mp-recorded" id="mp-recorded"></span>
      <span class="mp-played" id="mp-played"></span>
      <span class="mp-future" id="mp-future"></span>
      <span class="mp-ticks" id="mp-ticks"></span>
      <span class="mp-markers" id="mp-markers"></span>
      <span class="mp-playhead" id="mp-playhead" role="slider" tabindex="0"
        aria-label="Selected frame" aria-orientation="horizontal"></span>
      <span class="mp-preview-handle" id="mp-preview-handle" role="slider" tabindex="0"
        aria-label="Extrapolation endpoint" aria-orientation="horizontal"
        title="Drag to stretch the extrapolation window"></span>
      <span class="mp-overflow" id="mp-overflow"></span>
      <span class="mp-evt-tip" id="mp-evt-tip" role="status"></span>
      <span class="mp-rail-label"><b>#f</b> <span id="mp-frame">—</span></span>
    </span>
    <!-- Right of the rail: the two ways to LOOK at the parked frame — the
         scrubber's own grammar, so both bars read the same. -->
    <button class="mp-sbtn" id="mp-camera" title="Open the focused debug camera" hidden>📷</button>
    <button class="mp-sbtn" id="mp-extrap" title="Extrapolate: speculatively simulate forward from the parked frame">🔮</button>
    <span class="mp-viewseg" role="group" aria-label="Pane layout">
      <button id="mp-view-tiled" aria-pressed="true" title="Tiled: every pane in one row (f cycles)">⊞ tiled</button>
      <button id="mp-view-grid" aria-pressed="false" title="Grid: clients in two columns over the server strip (f cycles)">▦ grid</button>
      <button id="mp-view-network" aria-pressed="false">◈ network</button>
      <button id="mp-view-tabs" aria-pressed="false" title="Tabs: one pane full-size (f cycles)">▤ tabs</button>
    </span>`;

  const debugDrawer = document.createElement("section");
  debugDrawer.className = "mp-debug";
  debugDrawer.hidden = true;
  debugDrawer.setAttribute("aria-label", "Debug Camera controls");
  debugDrawer.innerHTML = `
    <strong>Debug Camera</strong>
    <label>View
      <select id="mp-debug-mode">
        <option value="0">FPS</option>
        <option value="1">Orbit</option>
      </select>
      <span id="mp-debug-pan2d" hidden>Pan 2D</span>
    </label>
    <label><span id="mp-debug-lens-name">FOV</span>
      <input id="mp-debug-fov" type="range" min="15" max="120" step="1" value="60">
      <output id="mp-debug-lens">60°</output>
    </label>
    <label id="mp-debug-material-label">Material
      <select id="mp-debug-material">
        <option value="0">Shaded</option>
        <option value="3">Transparent</option>
        <option value="1">Normals</option>
        <option value="2">Tangents</option>
      </select>
    </label>
    <label id="mp-debug-physics-label"><input id="mp-debug-physics" type="checkbox"> Physics</label>
    <label id="mp-debug-frustum-label"><input id="mp-debug-frustum" type="checkbox"> Authored frustum</label>
    <label><input id="mp-debug-game-ui" type="checkbox" checked> Game UI</label>
    <button id="mp-debug-reset" type="button">Reset</button>`;

  // -------------------------------------------------------------- pane grid
  const tabsStrip = document.createElement("div");
  tabsStrip.className = "mp-tabs";
  tabsStrip.hidden = true;

  const grid = document.createElement("div");
  grid.className = "mp-grid";
  grid.dataset.view = "tiled";

  // The "+" tile (Addendum 5a.7): the SPATIAL way to add a client, sitting
  // where the client it would create is about to appear — the CLIENTS dropdown
  // stays the numeric one, and both go through the same host call. It is a real
  // <button>, so it is tab-reachable and answers to Enter/Space for free; it is
  // a grid child so it takes a cell in tiled/grid, and CSS hides it in tabs
  // (which has its own "+" tab) and in network. NETWORK gets no ghost spoke: a
  // wire is measured between two panes that are actually talking, and drawing
  // one to a card that is not a participant would be the one lie this view
  // cannot afford.
  const addTile = document.createElement("button");
  addTile.type = "button";
  addTile.className = "mp-add-tile";
  addTile.title = "Add a client";
  addTile.innerHTML = `<span class="mp-add-plus" aria-hidden="true">+</span>
    <span class="mp-add-label">add a client</span>`;
  grid.appendChild(addTile);

  const addTab = document.createElement("button");
  addTab.type = "button";
  addTab.className = "mp-tab add";
  addTab.title = "Add a client";
  addTab.textContent = "+";
  addTab.setAttribute("aria-label", "Add a client");
  tabsStrip.appendChild(addTab);

  // The stage holds the grid plus the layers the NETWORK view adds (mounted by
  // the graph itself: the wires under the cards, the chips over them). They are
  // siblings of the grid, never children of it — the grid's cells are the panes
  // and the "+" tile, and another child would shift the `:nth-child` rules the
  // grid and network views lay out with.
  const stage = document.createElement("div");
  stage.className = "mp-stage";
  stage.dataset.view = "tiled";
  stage.append(grid);

  // The NETWORK view's status strip (Addendum 5a.3), homed in the status bar's
  // view slot: the packet legend, plus the pane readouts the headers drop when
  // they clip to thumbnail width. A thumbnail header has room for WHO and
  // WHETHER IT IS LINKED and nothing else, so the frame counters live here —
  // one place, aligned, instead of spilling out of four narrow headers.
  const viewStrip = statusBar.viewStrip;
  viewStrip.hidden = true;
  viewStrip.innerHTML = `
    <span class="mp-vs-legend">
      <span class="motion"><span class="mp-vs-inks"></span><b>intent</b> — client → server</span>
      <span class="motion"><i class="mp-sw down"></i><b>authority</b> — server → client</span>
      <span class="motion mp-vs-note">one dot per link per direction per frame · size = bytes carried</span>
      <span class="still mp-vs-note">reduced motion: each link shows its packet count instead of moving dots</span>
    </span>
    <span class="mp-vs-panes" title="Each pane's OWN frame count, which is boot-relative — a late-joined client counts from 0. The rail above counts the reference clock."></span>`;
  const stripPanes = viewStrip.querySelector(".mp-vs-panes") as HTMLElement;
  // The intent swatch is one dot PER CLIENT, in that client's ink: "color
  // carries identity" is the rule the dots follow, and a single swatch would
  // have to pick one player's color to stand for all of them — which is the
  // cyan-means-player-1 confusion 5a.1 retired the cyan square for.
  const stripInks = viewStrip.querySelector(".mp-vs-inks") as HTMLElement;

  previewPane.prepend(chrono, debugDrawer, tabsStrip);
  previewPane.appendChild(stage);

  const $ = (id: string) => chrono.querySelector(`#${id}`) as HTMLElement;
  const $btn = (id: string) => chrono.querySelector(`#${id}`) as HTMLButtonElement;
  const debugMode = debugDrawer.querySelector("#mp-debug-mode") as HTMLSelectElement;
  const debugPan2d = debugDrawer.querySelector("#mp-debug-pan2d") as HTMLElement;
  const debugFov = debugDrawer.querySelector("#mp-debug-fov") as HTMLInputElement;
  const debugLensName = debugDrawer.querySelector("#mp-debug-lens-name") as HTMLElement;
  const debugLens = debugDrawer.querySelector("#mp-debug-lens") as HTMLOutputElement;
  const debugMaterial = debugDrawer.querySelector("#mp-debug-material") as HTMLSelectElement;
  const debugMaterialLabel = debugDrawer.querySelector("#mp-debug-material-label") as HTMLElement;
  const debugPhysics = debugDrawer.querySelector("#mp-debug-physics") as HTMLInputElement;
  const debugPhysicsLabel = debugDrawer.querySelector("#mp-debug-physics-label") as HTMLElement;
  const debugFrustum = debugDrawer.querySelector("#mp-debug-frustum") as HTMLInputElement;
  const debugFrustumLabel = debugDrawer.querySelector("#mp-debug-frustum-label") as HTMLElement;
  const debugGameUi = debugDrawer.querySelector("#mp-debug-game-ui") as HTMLInputElement;
  const debugReset = debugDrawer.querySelector("#mp-debug-reset") as HTMLButtonElement;

  // Every pane's networking routes through ONE coordinator (net-coordinator.ts):
  // the panes boot `?net=embedder`, post their conn commands to this page, and
  // it routes them between panes on perfect links. This module owns it because
  // it owns every pane — including pane 1, the page's own #player — so no pane
  // can be created outside its view. It is inert for a game that declares no
  // `Sub.connect`/`Sub.listen`: those panes never post a command.
  const net = new NetCoordinator();

  const panes: Pane[] = [];

  // The server pane, when the current example declares one (examples.ts
  // `server`). It is NOT in `panes`: `panes` is the client set the CLIENTS
  // control sizes and the mirror bookkeeping pops from. Everything that is
  // about "every running pane" — the scrub broadcasts, the aggregate pill, the
  // frame labels, the coordinator's routing table — goes through `allPanes()`,
  // with the server always LAST so client indices (and their digits) never move.
  let serverPane: { pane: Pane; bridge: PlayerBridge } | null = null;
  const allPanes = (): Pane[] => (serverPane ? [...panes, serverPane.pane] : panes);

  // The NETWORK view's wires and packet dots. It measures the pane cards the
  // CSS laid out and reads the coordinator's packet feed; it is inert until the
  // view is switched on.
  const asNode = (pane: Pane): NetworkNode => ({
    id: pane.id,
    shell: pane.shell,
    color: pane.color,
    linkLabel: () => `${pane.link.name} · ${pane.link.ms}ms`,
  });
  const graph = initNetworkGraph({
    stage,
    grid,
    net,
    clients: () => panes.map(asNode),
    server: () => (serverPane ? asNode(serverPane.pane) : null),
  });

  const makePane = (role: PaneRole, index: number, iframe: HTMLIFrameElement): Pane => {
    const client = role === "client";
    const id = client ? `client ${index + 1}` : "server";
    const color = client ? PLAYER_COLORS[index % PLAYER_COLORS.length] : "var(--scrub-server)";
    const shell = document.createElement("div");
    shell.className = client ? "mp-pane" : "mp-pane server";
    shell.style.setProperty("--pc", color);
    // The server pane's header says what it is instead of who it is: no digit
    // (it is not in the client sequence), no link chip (latency is a property
    // of a client's link — the authority has none), no "⌨ you".
    shell.innerHTML = `
      <div class="mp-pane-hd">
        ${client ? `<span class="mp-digit">${index + 1}</span>` : ""}
        <span class="mp-role">${client ? "client" : "SERVER"}</span>
        ${
          client
            ? `<span class="mp-link-host">
          <button class="mp-link-chip"
            title="Link impairment for this client (prototype — recorded per client; applies once the coordinator's link impairment lands)">⇅ Wi-Fi ▾</button>
        </span>`
            : `<span class="mp-authority">authority</span>`
        }
        <span class="mp-hd-r">
          ${client ? `<span class="mp-you" hidden>⌨ you</span>` : ""}
          <span class="mp-conn" hidden><b class="mp-conn-d"></b><span class="mp-conn-t"></span></span>
          <span class="mp-pf"><b>#f</b> <span class="mp-pf-n">—</span></span>
          <span class="mp-st" data-state="busy"></span>
        </span>
      </div>
      <div class="mp-pane-body"></div>
      <div class="mp-pane-err" hidden></div>`;
    shell.querySelector(".mp-pane-body")!.appendChild(iframe);
    // A client tile is INSERTED BEFORE the "+" tile (and so before the server's
    // card, which follows it), never appended after them: re-appending a
    // mounted node is a remove + insert, and that reloads an iframe — the same
    // invariant that keeps pane 1 in place would be broken for the authority,
    // wiping the world every time the count changed.
    grid.insertBefore(shell, client ? addTile : null);

    const tab = document.createElement("button");
    tab.className = client ? "mp-tab" : "mp-tab server";
    tab.style.setProperty("--pc", color);
    tab.innerHTML = `${client ? `<span class="mp-digit">${index + 1}</span> client` : "SERVER"}
      <span class="mp-pf"><b>#f</b> <span class="mp-pf-n">—</span></span>
      <span class="mp-st" data-state="busy"></span>`;
    // Same rule in the tab strip, with the "+" tab pinned last: a client's tab
    // goes before the server's, and both go before the "+".
    tabsStrip.insertBefore(tab, (client && serverPane?.pane.tab) || addTab);

    const pane: Pane = {
      role,
      id,
      color,
      iframe,
      shell,
      tab,
      scope: new AbortController(),
      state: "busy",
      link: { ...LINK_PRESETS[1] },
      frameLabels: [
        shell.querySelector(".mp-pf-n") as HTMLElement,
        tab.querySelector(".mp-pf-n") as HTMLElement,
      ],
      dots: [
        shell.querySelector(".mp-st") as HTMLElement,
        tab.querySelector(".mp-st") as HTMLElement,
      ],
      conn: shell.querySelector(".mp-conn") as HTMLElement,
      connDot: shell.querySelector(".mp-conn-d") as HTMLElement,
      connText: shell.querySelector(".mp-conn-t") as HTMLElement,
      stripFrame: null,
      errStrip: shell.querySelector(".mp-pane-err") as HTMLElement,
    };

    // Selecting a pane is what tabs mode needs to show it, so the server pane
    // is selectable by click — but it never joins the KEYBOARD client cycle
    // (digits / `[` / `]`) and never claims "⌨ you": game input belongs to a
    // client, and the authority has no player to drive.
    // preventDefault: mousedown's default action re-points focus at the
    // nearest focusable ancestor (body) AFTER this handler runs, silently
    // undoing the iframe.focus() — a synthetic dispatch never shows this.
    // Only a CLIENT claims the keyboard; selecting the server (tabs mode
    // needs it) is selection, not input.
    const select = (event?: Event) => {
      if (event?.target && (event.target as HTMLElement).closest?.(".mp-link-host")) return;
      event?.preventDefault();
      focusPane(allPanes().indexOf(pane), client);
    };
    shell.querySelector(".mp-pane-hd")!.addEventListener("mousedown", select);
    tab.addEventListener("click", select);
    if (client) {
      buildLinkMenu(pane, shell.querySelector(".mp-link-host") as HTMLElement, pane.scope.signal);
      panes.push(pane);
    }
    net.addPane(id, iframe, pane.scope.signal);
    attachPickerKeys(pane);
    return pane;
  };

  // Pane 1 wraps the page's existing #player (lang-intel, the live trace, and
  // the primary bridge keep talking to it untouched). Panes 2..N are mirrors,
  // added and removed LIVE — pane 1's iframe never moves again after this
  // wrap, so its running model survives every count change.
  makePane("client", 0, frame);
  const mirrors: { pane: Pane; bridge: PlayerBridge }[] = [];

  // Everything a pane the module created itself needs: its own status bridge
  // and its own prefixed console relay. Pane 1 is excluded by construction —
  // the PAGE owns its bridge (see `aggregateStatus`).
  const attachBridge = (pane: Pane): PlayerBridge => {
    const bridge = new PlayerBridge(pane.iframe, {
      onReloading: () => setPaneState(pane, "busy"),
      onLive: () => setPaneState(pane, "live"),
      onResult: (ok, message) => {
        setPaneState(pane, ok ? "live" : "error", message);
        if (!ok) statusBar.appendOutput("error", `[${pane.id}] ${message}`);
      },
      signal: pane.scope.signal,
    });
    window.addEventListener(
      "message",
      (event) => {
        const data = asPlayerMessage(event.data);
        if (!data || data.type !== "functor-lang-console") return;
        if (event.source !== pane.iframe.contentWindow) return;
        statusBar.appendOutput(data.level, `[${pane.id}] ${data.message}`, data.frame ?? null);
      },
      { signal: pane.scope.signal }
    );
    return bridge;
  };

  const addMirror = (pushCurrent: boolean) => {
    const index = panes.length;
    const iframe = document.createElement("iframe");
    iframe.title = `client ${index + 1} preview`;
    iframe.allow = "pointer-lock";
    const pane = makePane("client", index, iframe);
    const bridge = attachBridge(pane);
    mirrors.push({ pane, bridge });
    // A mirror added mid-session boots the served program, then catches up to
    // the (possibly edited) buffer; the bridge holds the push until ready.
    if (frame.getAttribute("src")) {
      iframe.src = frame.src; // already carries ?scrubber=hidden (the page adds it)
      if (pushCurrent) bridge.push(getSource());
    }
  };

  // ------------------------------------------------------------ server pane
  // Mounted for an example that declares a SERVER role, at every client count
  // including 1, and left alone by count changes — its world is the session's,
  // not a client's. It runs the same project files with the server file as the
  // entry, and its packets reach the clients through the same coordinator.
  //
  // It does NOT take editor pushes: `functor-lang-set-source` replaces the
  // ENTRY module's source, and the sandbox's buffer is the client entry —
  // pushing it here would swap the server's program for the client's. So an
  // edit hot-reloads the clients live while the server keeps running the
  // served build. Editing server.fun needs the multi-file `set-project` path
  // (the IDE already speaks it); until the sandbox grows a file switcher,
  // `↺ reset` reboots both roles from disk.
  const mountServer = (src: string) => {
    const iframe = document.createElement("iframe");
    iframe.title = "server preview";
    iframe.allow = "pointer-lock";
    const pane = makePane("server", panes.length, iframe);
    serverPane = { pane, bridge: attachBridge(pane) };
    iframe.src = src;
  };

  const unmountServer = () => {
    if (!serverPane) return;
    serverPane.bridge.reset();
    serverPane.pane.scope.abort();
    serverPane.pane.shell.remove();
    serverPane.pane.tab.remove();
    paneDetails.delete(serverPane.pane);
    serverPane = null;
    if (focusedPane === null || !allPanes().includes(focusedPane)) focusPane(0);
  };

  const removeMirror = () => {
    const removed = mirrors.pop();
    if (!removed) return;
    removed.bridge.reset();
    removed.pane.scope.abort(); // window/document/bridge listeners
    removed.pane.shell.remove();
    removed.pane.tab.remove();
    panes.pop();
  };

  for (let i = 1; i < count; i++) addMirror(false);

  // ------------------------------------------------------------ focus model
  // Focus is held by IDENTITY, not by index: the server pane sits after the
  // clients, so growing or shrinking the client set would otherwise slide the
  // selection onto a different pane.
  let focusedPane: Pane | null = null;
  const focusedIndex = () => (focusedPane ? allPanes().indexOf(focusedPane) : 0);
  // claimKeyboard: user-intent callers (header click, tab click, digits,
  // `[` `]`) also hand the pane the BROWSER's keyboard — the chrome must
  // never say "⌨ you" while keys still land in the host page (the bug: WASD
  // only worked after a second click inside the canvas). Reconcile/boot
  // callers pass false so loading a session never steals the editor's caret.
  function focusPane(index: number, claimKeyboard = false) {
    const current = allPanes();
    const next = current[index] ?? current[0] ?? null;
    if (next !== focusedPane && focusedPane && seamOf(focusedPane.iframe)?.detached()) {
      exitPanePointerLock(focusedPane.iframe);
    }
    focusedPane = next;
    current.forEach((pane) => {
      const on = pane === focusedPane;
      pane.shell.classList.toggle("focus", on);
      pane.tab.classList.toggle("focus", on);
      const you = pane.shell.querySelector(".mp-you") as HTMLElement | null;
      if (you) you.hidden = !on;
    });
    if (claimKeyboard && focusedPane) focusedPane.iframe.focus();
  }
  focusPane(0);

  // Every transient surface (the link menus) dismisses together — popovers
  // behave modally without blocking the page.
  const closePopovers = () => {
    for (const menu of document.querySelectorAll<HTMLElement>(".mp-link-menu")) {
      menu.hidden = true;
    }
  };

  const moduleScope = new AbortController();

  // Clicking into a pane's iframe gives it the real keyboard; follow that
  // with the focus chrome so "who has my keys" never lies. The blur doubles
  // as click-away dismissal for the popovers — an iframe click never reaches
  // this document's mousedown, but it always steals the window's focus.
  window.addEventListener("blur", () => {
    closePopovers();
    const active = document.activeElement;
    const position = allPanes().findIndex((candidate) => candidate.iframe === active);
    if (position >= 0) focusPane(position);
  }, { signal: moduleScope.signal });


  // Digits jump panes, [ ] cycle, f toggles tiled/tabs — but never while the
  // caret is in an editor or input (digits are text there, no exceptions).
  //
  // Attached to the host window AND to every pane's contentDocument (same
  // origin) — once a pane owns the real keyboard, its document is where
  // these keys land, and the picker must keep working (not one-shot). The
  // pane's GAME still receives the key too (we never stop propagation):
  // a game that binds Num/bracket keys sees a spurious press when the user
  // works the picker — accepted for now, resolved properly when host-routed
  // input lands (coordinator arc, input inversion).
  function pickerKeys(event: KeyboardEvent) {
    const target = event.target as HTMLElement;
    if (event.key === "Escape") {
      closePopovers();
      primarySeam()?.selectEvent(null);
      return;
    }
    if (target.closest?.(".cm-editor") || /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName)) return;
    // Digits and `[` `]` walk the CLIENTS only — they are the keyboard's
    // player picker, and the authority has no player. (Its tab still selects
    // it, which is what tabs mode needs to show it at all.)
    if (/^[1-9]$/.test(event.key)) {
      const index = Number(event.key) - 1;
      if (index < panes.length) focusPane(index, true);
    } else if (event.key === "[" || event.key === "]") {
      const delta = event.key === "]" ? 1 : -1;
      const from = focusedIndex();
      // From the server pane (which sits past the clients), the cycle re-enters
      // the client set at whichever end the direction implies.
      if (from >= panes.length) focusPane(delta === 1 ? 0 : panes.length - 1, true);
      else focusPane((from + delta + panes.length) % panes.length, true);
    } else if (event.key === "f") {
      let next = VIEWS.indexOf(grid.dataset.view as PaneView);
      for (let hop = 0; hop < VIEWS.length; hop++) {
        next = (next + 1) % VIEWS.length;
        if (viewAvailable(VIEWS[next])) break;
      }
      setView(VIEWS[next]);
    }
  }
  window.addEventListener("keydown", pickerKeys, { signal: moduleScope.signal });
  // Every pane's contentDocument gets the same picker (wired at creation;
  // re-wired on every load since navigation replaces the document).
  function attachPickerKeys(pane: Pane) {
    const wire = () => {
      try {
        pane.iframe.contentDocument?.addEventListener("keydown", pickerKeys, {
          signal: pane.scope.signal,
        });
      } catch {
        // A cross-origin or torn-down document has no picker to serve.
      }
    };
    wire();
    pane.iframe.addEventListener("load", wire, { signal: pane.scope.signal });
  }

  // ------------------------------------------------ tiled / grid / tabs view
  // Switching layout is a CLASS CHANGE ONLY — `data-view` on the one grid, and
  // pure CSS from there. Nothing may reorder or re-parent a pane's node: a
  // re-appended iframe reloads and wipes its model (the same invariant that
  // keeps pane 1, and the server pane, in place across count changes).
  //
  // NETWORK is the one view that can be unavailable: it is a hub-and-spoke
  // picture, and a single-role example has no hub. Rather than inventing a ring
  // around nothing, the option is DISABLED with a tooltip that says why — and
  // `f` skips it, so the cycle never stalls on a view it cannot enter.
  const viewAvailable = (view: PaneView) => view !== "network" || serverPane !== null;

  // FLIP (Addendum 5a.4), by hand and deliberately NOT react-spring: the panes
  // live OUTSIDE React because a re-parented iframe reloads and wipes its model
  // (the e2e's document-lifetime markers exist to catch exactly that), so the
  // animation has to be one that never touches the tree. FLIP is that
  // animation: measure First, let the class change decide Last, then Invert the
  // delta as a transform on the SAME node and Play it back to none. Nothing is
  // created, moved, or re-parented — a switch mid-flight simply re-measures
  // from wherever the cards currently are.
  const SWITCH_MS = 420;
  /** Overshoot-free but spring-paced: quick off the mark, long settle. */
  const SWITCH_EASING = "cubic-bezier(0.22, 1, 0.3, 1)";
  const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
  /** Views in flight — while any is, the wires re-measure every frame, since a
   * transform moves a card's box without resizing anything. */
  let animating = 0;
  /** Our own FLIP per node, so an interrupted switch cancels exactly what it
   * started (a pane also carries CSS transitions we must not touch). */
  const inFlight = new Map<HTMLElement, Animation>();

  const flipViews = (apply: () => void) => {
    const nodes = [...allPanes().map((pane) => pane.shell), addTile];
    // Reduced motion: the layout still changes, it just arrives instantly.
    if (reduceMotion.matches) {
      apply();
      return;
    }
    // FIRST is measured WITH any running transform — a switch mid-flight has to
    // start from where the card visibly is. OUR previous animation on that node
    // is then cancelled before LAST is measured, or the destination box would be
    // read through a transform still being applied (its `finished` rejection is
    // caught below, and `animating` still settles in the `finally`). Only ours:
    // `getAnimations()` would also return the pane's CSS focus transitions.
    const first = nodes.map((node) => node.getBoundingClientRect());
    for (const node of nodes) inFlight.get(node)?.cancel();
    apply();
    nodes.forEach((node, index) => {
      const from = first[index];
      const to = node.getBoundingClientRect();
      if (from.width === 0 || to.width === 0) return;
      const dx = from.left - to.left;
      const dy = from.top - to.top;
      const sx = from.width / to.width;
      const sy = from.height / to.height;
      if (Math.abs(dx) < 1 && Math.abs(dy) < 1 && Math.abs(sx - 1) < 0.01 && Math.abs(sy - 1) < 0.01) {
        return;
      }
      const animation = node.animate(
        [
          { transformOrigin: "0 0", transform: `translate(${dx}px, ${dy}px) scale(${sx}, ${sy})` },
          { transformOrigin: "0 0", transform: "none" },
        ],
        { duration: SWITCH_MS, easing: SWITCH_EASING }
      );
      animating += 1;
      inFlight.set(node, animation);
      animation.finished
        .catch(() => {})
        .finally(() => {
          animating -= 1;
          if (inFlight.get(node) === animation) inFlight.delete(node);
          if (animating === 0 && grid.dataset.view === "network") graph.relayout();
        });
    });
  };

  const setView = (view: PaneView, force = false) => {
    if (allPanes().length === 1 && !force) return;
    if (!viewAvailable(view)) return;
    if (view === grid.dataset.view) return;
    flipViews(() => {
      grid.dataset.view = view;
      stage.dataset.view = view;
      tabsStrip.hidden = view !== "tabs";
      // The strip's LEGEND is network-only (nothing flies in the other views);
      // its pane readouts are not — see updateChrome.
      viewStrip.dataset.view = view;
    });
    for (const candidate of VIEWS) {
      $btn(`mp-view-${candidate}`).setAttribute("aria-pressed", String(view === candidate));
    }
    // Turning the graph on measures the boxes this class change just produced.
    graph.setActive(view === "network");
  };
  for (const view of VIEWS) $btn(`mp-view-${view}`).addEventListener("click", () => setView(view));

  // Everything on the chrono bar that depends on how many panes exist.
  // (CSS keyed on data-count hides the pane header / 🔮 / view toggle.)
  const updateChrome = () => {
    // "Single" is about TILES, not clients: one client plus a server is still
    // a grid, so it keeps its pane headers and the layout toggle.
    const single = allPanes().length === 1;
    previewPane.dataset.count = String(allPanes().length);
    // Grid's column rules need the CLIENT count, which `data-count` (every
    // pane, server included) cannot express: "2 panes" is one client under an
    // authority or two peers in a single-role example, and those lay out
    // differently.
    grid.dataset.clients = String(panes.length);
    for (const view of VIEWS) $btn(`mp-view-${view}`).disabled = single;
    // The network button's tooltip is written here and nowhere else — it says
    // either what the view is or why it is unavailable.
    const network = $btn("mp-view-network");
    if (!single) network.disabled = !viewAvailable("network");
    network.title = viewAvailable("network")
      ? "Network: the server as the hub, with live packet flow along each link (f cycles)"
      : "Network needs an authority — this example declares no server role, so there is no hub to arrange the clients around";
    // The "+" affordances: hidden at MAX_CLIENTS (there is nothing to add), and
    // hidden in the single-pane layout, which drops every other piece of
    // multiplayer chrome with it.
    // The strip carries the pane readouts in EVERY multi-pane view, not just
    // network: the headers clip by their own width (a tiled row of four is
    // thumbnails too), so the frame counters need a home wherever that happens.
    // Only the packet legend is network-scoped, via `data-view` above.
    viewStrip.hidden = single;
    const canAdd = !single && panes.length < MAX_CLIENTS;
    addTile.hidden = !canAdd;
    addTab.hidden = !canAdd;
    // Grid's cell rules need both of these as data, not as structure: a hidden
    // tile is still a child, so `:last-child` cannot answer "is the bottom row
    // closed" any more.
    grid.dataset.seat = canAdd ? "open" : "full";
    grid.dataset.server = serverPane ? "yes" : "no";
    if (single) setView("tiled", true);
    // An example that dropped its server pane cannot stay in the hub view.
    else if (grid.dataset.view === "network" && !viewAvailable("network")) setView("tiled");
    else if (grid.dataset.view === "network") graph.relayout();
    rebuildStrip();
  };

  // The network strip's pane readouts, rebuilt whenever the pane set changes.
  // Each row is the identity the clipped header keeps (digit / SERVER) plus the
  // frame counter it drops; the paint loop writes the numbers.
  function rebuildStrip() {
    stripInks.replaceChildren(
      ...panes.map((pane) => {
        const ink = document.createElement("i");
        ink.className = "mp-sw up";
        ink.style.setProperty("--pc", pane.color);
        return ink;
      })
    );
    stripPanes.replaceChildren(
      ...allPanes().map((pane) => {
        const row = document.createElement("span");
        row.className = `mp-vs-pane${pane.role === "server" ? " server" : ""}`;
        row.style.setProperty("--pc", pane.color);
        row.innerHTML =
          `${pane.role === "server" ? `<i class="mp-vs-who">SRV</i>` : `<i class="mp-digit">${panes.indexOf(pane) + 1}</i>`}` +
          `<b>#f</b> <span class="mp-pf-n">—</span>`;
        pane.stripFrame = row.querySelector(".mp-pf-n") as HTMLElement;
        return row;
      })
    );
  }

  const requestAnotherClient = () => requestCount(panes.length + 1);
  addTile.addEventListener("click", requestAnotherClient);
  addTab.addEventListener("click", requestAnotherClient);

  // Live client-count change: grow or shrink the mirror set in place. Pane 1
  // is untouched, so its model (and the editor session) survive.
  const setCount = (n: number) => {
    const target = Math.max(1, Math.min(MAX_CLIENTS, Math.floor(n) || 1));
    // Extrapolation is single-client-only, and its controls are hidden at
    // N > 1 — so leaving single must switch it OFF, or a pane keeps
    // speculatively re-simulating with no way to stop it.
    if (panes.length === 1 && target > 1) {
      for (const seam of seams()) seam.setPreview({ enabled: false });
    }
    while (panes.length < target) addMirror(true);
    while (panes.length > target) removeMirror();
    if (focusedPane === null || !allPanes().includes(focusedPane)) focusPane(0);
    updateChrome();
    paintAggregate();
  };
  updateChrome();

  // -------------------------------------------------------------- link menu
  function buildLinkMenu(pane: Pane, host: HTMLElement, signal: AbortSignal) {
    const chip = host.querySelector(".mp-link-chip") as HTMLButtonElement;
    const menu = document.createElement("div");
    menu.className = "mp-link-menu";
    menu.hidden = true;
    menu.innerHTML = `
      <div class="mp-link-presets">
        ${LINK_PRESETS.map(
          (preset) =>
            `<button data-name="${preset.name}" aria-pressed="${preset.name === "Wi-Fi"}">${preset.name}</button>`
        ).join("")}
      </div>
      <label>⇅ <input class="mp-l-ms" type="number" min="0" value="45"> ms
        ± <input class="mp-l-j" type="number" min="0" value="12"></label>
      <label>✂ <input class="mp-l-loss" type="number" min="0" max="100" step="0.1" value="1.2"> % loss</label>
      <p class="mp-link-note">prototype: recorded per client, applied when link impairment lands</p>`;
    host.appendChild(menu);

    const msInput = menu.querySelector(".mp-l-ms") as HTMLInputElement;
    const jitterInput = menu.querySelector(".mp-l-j") as HTMLInputElement;
    const lossInput = menu.querySelector(".mp-l-loss") as HTMLInputElement;

    const paintChip = () => {
      chip.textContent = `⇅ ${pane.link.name === "custom" ? `${pane.link.ms}ms` : pane.link.name} ▾`;
      msInput.value = String(pane.link.ms);
      jitterInput.value = String(pane.link.jitter);
      lossInput.value = String(pane.link.loss);
      for (const button of menu.querySelectorAll(".mp-link-presets button")) {
        button.setAttribute(
          "aria-pressed",
          String((button as HTMLElement).dataset.name === pane.link.name)
        );
      }
    };
    chip.addEventListener("click", () => {
      const open = menu.hidden;
      for (const other of document.querySelectorAll<HTMLElement>(".mp-link-menu")) {
        other.hidden = true;
      }
      menu.hidden = !open;
    });
    document.addEventListener(
      "mousedown",
      (event) => {
        if (!host.contains(event.target as Node)) menu.hidden = true;
      },
      { signal }
    );
    for (const button of menu.querySelectorAll<HTMLButtonElement>(".mp-link-presets button")) {
      button.addEventListener("click", () => {
        const preset = LINK_PRESETS.find((candidate) => candidate.name === button.dataset.name);
        if (preset) pane.link = { ...preset };
        paintChip();
      });
    }
    for (const input of menu.querySelectorAll("input")) {
      input.addEventListener("input", () => {
        pane.link = {
          name: "custom",
          ms: Number(msInput.value) || 0,
          jitter: Number(jitterInput.value) || 0,
          loss: Number(lossInput.value) || 0,
        };
        paintChip();
      });
    }
    paintChip();
  }

  // -------------------------------------------------- aggregate status pill
  // With one client the PAGE owns the pill (its exact loading…/live/error
  // texts); with more, the pill aggregates every pane. `lastMain` lets a
  // shrink back to one client restore the page's own wording.
  let mainState: PaneState = "busy";
  let mainDetail = "";
  const paneDetails = new Map<Pane, string>();
  let lastMain: { state: PaneState; text: string; detail: string } | null = null;
  const setPaneState = (pane: Pane, state: PaneState, detail = "") => {
    pane.state = state;
    paneDetails.set(pane, detail);
    for (const dot of pane.dots) dot.dataset.state = state;
    pane.errStrip.hidden = state !== "error";
    if (state === "error") pane.errStrip.textContent = `✕ ${detail}`;
    paintAggregate();
  };
  const paintAggregate = () => {
    const running = allPanes();
    if (running.length === 1) {
      if (lastMain) setPill(lastMain.state, lastMain.text, lastMain.detail);
      return;
    }
    // Pane 1's state is the page's (`mainState`); the rest report their own.
    const states = [mainState, ...running.slice(1).map((pane) => pane.state)];
    if (states.includes("error")) {
      const failing = running.find((pane) => pane.state === "error");
      setPill("error", "✕ build error", (failing && paneDetails.get(failing)) || mainDetail);
    } else if (states.includes("busy")) {
      setPill("busy", "◐ reloading…", "");
    } else {
      // The clients and the server are counted SEPARATELY ("2+1 running"): the
      // CLIENTS control says 2, and a bare "3" would read as a third client.
      setPill(
        "live",
        serverPane ? `● ${panes.length}+1 running` : `● ${running.length} running`,
        mainDetail
      );
    }
  };

  // ------------------------------------------------------ chrono bar wiring
  // The transport speaks to EVERY pane, server included: the authority parks
  // and steps with its clients, or a paused session keeps simulating.
  const seams = () =>
    allPanes()
      .map((pane) => seamOf(pane.iframe))
      .filter((s): s is ScrubSeam => !!s);
  // The pane the chrono bar mirrors: the focused one, or — while that pane is
  // reloading and has no seam — the first that has one. Resolved as a PANE, not
  // as a seam: the bar reads its viewport AND its frame offset, and taking those
  // two from different panes is how a reload window mislabels the rail.
  const primaryPane = (): Pane | null => {
    if (focusedPane && seamOf(focusedPane.iframe)) return focusedPane;
    return allPanes().find((pane) => seamOf(pane.iframe)) ?? null;
  };
  const primarySeam = () => {
    const pane = primaryPane();
    return pane ? seamOf(pane.iframe) : null;
  };

  // ------------------------------------------------------ per-pane frame offsets
  // A pane's frame index is BOOT-RELATIVE: it counts from 0 when that document
  // starts running. So a client added three seconds into a session is ~180
  // frames behind the rest for the very same instant, and a broadcast seek sent
  // as a bare number lands somewhere else in its world (or clamps to the end of
  // its recording, which is what you actually see: the late client refuses to
  // move while everyone else rewinds).
  //
  // There is no session epoch to translate through yet, so the host MEASURES
  // the difference, every frame, from the panes' LIVE HEADS:
  //
  //     offset(pane) = head(pane) - head(reference)
  //
  // where a head is `range()[1]` — the newest frame a pane has simulated — read
  // for every pane in the same rAF, so the numbers describe one moment. It is
  // deliberately the head and NOT `frame()`: `frame()` is where that pane's
  // scrubber is currently PARKED, so measuring from it while the session sits
  // rewound would call the parked position the present and bake the rewind into
  // the offset. The reference clock is the SERVER pane when there is one (the
  // only pane that outlives every client) and pane 1 otherwise.
  //
  // Nothing is latched, and that is what keeps this honest: a pane whose head
  // has not appeared yet (booting — the range is empty until the first frame is
  // recorded) is simply not measured, and one that reloads, or joins while the
  // session is parked, or hitches behind the others, is re-measured on the next
  // frame from whatever its head actually is. An earlier draft latched a first
  // reading and needed reload detection, boot detection, and a reference-change
  // rule to stay correct; measuring continuously needs none of them.
  //
  // Every broadcast seek, and the rail's own label, is translated through it —
  // the chrono bar speaks reference-clock frames and each pane hears its own.
  //
  // The exact epoch — a frame index that means the same instant everywhere by
  // construction — arrives with barrier stepping, when the coordinator delivers
  // on step boundaries instead of on the host's rAF (coordinator arc).
  const offsets = new Map<Pane, number>();
  const referencePane = (): Pane | null => serverPane?.pane ?? panes[0] ?? null;

  /** A pane's live head, or null while it has recorded nothing. */
  const headOf = (pane: Pane): number | null => {
    const range = seamOf(pane.iframe)?.range();
    const head = range && range.length === 2 ? range[1] : null;
    return typeof head === "number" && head > 0 ? head : null;
  };

  const measureClocks = () => {
    const reference = referencePane();
    const referenceHead = reference ? headOf(reference) : null;
    if (!reference || referenceHead === null) return;
    offsets.set(reference, 0);
    const live = allPanes();
    for (const pane of live) {
      if (pane === reference) continue;
      const head = headOf(pane);
      // No head yet: keep whatever was last measured rather than claiming 0 —
      // a booting pane is not "in step with the server".
      if (head !== null) offsets.set(pane, head - referenceHead);
    }
    for (const pane of [...offsets.keys()]) if (!live.includes(pane)) offsets.delete(pane);
  };

  const offsetOf = (pane: Pane | null): number => (pane && offsets.get(pane)) || 0;

  /** A frame the PRIMARY pane reported (the rail mirrors that pane), read back
   * in reference-clock frames. */
  const toReference = (paneFrame: number): number => paneFrame - offsetOf(primaryPane());

  /**
   * Seek every pane to one reference-clock frame, each in its own indices.
   *
   * A pane whose recording does not reach that instant — a client that joined
   * after it — clamps to its own earliest frame rather than the seek being
   * refused for everyone: rewinding to before a late client joined is a normal
   * thing to want, and the pane's own rail says where it actually landed.
   */
  const seekAll = (referenceFrame: number) => {
    for (const pane of allPanes()) {
      seamOf(pane.iframe)?.seek(Math.max(0, Math.round(referenceFrame + offsetOf(pane))));
    }
  };
  let pendingDetached:
    | { seam: ScrubSeam; generation: number; iframe: HTMLIFrameElement }
    | null = null;
  let pendingPointerLock: { seam: ScrubSeam; iframe: HTMLIFrameElement } | null = null;
  const reconcilePendingCamera = () => {
    if (
      pendingPointerLock !== null &&
      (!pendingPointerLock.iframe.isConnected ||
        seamOf(pendingPointerLock.iframe) !== pendingPointerLock.seam ||
        focusedPane?.iframe !== pendingPointerLock.iframe)
    ) {
      try {
        pendingPointerLock.seam.setDetachedPointerLockPending(false);
      } catch {
        // A navigated iframe has already destroyed the old seam.
      }
      exitPanePointerLock(pendingPointerLock.iframe);
      pendingPointerLock = null;
    }
    if (pendingDetached === null) return;

    const request = pendingDetached;
    const liveSeam = request.iframe.isConnected ? seamOf(request.iframe) : null;
    let acknowledged = liveSeam !== request.seam;
    if (!acknowledged) {
      try {
        acknowledged = request.seam.detachedGeneration() !== request.generation;
      } catch {
        acknowledged = true;
      }
    }
    if (!acknowledged) return;

    pendingDetached = null;
    let detached = false;
    if (liveSeam === request.seam) {
      try {
        detached = request.seam.detached();
      } catch {
        // Treat a destroyed realm as a refused request.
      }
    }
    if (!detached || focusedPane?.iframe !== request.iframe) {
      exitPanePointerLock(request.iframe);
    }
  };

  $btn("mp-pause").addEventListener("click", () => {
    const primary = primarySeam();
    if (!primary) return;
    const desired = !primary.paused();
    for (const seam of seams()) if (seam.paused() !== desired) seam.togglePause();
  });
  $btn("mp-step").addEventListener("click", () => {
    for (const seam of seams()) seam.step();
  });
  $btn("mp-camera").addEventListener("click", () => {
    const iframe = focusedPane!.iframe;
    const seam = seamOf(iframe);
    if (!seam?.canDetach()) return;
    const setPointerClaim = (pending: boolean) => {
      try {
        seam.setDetachedPointerLockPending(pending);
      } catch {
        // A navigation can destroy the iframe realm while capture is pending.
      }
    };
    const iframeDocument = iframe.contentDocument;
    const wasDetached = seam.detached();
    const queueToggle = () => {
      pendingDetached = {
        seam,
        generation: seam.detachedGeneration(),
        iframe,
      };
      $btn("mp-camera").disabled = true;
      seam.toggleDetached();
    };
    if (wasDetached) {
      queueToggle();
      exitPanePointerLock(iframe);
      return;
    }
    const canvas = iframeDocument?.getElementById("canvas") as HTMLCanvasElement | null;
    if (!canvas) return;
    setPointerClaim(true);
    pendingPointerLock = { seam, iframe };
    $btn("mp-camera").disabled = true;
    const accepted = () => {
      if (
        pendingPointerLock?.seam !== seam ||
        pendingPointerLock.iframe !== iframe ||
        !iframe.isConnected ||
        seamOf(iframe) !== seam ||
        focusedPane?.iframe !== iframe
      ) {
        if (pendingPointerLock?.seam === seam && pendingPointerLock.iframe === iframe) {
          pendingPointerLock = null;
        }
        setPointerClaim(false);
        exitPanePointerLock(iframe);
        return;
      }
      pendingPointerLock = null;
      focusPaneCameraInput(iframe, canvas);
      queueToggle();
      setPointerClaim(false);
    };
    const refused = () => {
      if (pendingPointerLock?.seam === seam && pendingPointerLock.iframe === iframe) {
        pendingPointerLock = null;
      }
      setPointerClaim(false);
    };
    try {
      const request = canvas.requestPointerLock();
      if (request && typeof request.then === "function") {
        request.then(accepted, refused);
      } else {
        accepted();
      }
    } catch {
      refused();
    }
  });
  debugMode.addEventListener("change", () => {
    primarySeam()?.setDebugCamera({ mode: Number(debugMode.value) });
  });
  debugFov.addEventListener("input", () => {
    debugLens.value = `${Math.round(debugFov.valueAsNumber)}°`;
    primarySeam()?.setDebugCamera({ fov: debugFov.valueAsNumber });
  });
  debugMaterial.addEventListener("change", () => {
    primarySeam()?.setDebugCamera({ material: Number(debugMaterial.value) });
  });
  debugPhysics.addEventListener("change", () => {
    primarySeam()?.setDebugCamera({ physics: debugPhysics.checked });
  });
  debugFrustum.addEventListener("change", () => {
    primarySeam()?.setDebugCamera({ authoredFrustum: debugFrustum.checked });
  });
  debugGameUi.addEventListener("change", () => {
    primarySeam()?.setDebugCamera({ gameUi: debugGameUi.checked });
  });
  debugReset.addEventListener("click", () => {
    primarySeam()?.setDebugCamera({ reset: true });
  });
  // The speculative preview (🔮): the pane simulates forward from its parked
  // frame under the current code, replaying recorded input. Broadcast like
  // pause (single-client-only in the UI; the CSS hides it at N > 1).
  $btn("mp-extrap").addEventListener("click", () => {
    const primary = primarySeam();
    if (!primary) return;
    const enabled = !primary.model().preview.enabled;
    for (const seam of seams()) seam.setPreview({ enabled });
  });

  // The pink preview handle: drag (or arrow keys) to stretch the window.
  const previewHandle = $("mp-preview-handle");
  let previewDrag: { x: number; seconds: number } | null = null;
  previewHandle.addEventListener("pointerdown", (event) => {
    event.preventDefault();
    event.stopPropagation();
    previewHandle.setPointerCapture(event.pointerId);
    previewDrag = { x: event.clientX, seconds: primarySeam()?.model().preview.seconds ?? 2 };
  });
  previewHandle.addEventListener("pointermove", (event) => {
    if (!previewDrag || !previewHandle.hasPointerCapture(event.pointerId)) return;
    const current = primarySeam()?.view();
    const width = rail.getBoundingClientRect().width;
    const span = current ? current.viewport.hi - current.viewport.lo : 0;
    if (width <= 0 || span <= 0) return;
    const deltaFrames = ((event.clientX - previewDrag.x) / width) * span;
    for (const seam of seams()) {
      seam.setPreview({ seconds: previewDrag.seconds + deltaFrames / 60 });
    }
  });
  const endPreviewDrag = () => {
    previewDrag = null;
  };
  previewHandle.addEventListener("pointerup", endPreviewDrag);
  previewHandle.addEventListener("pointercancel", endPreviewDrag);
  previewHandle.addEventListener("lostpointercapture", endPreviewDrag);
  previewHandle.addEventListener("keydown", (event) => {
    if (event.key === "Home" || event.key === "End") {
      event.preventDefault();
      event.stopPropagation();
      const seconds = event.key === "Home" ? 0.5 : 5;
      for (const seam of seams()) seam.setPreview({ seconds });
      return;
    }
    const step = event.shiftKey ? 1 : 0.5;
    const deltas: Record<string, number> = {
      ArrowLeft: -step, ArrowDown: -step, ArrowRight: step, ArrowUp: step,
      PageDown: -1, PageUp: 1,
    };
    const delta = deltas[event.key];
    if (delta === undefined) return;
    event.preventDefault();
    event.stopPropagation();
    const seconds = (primarySeam()?.model().preview.seconds ?? 2) + delta;
    for (const seam of seams()) seam.setPreview({ seconds });
  });
  const tip = $("mp-evt-tip");
  const showTip = (unit: number, text: string) => {
    tip.style.display = "block";
    tip.style.left = `${Math.min(95, Math.max(5, unit * 100))}%`;
    tip.textContent = text;
  };
  const hideTip = () => {
    tip.style.display = "none";
  };

  const rail = $("mp-rail");
  const seekAt = (event: PointerEvent) => {
    const primary = primarySeam();
    const current = primary?.view();
    if (!current) return;
    const rect = rail.getBoundingClientRect();
    const unit =
      rect.width > 0 ? Math.max(0, Math.min(1, (event.clientX - rect.left) / rect.width)) : 0;
    const target = Math.round(
      current.viewport.lo + unit * (current.viewport.hi - current.viewport.lo)
    );
    // The viewport is the FOCUSED pane's, so the target is in its indices:
    // back to the reference clock, then out to every pane in its own.
    seekAll(toReference(target));
  };
  rail.addEventListener("pointerdown", (event) => {
    event.preventDefault();
    rail.setPointerCapture(event.pointerId);
    seekAt(event);
  });
  rail.addEventListener("pointermove", (event) => {
    if (rail.hasPointerCapture(event.pointerId)) seekAt(event);
  });
  const playheadEl = $("mp-playhead");
  playheadEl.addEventListener("keydown", (event) => {
    const current = primarySeam()?.view();
    if (!current) return;
    const steps = event.shiftKey ? 10 : 1;
    const targets: Record<string, number> = {
      ArrowLeft: current.selectedFrame - steps,
      ArrowDown: current.selectedFrame - steps,
      ArrowRight: current.selectedFrame + steps,
      ArrowUp: current.selectedFrame + steps,
      PageDown: current.selectedFrame - TIMELINE_FPS,
      PageUp: current.selectedFrame + TIMELINE_FPS,
      // Recorded bounds, not the viewport's: after a reload the viewport can
      // include striped history that is provably unseekable.
      Home: current.recorded.lo,
      End: current.recorded.hi,
    };
    const target = targets[event.key];
    if (target === undefined) return;
    event.preventDefault();
    event.stopPropagation();
    seekAll(toReference(Math.round(target)));
  });

  // The rail mirrors the FOCUSED pane (each pane is an independent sim in the
  // prototype, so one authoritative rail per focus is the honest picture; the
  // sim substrate replaces this with server time).
  const markersHost = $("mp-markers");
  const ticksHost = $("mp-ticks");
  let markerKey = "";
  let lastLabel = "";
  const paint = () => {
    // Reconcile against the pane that initiated the request even while the
    // focused pane is loading. Navigation/removal makes that old seam unable
    // to acknowledge, so treat it as a refusal instead of stalling the bar.
    reconcilePendingCamera();
    measureClocks();
    const primary = primarySeam();
    const current = primary?.view();
    // The rail mirrors the focused pane, but it LABELS the reference clock:
    // the numbers on the bar mean the same instant whichever pane you focus.
    // (Geometry is unit fractions of that pane's own viewport, so only the
    // numbers shift.)
    const clockShift = offsetOf(primaryPane());
    // Clamped at 0: a pane that booted BEFORE the reference has a positive
    // offset, and its first second would otherwise label as negative frames.
    const inReference = (frame: number) => Math.max(0, Math.round(frame - clockShift));
    if (!primary?.detached()) debugDrawer.hidden = true;
    chrono.classList.toggle("dormant", !current);
    if (primary && current) {
      const span = Math.max(current.viewport.hi - current.viewport.lo, 1);
      const played = $("mp-played");
      played.style.left = `${current.recordedStartUnit * 100}%`;
      played.style.width = `${
        Math.max(
          Math.min(current.playheadUnit, current.recordedEndUnit) - current.recordedStartUnit,
          0
        ) * 100
      }%`;
      $("mp-recorded").style.left = `${current.recordedStartUnit * 100}%`;
      $("mp-recorded").style.width =
        `${Math.max(current.recordedEndUnit - current.recordedStartUnit, 0) * 100}%`;
      $("mp-playhead").style.left = `${current.playheadUnit * 100}%`;
      const outside = current.playheadClippedBefore || current.playheadClippedAfter;
      $("mp-playhead").classList.toggle("outside", outside);
      // Unavailable-history stripes: the frozen viewport's regions whose
      // snapshots did not survive a reload (same geometry as the in-frame bar).
      const striped = current.hasUnavailableHistory;
      $("mp-unavailable").style.width = striped
        ? `${current.unavailableEndUnit * 100}%`
        : "0";
      const unavailableAfter = $("mp-unavailable-after");
      unavailableAfter.style.left = `${current.unavailableAfterStartUnit * 100}%`;
      unavailableAfter.style.width = striped
        ? `${Math.max(1 - current.unavailableAfterStartUnit, 0) * 100}%`
        : "0";
      const previewOn = primary.model().preview.enabled;
      const future = $("mp-future");
      future.style.left = `${current.playheadUnit * 100}%`;
      future.style.width = previewOn
        ? `${Math.max(current.previewEndUnit - current.playheadUnit, 0) * 100}%`
        : "0";
      previewHandle.style.display = previewOn ? "block" : "none";
      previewHandle.style.left = `${current.previewEndUnit * 100}%`;
      previewHandle.classList.toggle("clipped", current.previewClippedFrames > 0);
      previewHandle.classList.toggle(
        "fully-clipped",
        previewOn && current.previewFrames > 0 && current.previewEndUnit <= current.playheadUnit
      );
      previewHandle.setAttribute(
        "aria-valuemin",
        String(current.selectedFrame + Math.round(0.5 * TIMELINE_FPS))
      );
      previewHandle.setAttribute(
        "aria-valuemax",
        String(current.selectedFrame + 5 * TIMELINE_FPS)
      );
      previewHandle.setAttribute(
        "aria-valuenow",
        String(current.selectedFrame + current.previewFrames)
      );
      previewHandle.setAttribute(
        "aria-valuetext",
        `${primary.model().preview.seconds} seconds ahead` +
          (current.previewClippedFrames ? `, ${current.previewClippedFrames} frames clipped` : "")
      );
      const overflow = $("mp-overflow");
      overflow.style.display = previewOn && current.previewClippedFrames > 0 ? "block" : "none";
      overflow.textContent = `+${current.previewClippedFrames}`;
      $btn("mp-extrap").classList.toggle("on", previewOn);
      $btn("mp-extrap").setAttribute("aria-pressed", String(previewOn));
      const selectedRef = inReference(current.selectedFrame);
      const viewportHiRef = inReference(current.viewport.hi);
      const labelHtml =
        `${selectedRef}` +
        (outside ? ` <span class="out">outside</span>` : "") +
        (previewOn ? ` <span class="fut">+${current.previewFrames}</span>` : "") +
        ` / ${viewportHiRef}`;
      // The stripes' availability note is ARIA-only, but it shares the label's
      // change guard so it repaints exactly when the label does. It follows
      // timeline-model's describeRecordedAvailability WORD for word, but not
      // number for number: an in-pane bar says the pane's own frames, while
      // this bar speaks the reference clock (see the offsets above).
      const availability = !current.recordingAvailable
        ? ", no recorded history is currently available"
        : striped
          ? `, recorded frames ${inReference(current.recorded.lo)} to ` +
            `${inReference(current.recorded.hi)}; striped history outside that range is unavailable`
          : "";
      if (labelHtml + availability !== lastLabel) {
        lastLabel = labelHtml + availability;
        $("mp-frame").innerHTML = labelHtml;
        playheadEl.setAttribute("aria-valuemin", String(inReference(current.viewport.lo)));
        playheadEl.setAttribute("aria-valuemax", String(Math.max(viewportHiRef, selectedRef)));
        playheadEl.setAttribute("aria-valuenow", String(selectedRef));
        playheadEl.setAttribute(
          "aria-valuetext",
          `frame ${selectedRef} of ${viewportHiRef}` +
            (outside ? ", outside the frozen viewport" : "") +
            availability
        );
      }
      const pauseBtn = $btn("mp-pause");
      pauseBtn.textContent = current.paused ? "▶" : "⏸";
      pauseBtn.setAttribute("aria-label", current.paused ? "Resume" : "Pause");
      const cameraBtn = $btn("mp-camera");
      const cameraSeam = focusedPane ? seamOf(focusedPane.iframe) : null;
      const canDetach = cameraSeam?.canDetach() ?? false;
      const detached = cameraSeam?.detached() ?? false;
      cameraBtn.hidden = !canDetach;
      cameraBtn.disabled =
        pendingPointerLock !== null ||
        pendingDetached !== null;
      cameraBtn.textContent = detached ? "🔗" : "📷";
      cameraBtn.title = detached
        ? "Exit the focused debug camera"
        : "Debug camera — FPS in 3D, pan/zoom in 2D";
      cameraBtn.setAttribute("aria-label", cameraBtn.title);
      cameraBtn.setAttribute("aria-pressed", String(detached));
      cameraBtn.classList.toggle("on", detached);
      debugDrawer.hidden = !detached;
      if (detached && cameraSeam) {
        const debug = cameraSeam.debugCamera();
        const pan2d = debug.mode === 2;
        debugMode.value = String(debug.mode);
        debugMode.hidden = pan2d;
        debugPan2d.hidden = !pan2d;
        debugFov.hidden = pan2d;
        debugLensName.textContent = pan2d ? "Zoom" : "FOV";
        if (pan2d) {
          debugLens.value = debug.zoom2d > 0 ? `${debug.zoom2d.toFixed(2)}×` : "—";
        } else {
          const fov = debug.fov > 0 ? debug.fov : 60;
          if (document.activeElement !== debugFov) debugFov.value = String(fov);
          debugLens.value = `${Math.round(fov)}°`;
        }
        debugMaterial.value = String(debug.material);
        debugMaterialLabel.hidden = pan2d;
        debugPhysics.checked = debug.physics;
        debugPhysicsLabel.hidden = pan2d;
        debugFrustum.checked = debug.authoredFrustum;
        debugFrustumLabel.hidden = pan2d;
        debugGameUi.checked = debug.gameUi;
      }
      // A pane that boots while the session is parked must not run off on its
      // own: keep every seam's pause state converged on the focused pane's.
      // Idempotent, so this costs one boolean read per pane per frame.
      for (const seam of seams()) {
        if (seam !== primary && seam.paused() !== current.paused) seam.togglePause();
      }
      // Second ticks (60 frames), heavier every 5s. Positions track the moving
      // viewport each frame; nodes are recycled, only count changes touch DOM.
      const lo = current.viewport.lo;
      const firstTick = Math.ceil(lo / 60) * 60;
      const tickFrames: number[] = [];
      for (let f = firstTick; f <= current.viewport.hi; f += 60) tickFrames.push(f);
      while (ticksHost.children.length > tickFrames.length) ticksHost.lastChild?.remove();
      while (ticksHost.children.length < tickFrames.length) {
        ticksHost.appendChild(document.createElement("i"));
      }
      tickFrames.forEach((tickFrame, index) => {
        const node = ticksHost.children[index] as HTMLElement;
        node.className = tickFrame % 300 === 0 ? "mp-tick major" : "mp-tick";
        node.style.left = `${((tickFrame - lo) / span) * 100}%`;
      });
      // Event markers (input / reload / reload-error): the reload ticks are the
      // "source changes on the timeline" — every hot-swap is already recorded.
      const key =
        `${focusedPane?.id ?? ""}|` +
        current.eventMarkers
          .map(
            (marker) =>
              `${marker.id}@${marker.frame}:${marker.unit.toFixed(4)}:${marker.kind}:` +
              `${marker.category}:${marker.count}`
          )
          .join("|");
      if (key !== markerKey) {
        markerKey = key;
        hideTip();
        markersHost.replaceChildren(
          ...current.eventMarkers.map((marker) => {
            const tick = document.createElement("button");
            tick.className = `mp-evt ${marker.kind === "reload-error" ? "err" : marker.category}`;
            tick.style.left = `${marker.unit * 100}%`;
            const detail =
              `frame ${marker.frame} · ${marker.labels[0]}` +
              (marker.count > 1 ? ` · ${marker.count} events` : "");
            tick.setAttribute("aria-label", detail);
            tick.addEventListener("pointerdown", (event) => event.stopPropagation());
            tick.addEventListener("click", (event) => {
              event.stopPropagation();
              primarySeam()?.selectEvent(marker.id);
              // The markers are the focused pane's, so its frames are too.
              seekAll(toReference(marker.frame));
            });
            tick.addEventListener("mouseenter", () => showTip(marker.unit, detail));
            tick.addEventListener("mouseleave", hideTip);
            tick.addEventListener("focus", () => showTip(marker.unit, detail));
            tick.addEventListener("blur", hideTip);
            return tick;
          })
        );
      }
    }
    // Link state, straight from the coordinator's connection table — the seed
    // of the link chip becoming real. Only a session that HAS an authority
    // shows it: in a single-player example there is nothing to wait for.
    const links = serverPane ? net.connectionCounts() : null;
    for (const pane of allPanes()) {
      const seam = seamOf(pane.iframe);
      const frameNow = seam ? seam.frame() : null;
      const text = frameNow === null || frameNow === undefined ? "—" : String(frameNow);
      for (const label of pane.frameLabels) {
        if (label.textContent !== text) label.textContent = text;
      }
      // The same number the header drops when it clips (see the strip's note).
      if (pane.stripFrame && pane.stripFrame.textContent !== text) {
        pane.stripFrame.textContent = text;
      }
      pane.conn.hidden = !links;
      if (!links) continue;
      const count = links.get(pane.id) ?? 0;
      const linked =
        count > 0 ? (pane.role === "server" ? `${count} linked` : "linked") : "waiting";
      if (pane.connText.textContent !== linked) {
        pane.connDot.textContent = count > 0 ? "●" : "○";
        pane.connText.textContent = linked;
        pane.conn.dataset.linked = String(count > 0);
        pane.conn.title =
          count > 0
            ? "Connected through the host net coordinator (perfect link)"
            : "No connection yet — waiting for the server pane's Sub.listen";
      }
    }
  };
  // rAF, not a timer: while dragging the rail the playhead/label must track
  // the pointer at frame rate — a slow poll here reads as a sluggish sim.
  // The label and marker writes are change-guarded; the geometry writes are
  // plain style assignments (same shape as the in-frame scrubber's loop).
  let raf = 0;
  const tickLoop = () => {
    paint();
    // A FLIP transform moves a card's box without resizing anything, so the
    // ResizeObserver the graph normally re-measures from never fires: while a
    // view switch is in flight, re-measure every frame and the wires fly with
    // the cards.
    if (animating > 0 && grid.dataset.view === "network") graph.relayout();
    // One loop for the whole preview column: the network graph advances its
    // packets here rather than running a second rAF beside this one.
    graph.step();
    raf = requestAnimationFrame(tickLoop);
  };
  raf = requestAnimationFrame(tickLoop);

  return {
    // Mirror a fresh program load into the extra panes, and mount/remove the
    // server pane to match what the newly loaded example declares.
    setSrc(src, serverSrc = null) {
      for (const { pane, bridge } of mirrors) {
        bridge.reset();
        setPaneState(pane, "busy");
        pane.iframe.src = src;
      }
      if (!serverSrc) unmountServer();
      else if (serverPane) {
        serverPane.bridge.reset();
        setPaneState(serverPane.pane, "busy");
        serverPane.pane.iframe.src = serverSrc;
      } else {
        mountServer(serverSrc);
      }
      // A different program is a different session: the per-link packet totals
      // the reduced-motion badge shows start over with it.
      graph.resetCounts();
      updateChrome();
      paintAggregate();
    },
    // Mirror a debounced hot-reload push. The server pane is deliberately not
    // a target — the buffer being edited is the CLIENT entry (see mountServer).
    push(source) {
      for (const { bridge } of mirrors) bridge.push(source);
    },
    reset() {
      for (const { bridge } of mirrors) bridge.reset();
    },
    // The page's own status writer delegates here. It has ALREADY written the
    // pill (its single-client wording); with more than one pane the aggregate
    // overwrites it, and `lastMain` remembers the wording for a shrink to 1.
    aggregateStatus(state, text, detail) {
      mainState = state;
      mainDetail = detail;
      lastMain = { state, text, detail };
      setPaneState(panes[0], state, detail);
    },
    setCount,
    count: () => panes.length,
    destroy() {
      cancelAnimationFrame(raf);
      viewStrip.hidden = true;
      viewStrip.replaceChildren();
      graph.destroy();
      net.destroy();
      moduleScope.abort();
      unmountServer();
      while (mirrors.length) removeMirror();
    },
  };
}
