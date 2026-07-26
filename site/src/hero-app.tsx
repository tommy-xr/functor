// The landing hero's live code panel: a small editor mounted over ONE region
// of examples/hero.fun (the `dot` def, between `// <editable>` sentinels).
// Edit the region and the running grid hot-swaps with the model preserved —
// the wave keeps rolling — then drag the timeline back through the change.
//
// This is the light sibling of sandbox.tsx: it reuses the same editor↔player
// seam (player-bridge.ts) but a stripped editor (mini-editor.ts — no basicSetup
// or lint). It NEVER reloads the iframe; every edit is a source push, because
// state preservation IS the demo.
//
// It is NOT on the landing page's critical path: src/hero.ts is the eager entry
// and dynamic-imports this module, so all of it — CodeMirror, the bridge,
// React — downloads after the static shell has painted.

import type { EditorView } from "@codemirror/view";
import { createRoot } from "react-dom/client";
import { createMiniEditor } from "./mini-editor.js";
import { PlayerBridge } from "./player-bridge.js";
import { createStore } from "./store.js";
import { HeroStatusDot } from "./components/HeroStatusDot.js";
import type { HeroState, HeroStatus } from "./components/HeroStatusDot.js";

/** The landing page's e2e seam (driven by e2e/site-sandbox.mjs). */
interface HeroSeam {
  setRegion(src: string): void;
  region: () => string;
  status: () => HeroStatus;
}

const HERO_URL = "examples/hero.fun";
const OPEN = "// <editable>";
const CLOSE = "// </editable>";

const frame = document.querySelector<HTMLIFrameElement>(".hero-scene")!;
const mount = document.getElementById("hero-editor")!;
const card = document.querySelector(".hero-card")!;

// A small, unobtrusive status dot pinned to the card corner — the hero's only
// chrome, rendered as a React island over this store. The store is also what
// the __hero.status() seam reads, so the seam never waits on a render.
// The slot is an empty block whose only child is absolutely positioned, so it
// adds no layout to the card (same DOM shape as the hand-built dot).
const statusStore = createStore<HeroStatus>({ state: "busy", message: "" });
const statusSlot = document.createElement("div");
card.appendChild(statusSlot);
createRoot(statusSlot).render(<HeroStatusDot store={statusStore} />);

const setStatus = (state: HeroState, message = "") => {
  statusStore.set({ state, message });
};

// The boot loader (static markup in index.html) evaporates when the PLAYER
// reports in — never on any other status change, since a failure to fetch the
// editable region says nothing about whether the card has pixels yet. One
// class toggle; the 620ms evaporate is all CSS.
const dismissBootLoader = () =>
  document.querySelector("[data-fn-boot]")?.classList.add("is-done");
// Busy until the player's ready handshake: the bridge's onLive (or a
// successful onResult) is what turns the dot green, never the mount itself.
setStatus("busy", "loading…");

// The file split around the editable region. prefix + region + suffix always
// reconstructs the exact served source, so a push preserves the sentinels
// (and thus keeps the grid a byte-valid program on the next reload).
let prefix = "";
let suffix = "";
let region = "";

const fullProgram = () => prefix + region + suffix;

const bridge = new PlayerBridge(frame, {
  onReloading: () => setStatus("busy"),
  onLive: () => {
    dismissBootLoader();
    setStatus("live", "live");
  },
  onResult: (ok, message) => {
    dismissBootLoader();
    ok ? setStatus("live", message) : setStatus("error", message);
  },
});

let editor: EditorView | null = null;

const boot = async () => {
  let source: string;
  try {
    const response = await fetch(HERO_URL);
    if (!response.ok) {
      setStatus("error", `cannot fetch ${HERO_URL}: HTTP ${response.status}`);
      return; // No editor panel; the scene may still run on its own.
    }
    source = await response.text();
  } catch (err) {
    setStatus("error", `cannot fetch ${HERO_URL}: ${err}`);
    return;
  }

  const open = source.indexOf(OPEN);
  const close = source.indexOf(CLOSE, open + OPEN.length);
  if (open !== -1 && close !== -1) {
    // Region = everything on the lines strictly between the sentinels; the
    // sentinels themselves live in prefix/suffix so they never get edited away.
    const regionStart = source.indexOf("\n", open) + 1;
    prefix = source.slice(0, regionStart);
    region = source.slice(regionStart, close);
    suffix = source.slice(close);
  } else {
    // Sentinels missing: fail soft to editing the whole file.
    prefix = "";
    region = source;
    suffix = "";
  }

  mount.hidden = false;
  editor = createMiniEditor({
    parent: mount,
    doc: region,
    onChange: (src) => {
      region = src;
      bridge.push(fullProgram());
    },
  });
  // No setStatus here: the dot stays busy until the player announces ready
  // (bridge onLive) or the first push result comes back.
};

boot();

// Test seam for the headless e2e (e2e/site-sandbox.mjs), on the landing window.
(window as Window & { __hero?: HeroSeam }).__hero = {
  setRegion(src) {
    if (editor) {
      editor.dispatch({
        changes: { from: 0, to: editor.state.doc.length, insert: src },
      });
    } else {
      region = src;
      bridge.push(fullProgram());
    }
  },
  region: () => region,
  status: () => ({ ...statusStore.getSnapshot() }),
};
