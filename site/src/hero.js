// The landing hero's live code panel: a small editor mounted over ONE region
// of examples/hero.fun (the `dot` def, between `// <editable>` sentinels).
// Edit the region and the running grid hot-swaps with the model preserved —
// the wave keeps rolling — then drag the timeline back through the change.
//
// This is the light sibling of sandbox.js: it reuses the same editor↔player
// seam (player-bridge.js) but a stripped editor (mini-editor.js — no basicSetup
// or lint) so the landing bundle stays tiny. It NEVER reloads the iframe; every
// edit is a source push, because state preservation IS the demo.

import { createMiniEditor } from "./mini-editor.js";
import { PlayerBridge } from "./player-bridge.js";

const HERO_URL = "examples/hero.fun";
const OPEN = "// <editable>";
const CLOSE = "// </editable>";

const frame = document.querySelector(".hero-scene");
const mount = document.getElementById("hero-editor");
const card = document.querySelector(".hero-card");

// A small, unobtrusive status dot pinned to the card corner: green when the
// last edit is live, red on a broken edit (the old program keeps running).
// The full message lives in its tooltip and the __hero.status() seam.
const dot = document.createElement("div");
dot.className = "hero-status";
card.appendChild(dot);

let statusState = { state: "busy", message: "" };
const setStatus = (state, message = "") => {
  statusState = { state, message };
  dot.dataset.state = state;
  dot.title = message || state;
};

// The file split around the editable region. prefix + region + suffix always
// reconstructs the exact served source, so a push preserves the sentinels
// (and thus keeps the grid a byte-valid program on the next reload).
let prefix = "";
let suffix = "";
let region = "";

const fullProgram = () => prefix + region + suffix;

const bridge = new PlayerBridge(frame, {
  onReloading: () => setStatus("busy"),
  onLive: () => setStatus("live", "live"),
  onResult: (ok, message) =>
    ok ? setStatus("live", message) : setStatus("error", message),
});

let editor = null;

const boot = async () => {
  let source;
  try {
    const response = await fetch(HERO_URL);
    if (!response.ok) return;
    source = await response.text();
  } catch {
    return; // Fail soft: no panel, the scene still runs on its own.
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
  setStatus("live", "live");
};

boot();

// Test seam for the headless e2e (e2e/site-sandbox.mjs), on the landing window.
window.__hero = {
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
};
