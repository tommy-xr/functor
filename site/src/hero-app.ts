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
// and dynamic-imports this module, so all of it — CodeMirror, the bridge —
// downloads after the static shell has painted.

import type { EditorView } from "@codemirror/view";
import { createMiniEditor } from "./mini-editor.js";
import {
  createEditorKeybindingsController,
  editorKeybindingsButtonPresentation,
} from "./editor-keybindings.js";
import type { EditorKeybindings, EditorKeybindingsState } from "./editor-keybindings.js";
import { PlayerBridge } from "./player-bridge.js";

/** The status dot's three states — also its `data-state` attribute value. */
type HeroState = "busy" | "live" | "error";

interface HeroStatus {
  state: HeroState;
  message: string;
}

/** The landing page's e2e seam (driven by e2e/site-sandbox.mjs). */
interface HeroSeam {
  setRegion(src: string): void;
  region: () => string;
  status: () => HeroStatus;
  keybindings: () => EditorKeybindingsState;
  setKeybindings: (mode: EditorKeybindings) => Promise<void>;
}

const HERO_URL = "examples/hero.fun";
const OPEN = "// <editable>";
const CLOSE = "// </editable>";

const frame = document.querySelector<HTMLIFrameElement>(".hero-scene")!;
const mount = document.getElementById("hero-editor")!;
const card = document.querySelector(".hero-card")!;
const editorKeybindings = createEditorKeybindingsController({
  showStatus: false,
  includeDrawSelection: true,
});

// A small, unobtrusive status dot pinned to the card corner: green when the
// last edit is live, red on a broken edit (the old program keeps running).
// The full message lives in its tooltip and the __hero.status() seam.
const dot = document.createElement("div");
dot.className = "hero-status";
card.appendChild(dot);

let statusState: HeroStatus = { state: "busy", message: "" };
const setStatus = (state: HeroState, message = "") => {
  statusState = { state, message };
  dot.dataset.state = state;
  dot.title = message || state;
};

// The boot loader (static markup in index.html) evaporates only once the card
// has BOTH its pixels and its final shape: the player's ready handshake AND a
// settled editor panel, which grows the card by its own 172px.
//
// Both conditions, not just the player, because this module is now lazy. At
// base it was eager, so the panel had always mounted (a same-origin fetch)
// long before the player's handshake — the ordering styles.css relies on to
// keep the panel out of the hit region while it is occluded. Deferred, the
// player is often ready FIRST, and dismissing on that alone would lift the
// overlay and only then pop the panel in: a layout shift in full view instead
// of one hidden behind the loader. `editorSettled` is set on every exit path
// of boot(), including its failures, so a missing hero.fun can only cost the
// panel, never leave the loader spinning over a scene that is already running.
let playerReady = false;
let editorSettled = false;
const dismissBootLoader = () => {
  if (!playerReady || !editorSettled) return;
  document.querySelector("[data-fn-boot]")?.classList.add("is-done");
};
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
    playerReady = true;
    dismissBootLoader();
    setStatus("live", "live");
  },
  onResult: (ok, message) => {
    playerReady = true;
    dismissBootLoader();
    ok ? setStatus("live", message) : setStatus("error", message);
  },
});

let editor: EditorView | null = null;

const mountKeybindingsButton = () => {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "editor-keybindings-toggle hero-editor-keybindings";
  button.setAttribute("aria-live", "polite");
  button.addEventListener("click", () => {
    const mode = editorKeybindings.state.getSnapshot().mode;
    void editorKeybindings.setMode(mode === "vim" ? "standard" : "vim");
  });
  const render = () => {
    const state = editorKeybindings.state.getSnapshot();
    const presentation = editorKeybindingsButtonPresentation(state);
    button.textContent = presentation.text;
    button.setAttribute("aria-pressed", String(presentation.enabled));
    button.setAttribute("aria-busy", String(state.loading));
    button.title = presentation.title;
  };
  editorKeybindings.state.subscribe(render);
  render();
  // Keep the control inside the editor's reserved footer. The whole editor is
  // visibility-hidden during boot, so this also removes the occluded button
  // from hit-testing and the tab order until the hero is ready.
  mount.appendChild(button);
};

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
  mountKeybindingsButton();
  editor = createMiniEditor({
    parent: mount,
    doc: region,
    keybindings: editorKeybindings,
    onChange: (src) => {
      region = src;
      bridge.push(fullProgram());
    },
  });
  // No setStatus here: the dot stays busy until the player announces ready
  // (bridge onLive) or the first push result comes back.
};

// Settled means "the card will not change shape again", which every exit path
// of boot() reaches — including its failures, where the panel simply never
// appears. Finalizing here rather than at each `return` is what makes that
// exhaustive.
void boot().finally(() => {
  editorSettled = true;
  dismissBootLoader();
});

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
  status: () => ({ ...statusState }),
  keybindings: () => editorKeybindings.state.getSnapshot(),
  setKeybindings: (mode) => editorKeybindings.setMode(mode),
};
