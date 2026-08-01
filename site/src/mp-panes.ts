// Multiplayer pane-grid PROTOTYPE for the sandbox (design:
// ~/notes/projects/functor/design-multiplayer-ide-frontend.md — this is the
// LIVE-mode skeleton of Addendum 2: hard per-client boundaries, iframe panes).
//
// The preview column becomes:
//
//   ┌ chrono bar ─ ⏸ ⏭ ═══rail═══╪══ 🔮 | ⊞ tiled ▤ tabs ┐   (game side only —
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

/** The shared scrubber's `window.__scrub` seam (runtime/functor-runtime-web/scrubber.js). */
interface ScrubSeam {
  paused(): boolean;
  frame(): number;
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

interface Pane {
  index: number;
  /** Aborts every listener this pane registered outside its own subtree. */
  scope: AbortController;
  iframe: HTMLIFrameElement;
  shell: HTMLElement;
  tab: HTMLButtonElement;
  state: PaneState;
  link: LinkProfile;
  frameLabels: HTMLElement[];
  dots: HTMLElement[];
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
}

export interface MultiplayerPanes {
  setSrc(src: string): void;
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
}: MultiplayerPanesOptions): MultiplayerPanes {
  const previewPane = frame.closest(".preview-pane") as HTMLElement;
  previewPane.classList.add("mp");

  // ------------------------------------------------------------- chrono bar
  const chrono = document.createElement("div");
  chrono.className = "mp-chrono";
  chrono.innerHTML = `
    <button class="mp-sbtn" id="mp-pause" title="Pause / resume every client">⏸</button>
    <button class="mp-sbtn" id="mp-camera" title="Open the focused debug camera" hidden>📷</button>
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
    <button class="mp-sbtn" id="mp-extrap" title="Extrapolate: speculatively simulate forward from the parked frame">🔮</button>
    <span class="mp-viewseg" role="group" aria-label="Pane layout">
      <button id="mp-view-tiled" aria-pressed="true" title="Tiled: every client visible">⊞ tiled</button>
      <button id="mp-view-tabs" aria-pressed="false" title="Tabs: one client full-size (f)">▤ tabs</button>
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

  previewPane.prepend(chrono, debugDrawer, tabsStrip);
  previewPane.appendChild(grid);

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

  const panes: Pane[] = [];
  const makePane = (index: number, iframe: HTMLIFrameElement): Pane => {
    const color = PLAYER_COLORS[index % PLAYER_COLORS.length];
    const shell = document.createElement("div");
    shell.className = "mp-pane";
    shell.style.setProperty("--pc", color);
    shell.innerHTML = `
      <div class="mp-pane-hd">
        <span class="mp-digit">${index + 1}</span>
        <span class="mp-role">client</span>
        <span class="mp-link-host">
          <button class="mp-link-chip"
            title="Link impairment for this client (prototype — recorded per client; applies once the netsim transport lands)">⇅ Wi-Fi ▾</button>
        </span>
        <span class="mp-hd-r">
          <span class="mp-you" hidden>⌨ you</span>
          <span class="mp-pf"><b>#f</b> <span class="mp-pf-n">—</span></span>
          <span class="mp-st" data-state="busy"></span>
        </span>
      </div>
      <div class="mp-pane-body"></div>
      <div class="mp-pane-err" hidden></div>`;
    shell.querySelector(".mp-pane-body")!.appendChild(iframe);
    grid.appendChild(shell);

    const tab = document.createElement("button");
    tab.className = "mp-tab";
    tab.style.setProperty("--pc", color);
    tab.innerHTML = `<span class="mp-digit">${index + 1}</span> client
      <span class="mp-pf"><b>#f</b> <span class="mp-pf-n">—</span></span>
      <span class="mp-st" data-state="busy"></span>`;
    tabsStrip.appendChild(tab);

    const pane: Pane = {
      index,
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
      errStrip: shell.querySelector(".mp-pane-err") as HTMLElement,
    };

    shell.querySelector(".mp-pane-hd")!.addEventListener("mousedown", () => focusPane(index));
    tab.addEventListener("click", () => focusPane(index));
    buildLinkMenu(pane, shell.querySelector(".mp-link-host") as HTMLElement, pane.scope.signal);
    panes.push(pane);
    return pane;
  };

  // Pane 1 wraps the page's existing #player (lang-intel, the live trace, and
  // the primary bridge keep talking to it untouched). Panes 2..N are mirrors,
  // added and removed LIVE — pane 1's iframe never moves again after this
  // wrap, so its running model survives every count change.
  makePane(0, frame);
  const mirrors: { pane: Pane; bridge: PlayerBridge }[] = [];

  const addMirror = (pushCurrent: boolean) => {
    const index = panes.length;
    const label = `client ${index + 1}`;
    const iframe = document.createElement("iframe");
    iframe.title = `${label} preview`;
    iframe.allow = "pointer-lock";
    const pane = makePane(index, iframe);
    const bridge = new PlayerBridge(iframe, {
      onReloading: () => setPaneState(pane, "busy"),
      onLive: () => setPaneState(pane, "live"),
      onResult: (ok, message) => {
        setPaneState(pane, ok ? "live" : "error", message);
        if (!ok) statusBar.appendOutput("error", `[${label}] ${message}`);
      },
      signal: pane.scope.signal,
    });
    mirrors.push({ pane, bridge });
    // Runtime console lines from this mirror, prefixed with its identity.
    window.addEventListener(
      "message",
      (event) => {
        const data = asPlayerMessage(event.data);
        if (!data || data.type !== "functor-lang-console") return;
        if (event.source !== iframe.contentWindow) return;
        statusBar.appendOutput(data.level, `[${label}] ${data.message}`, data.frame ?? null);
      },
      { signal: pane.scope.signal }
    );
    // A mirror added mid-session boots the served program, then catches up to
    // the (possibly edited) buffer; the bridge holds the push until ready.
    if (frame.getAttribute("src")) {
      iframe.src = frame.src; // already carries ?scrubber=hidden (the page adds it)
      if (pushCurrent) bridge.push(getSource());
    }
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
  let focused = 0;
  function focusPane(index: number) {
    if (index !== focused) {
      const previous = panes[focused];
      if (previous && seamOf(previous.iframe)?.detached()) {
        exitPanePointerLock(previous.iframe);
      }
    }
    focused = index;
    for (const pane of panes) {
      const on = pane.index === index;
      pane.shell.classList.toggle("focus", on);
      pane.tab.classList.toggle("focus", on);
      (pane.shell.querySelector(".mp-you") as HTMLElement).hidden = !on;
    }
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
    const pane = panes.find((candidate) => candidate.iframe === active);
    if (pane) focusPane(pane.index);
  }, { signal: moduleScope.signal });


  // Digits jump panes, [ ] cycle, f toggles tiled/tabs — but never while the
  // caret is in an editor or input (digits are text there, no exceptions).
  window.addEventListener("keydown", (event) => {
    const target = event.target as HTMLElement;
    if (event.key === "Escape") {
      closePopovers();
      primarySeam()?.selectEvent(null);
      return;
    }
    if (target.closest?.(".cm-editor") || /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName)) return;
    if (/^[1-9]$/.test(event.key)) {
      const index = Number(event.key) - 1;
      if (index < panes.length) focusPane(index);
    } else if (event.key === "[" || event.key === "]") {
      const delta = event.key === "]" ? 1 : -1;
      focusPane((focused + delta + panes.length) % panes.length);
    } else if (event.key === "f") {
      setView(grid.dataset.view === "tiled" ? "tabs" : "tiled");
    }
  }, { signal: moduleScope.signal });

  // ------------------------------------------------------- tiled / tabs view
  const setView = (view: "tiled" | "tabs", force = false) => {
    if (panes.length === 1 && !force) return;
    grid.dataset.view = view;
    tabsStrip.hidden = view !== "tabs";
    $btn("mp-view-tiled").setAttribute("aria-pressed", String(view === "tiled"));
    $btn("mp-view-tabs").setAttribute("aria-pressed", String(view === "tabs"));
  };
  $btn("mp-view-tiled").addEventListener("click", () => setView("tiled"));
  $btn("mp-view-tabs").addEventListener("click", () => setView("tabs"));

  // Everything on the chrono bar that depends on how many panes exist.
  // (CSS keyed on data-count hides the pane header / 🔮 / view toggle.)
  const updateChrome = () => {
    const single = panes.length === 1;
    previewPane.dataset.count = String(panes.length);
    $btn("mp-view-tiled").disabled = single;
    $btn("mp-view-tabs").disabled = single;
    if (single) setView("tiled", true);
  };

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
    if (focused >= panes.length) focusPane(0);
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
      <p class="mp-link-note">prototype: recorded per client, applied when the netsim transport lands</p>`;
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
    if (panes.length === 1) {
      if (lastMain) setPill(lastMain.state, lastMain.text, lastMain.detail);
      return;
    }
    const states = [mainState, ...panes.slice(1).map((pane) => pane.state)];
    if (states.includes("error")) {
      const failing = panes.find((pane) => pane.state === "error");
      setPill("error", "✕ build error", (failing && paneDetails.get(failing)) || mainDetail);
    } else if (states.includes("busy")) {
      setPill("busy", "◐ reloading…", "");
    } else {
      setPill("live", `● ${panes.length} running`, mainDetail);
    }
  };

  // ------------------------------------------------------ chrono bar wiring
  const seams = () => panes.map((pane) => seamOf(pane.iframe)).filter((s): s is ScrubSeam => !!s);
  const primarySeam = () => seamOf(panes[focused].iframe) ?? seams()[0] ?? null;
  let pendingDetached:
    | { seam: ScrubSeam; generation: number; iframe: HTMLIFrameElement }
    | null = null;
  let pendingPointerLock: { seam: ScrubSeam; iframe: HTMLIFrameElement } | null = null;
  const reconcilePendingCamera = () => {
    if (
      pendingPointerLock !== null &&
      (!pendingPointerLock.iframe.isConnected ||
        seamOf(pendingPointerLock.iframe) !== pendingPointerLock.seam ||
        panes[focused]?.iframe !== pendingPointerLock.iframe)
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
    if (!detached || panes[focused]?.iframe !== request.iframe) {
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
    const iframe = panes[focused].iframe;
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
        panes[focused]?.iframe !== iframe
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
    for (const seam of seams()) seam.seek(target);
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
    for (const seam of seams()) seam.seek(Math.round(target));
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
    const primary = primarySeam();
    const current = primary?.view();
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
      const labelHtml =
        `${current.selectedFrame}` +
        (outside ? ` <span class="out">outside</span>` : "") +
        (previewOn ? ` <span class="fut">+${current.previewFrames}</span>` : "") +
        ` / ${Math.round(current.viewport.hi)}`;
      // The stripes' availability note is ARIA-only, but it shares the label's
      // change guard so it repaints exactly when the label does.
      // Mirrors timeline-model's describeRecordedAvailability exactly.
      const availability = !current.recordingAvailable
        ? ", no recorded history is currently available"
        : striped
          ? `, recorded frames ${Math.round(current.recorded.lo)} to ` +
            `${Math.round(current.recorded.hi)}; striped history outside that range is unavailable`
          : "";
      if (labelHtml + availability !== lastLabel) {
        lastLabel = labelHtml + availability;
        $("mp-frame").innerHTML = labelHtml;
        playheadEl.setAttribute("aria-valuemin", String(Math.round(current.viewport.lo)));
        playheadEl.setAttribute(
          "aria-valuemax",
          String(Math.max(Math.round(current.viewport.hi), current.selectedFrame))
        );
        playheadEl.setAttribute("aria-valuenow", String(current.selectedFrame));
        playheadEl.setAttribute(
          "aria-valuetext",
          `frame ${current.selectedFrame} of ${Math.round(current.viewport.hi)}` +
            (outside ? ", outside the frozen viewport" : "") +
            availability
        );
      }
      const pauseBtn = $btn("mp-pause");
      pauseBtn.textContent = current.paused ? "▶" : "⏸";
      pauseBtn.setAttribute("aria-label", current.paused ? "Resume" : "Pause");
      const cameraBtn = $btn("mp-camera");
      const cameraSeam = seamOf(panes[focused].iframe);
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
        `${focused}|` +
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
              for (const seam of seams()) seam.seek(marker.frame);
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
    for (const pane of panes) {
      const seam = seamOf(pane.iframe);
      const frameNow = seam ? seam.frame() : null;
      const text = frameNow === null || frameNow === undefined ? "—" : String(frameNow);
      for (const label of pane.frameLabels) {
        if (label.textContent !== text) label.textContent = text;
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
    raf = requestAnimationFrame(tickLoop);
  };
  raf = requestAnimationFrame(tickLoop);

  return {
    // Mirror a fresh program load into the extra panes.
    setSrc(src) {
      for (const { pane, bridge } of mirrors) {
        bridge.reset();
        setPaneState(pane, "busy");
        pane.iframe.src = src;
      }
    },
    // Mirror a debounced hot-reload push.
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
      moduleScope.abort();
      while (mirrors.length) removeMirror();
    },
  };
}
