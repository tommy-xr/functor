// Multiplayer pane-grid PROTOTYPE for the sandbox (design:
// ~/notes/projects/functor/design-multiplayer-ide-frontend.md — this is the
// LIVE-mode skeleton of Addendum 2: hard per-client boundaries, iframe panes).
//
// The preview column becomes:
//
//   ┌ chrono bar ─ ⏸ ⏭ ═══rail═══╪══ 🔮 ⚙ | ⊞ tiled ▤ tabs ┐   (game side only —
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
//  - mirror panes boot with `?scrubber=hidden` (the seam without the bar);
//    pane 1 cannot re-boot (its model must survive), so while count > 1 its
//    in-frame bar is suppressed with a REVERSIBLE injected style — removed
//    the moment the count returns to 1, so single-client is exactly stock.
//    The single-client unification (chrono bar at every N) follows separately
//    with the a11y parity work it requires;
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
  step(): void;
  model(): { preview: { enabled: boolean; seconds: number; rate: number } };
  view(): TimelineView | null;
  setPreview(preview: { enabled?: boolean; seconds?: number; rate?: number }): void;
}

/** The slice of timeline-model's derived view the chrono bar renders. */
interface TimelineView {
  viewport: { lo: number; hi: number };
  playheadUnit: number;
  recordedStartUnit: number;
  recordedEndUnit: number;
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

const LINK_PRESETS: LinkProfile[] = [
  { name: "LAN", ms: 8, jitter: 2, loss: 0 },
  { name: "Wi-Fi", ms: 45, jitter: 12, loss: 1.2 },
  { name: "mobile", ms: 132, jitter: 38, loss: 3.4 },
  { name: "awful", ms: 400, jitter: 120, loss: 12 },
];

const HIDE_SCRUBBER_CSS = "#scrubber { display: none !important; }";

// Suppress (or restore) pane 1's own bottom-docked scrubber overlay. While
// count > 1 the host chrono bar is the one instrument; back at 1 the style is
// removed and the stock in-frame bar returns. The `__scrub` seam is live
// either way (it is how the chrono bar drives the pane).
const setInnerScrubberHidden = (iframe: HTMLIFrameElement, hidden: boolean) => {
  try {
    const doc = iframe.contentDocument;
    if (!doc) return;
    const existing = doc.getElementById("mp-hide-scrubber");
    if (hidden && !existing) {
      const style = doc.createElement("style");
      style.id = "mp-hide-scrubber";
      style.textContent = HIDE_SCRUBBER_CSS;
      (doc.head || doc.documentElement).appendChild(style);
    } else if (!hidden && existing) {
      existing.remove();
    }
  } catch {
    // A pane that is mid-navigation has no reachable document yet; the load
    // listener re-runs this.
  }
};

// Mirror panes get the honest mechanism: mountScrubber({ hidden }) via the
// player's ?scrubber=hidden — the seam mounts, the bar's DOM never does.
const withHiddenScrubber = (src: string) => {
  const url = new URL(src, window.location.href);
  url.searchParams.set("scrubber", "hidden");
  return url.toString();
};

const seamOf = (iframe: HTMLIFrameElement): ScrubSeam | null => {
  try {
    return (iframe.contentWindow as (Window & { __scrub?: ScrubSeam }) | null)?.__scrub ?? null;
  } catch {
    return null;
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
    <button class="mp-sbtn" id="mp-step" title="Step every client one frame">⏭</button>
    <span class="mp-rail" id="mp-rail" title="Drag to seek every client">
      <span class="mp-track"></span>
      <span class="mp-recorded" id="mp-recorded"></span>
      <span class="mp-played" id="mp-played"></span>
      <span class="mp-future" id="mp-future"></span>
      <span class="mp-ticks" id="mp-ticks"></span>
      <span class="mp-markers" id="mp-markers"></span>
      <span class="mp-playhead" id="mp-playhead"></span>
      <span class="mp-preview-handle" id="mp-preview-handle" title="Drag to stretch the extrapolation window"></span>
      <span class="mp-overflow" id="mp-overflow"></span>
      <span class="mp-evt-tip" id="mp-evt-tip" role="status"></span>
      <span class="mp-rail-label"><b>#f</b> <span id="mp-frame">—</span></span>
    </span>
    <button class="mp-sbtn" id="mp-extrap" title="Extrapolate: speculatively simulate forward from the parked frame">🔮</button>
    <details class="mp-adv" id="mp-adv">
      <summary title="Extrapolation settings">⚙</summary>
      <div class="mp-adv-pop">
        <label>show
          <select id="mp-adv-mode">
            <option value="1">trail</option><option value="2">strobe</option>
            <option value="3" selected>both</option><option value="4">ghost</option>
          </select>
        </label>
        <label>window <input id="mp-adv-win" type="number" step="0.5" min="0.5" max="5" value="2" />s</label>
        <label>rate <input id="mp-adv-rate" type="number" min="1" max="30" value="5" />/s</label>
      </div>
    </details>
    <span class="mp-viewseg" role="group" aria-label="Pane layout">
      <button id="mp-view-tiled" aria-pressed="true" title="Tiled: every client visible">⊞ tiled</button>
      <button id="mp-view-tabs" aria-pressed="false" title="Tabs: one client full-size (f)">▤ tabs</button>
    </span>`;

  // -------------------------------------------------------------- pane grid
  const tabsStrip = document.createElement("div");
  tabsStrip.className = "mp-tabs";
  tabsStrip.hidden = true;

  const grid = document.createElement("div");
  grid.className = "mp-grid";
  grid.dataset.view = "tiled";

  previewPane.prepend(chrono, tabsStrip);
  previewPane.appendChild(grid);

  const $ = (id: string) => chrono.querySelector(`#${id}`) as HTMLElement;
  const $btn = (id: string) => chrono.querySelector(`#${id}`) as HTMLButtonElement;
  const $input = (id: string) => chrono.querySelector(`#${id}`) as HTMLInputElement;

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

    iframe.addEventListener("load", () => syncInnerScrubber());
    shell.querySelector(".mp-pane-hd")!.addEventListener("mousedown", () => focusPane(index));
    tab.addEventListener("click", () => focusPane(index));
    buildLinkMenu(pane, shell.querySelector(".mp-link-host") as HTMLElement);
    panes.push(pane);
    return pane;
  };

  // Pane 1's in-frame scrubber shows exactly when it is the ONLY pane.
  function syncInnerScrubber() {
    setInnerScrubberHidden(panes[0].iframe, panes.length > 1);
  }

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
    });
    mirrors.push({ pane, bridge });
    // Runtime console lines from this mirror, prefixed with its identity.
    window.addEventListener("message", (event) => {
      const data = asPlayerMessage(event.data);
      if (!data || data.type !== "functor-lang-console") return;
      if (event.source !== iframe.contentWindow) return;
      statusBar.appendOutput(data.level, `[${label}] ${data.message}`, data.frame ?? null);
    });
    // A mirror added mid-session boots the served program, then catches up to
    // the (possibly edited) buffer; the bridge holds the push until ready.
    if (frame.getAttribute("src")) {
      iframe.src = withHiddenScrubber(frame.src);
      if (pushCurrent) bridge.push(getSource());
    }
  };

  const removeMirror = () => {
    const removed = mirrors.pop();
    if (!removed) return;
    removed.bridge.reset();
    removed.pane.shell.remove();
    removed.pane.tab.remove();
    panes.pop();
  };

  for (let i = 1; i < count; i++) addMirror(false);

  // ------------------------------------------------------------ focus model
  let focused = 0;
  function focusPane(index: number) {
    focused = index;
    for (const pane of panes) {
      const on = pane.index === index;
      pane.shell.classList.toggle("focus", on);
      pane.tab.classList.toggle("focus", on);
      (pane.shell.querySelector(".mp-you") as HTMLElement).hidden = !on;
    }
  }
  focusPane(0);

  // Every transient surface (link menus, the ⚙ popover) dismisses together —
  // popovers behave modally without blocking the page.
  const closePopovers = () => {
    for (const menu of document.querySelectorAll<HTMLElement>(".mp-link-menu")) {
      menu.hidden = true;
    }
    (chrono.querySelector("#mp-adv") as HTMLDetailsElement).open = false;
  };

  // Clicking into a pane's iframe gives it the real keyboard; follow that
  // with the focus chrome so "who has my keys" never lies. The blur doubles
  // as click-away dismissal for the popovers — an iframe click never reaches
  // this document's mousedown, but it always steals the window's focus.
  window.addEventListener("blur", () => {
    closePopovers();
    const active = document.activeElement;
    const pane = panes.find((candidate) => candidate.iframe === active);
    if (pane) focusPane(pane.index);
  });

  // Click-away anywhere in the page dismisses too (each link menu already
  // guards its own host; the ⚙ popover gets the same rule here).
  document.addEventListener("mousedown", (event) => {
    const adv = chrono.querySelector("#mp-adv") as HTMLDetailsElement;
    if (adv.open && !adv.contains(event.target as Node)) adv.open = false;
  });

  // Digits jump panes, [ ] cycle, f toggles tiled/tabs — but never while the
  // caret is in an editor or input (digits are text there, no exceptions).
  window.addEventListener("keydown", (event) => {
    const target = event.target as HTMLElement;
    if (event.key === "Escape") {
      closePopovers();
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
  });

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
    syncInnerScrubber();
  };

  // Live client-count change: grow or shrink the mirror set in place. Pane 1
  // is untouched, so its model (and the editor session) survive.
  const setCount = (n: number) => {
    const target = Math.max(1, Math.min(PLAYER_COLORS.length, Math.floor(n) || 1));
    while (panes.length < target) addMirror(true);
    while (panes.length > target) removeMirror();
    if (focused >= panes.length) focusPane(0);
    updateChrome();
    paintAggregate();
  };
  updateChrome();

  // -------------------------------------------------------------- link menu
  function buildLinkMenu(pane: Pane, host: HTMLElement) {
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
    document.addEventListener("mousedown", (event) => {
      if (!host.contains(event.target as Node)) menu.hidden = true;
    });
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
  let lastMain: { state: PaneState; text: string; detail: string } | null = null;
  const setPaneState = (pane: Pane, state: PaneState, detail = "") => {
    pane.state = state;
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
      setPill("error", "✕ build error", mainDetail);
    } else if (states.includes("busy")) {
      setPill("busy", "◐ reloading…", "");
    } else {
      setPill("live", `● ${panes.length} running`, mainDetail);
    }
  };

  // ------------------------------------------------------ chrono bar wiring
  const seams = () => panes.map((pane) => seamOf(pane.iframe)).filter((s): s is ScrubSeam => !!s);
  const primarySeam = () => seamOf(panes[focused].iframe) ?? seams()[0] ?? null;

  $btn("mp-pause").addEventListener("click", () => {
    const primary = primarySeam();
    if (!primary) return;
    const desired = !primary.paused();
    for (const seam of seams()) if (seam.paused() !== desired) seam.togglePause();
  });
  $btn("mp-step").addEventListener("click", () => {
    for (const seam of seams()) seam.step();
  });
  // The speculative preview (🔮): each pane simulates forward from its parked
  // frame under the current code, replaying recorded input. Broadcast like
  // pause — every pane extrapolates its own sim.
  $btn("mp-extrap").addEventListener("click", () => {
    const primary = primarySeam();
    if (!primary) return;
    const enabled = !primary.model().preview.enabled;
    for (const seam of seams()) seam.setPreview({ enabled });
  });

  // ⚙ advanced: window/rate broadcast through the seam. The ghost-mode select
  // isn't in the seam's model, so it reaches into each pane's own (hidden)
  // scrubber select — a prototype-only shortcut, like the CSS injection.
  const advWin = $input("mp-adv-win");
  const advRate = $input("mp-adv-rate");
  const advMode = chrono.querySelector("#mp-adv-mode") as HTMLSelectElement;
  const pushAdv = () => {
    if (!advWin.validity.valid || !advRate.validity.valid) return;
    for (const seam of seams()) {
      seam.setPreview({ seconds: advWin.valueAsNumber, rate: advRate.valueAsNumber });
    }
  };
  advWin.addEventListener("input", pushAdv);
  advRate.addEventListener("input", pushAdv);
  advMode.addEventListener("change", () => {
    for (const pane of panes) {
      try {
        const select = pane.iframe.contentDocument?.getElementById(
          "scrub-mode"
        ) as HTMLSelectElement | null;
        if (select) {
          select.value = advMode.value;
          select.dispatchEvent(new Event("change"));
        }
      } catch {
        // a pane mid-navigation has no reachable document
      }
    }
  });

  // The pink preview handle: drag to stretch the extrapolation window, exactly
  // like the old in-frame scrubber's second slider.
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
    const settled = primarySeam()?.model().preview.seconds;
    if (settled !== undefined) advWin.value = String(settled);
  });
  const endPreviewDrag = () => {
    previewDrag = null;
  };
  previewHandle.addEventListener("pointerup", endPreviewDrag);
  previewHandle.addEventListener("pointercancel", endPreviewDrag);

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

  // The rail mirrors the FOCUSED pane (each pane is an independent sim in the
  // prototype, so one authoritative rail per focus is the honest picture; the
  // sim substrate replaces this with server time).
  const markersHost = $("mp-markers");
  const ticksHost = $("mp-ticks");
  let markerKey = "";
  let lastLabel = "";
  const paint = () => {
    const primary = primarySeam();
    const current = primary?.view();
    chrono.classList.toggle("dormant", !current);
    if (primary && current) {
      const span = Math.max(current.viewport.hi - current.viewport.lo, 1);
      $("mp-played").style.width = `${Math.min(current.playheadUnit, 1) * 100}%`;
      $("mp-recorded").style.left = `${current.recordedStartUnit * 100}%`;
      $("mp-recorded").style.width =
        `${Math.max(current.recordedEndUnit - current.recordedStartUnit, 0) * 100}%`;
      $("mp-playhead").style.left = `${current.playheadUnit * 100}%`;
      const previewOn = primary.model().preview.enabled;
      const future = $("mp-future");
      future.style.left = `${current.playheadUnit * 100}%`;
      future.style.width = previewOn
        ? `${Math.max(current.previewEndUnit - current.playheadUnit, 0) * 100}%`
        : "0";
      previewHandle.style.display = previewOn ? "block" : "none";
      previewHandle.style.left = `${current.previewEndUnit * 100}%`;
      previewHandle.classList.toggle("clipped", current.previewClippedFrames > 0);
      const overflow = $("mp-overflow");
      overflow.style.display = previewOn && current.previewClippedFrames > 0 ? "block" : "none";
      overflow.textContent = `+${current.previewClippedFrames}`;
      $btn("mp-extrap").classList.toggle("on", previewOn);
      $btn("mp-extrap").setAttribute("aria-pressed", String(previewOn));
      const labelHtml =
        `${current.selectedFrame}` +
        (previewOn ? ` <span class="fut">+${current.previewFrames}</span>` : "") +
        ` / ${Math.round(current.viewport.hi)}`;
      if (labelHtml !== lastLabel) {
        lastLabel = labelHtml;
        $("mp-frame").innerHTML = labelHtml;
      }
      $btn("mp-pause").textContent = current.paused ? "▶" : "⏸";
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
      const key = current.eventMarkers
        .map((marker) => `${marker.id}:${marker.unit.toFixed(4)}:${marker.kind}`)
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
              for (const seam of seams()) seam.seek(marker.frame);
            });
            tick.addEventListener("mouseenter", () => showTip(marker.unit, detail));
            tick.addEventListener("mouseleave", hideTip);
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
  // All DOM writes above are change-guarded, so the steady-state cost is
  // reads only (same shape as the in-frame scrubber's own rAF loop).
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
        pane.iframe.src = withHiddenScrubber(src);
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
    },
  };
}
