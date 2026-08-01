// The landing hero's live code panel: a small editor mounted over a real excerpt
// of examples/mario/game.fun (jump tuning + the world-composition function).
// Behind the boot loader, the page queues the checked-in jump.script at the
// browser runtime's fixed-step input boundary, waits for its authoritative
// timeline markers, then pauses and seeks just before takeoff. The visitor's
// first click on 🔮 reveals the jump.
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
  staged: () => boolean;
}

interface ScriptedInput {
  frame: number;
  code: number;
  label: string;
  isDown: boolean;
}

interface HeroScrubEvent {
  frame: number;
  label: string;
}

interface HeroScrubSeam {
  paused(): boolean;
  frame(): number;
  seek(frame: number): void;
  scheduleKeyInputs(inputs: Array<{ frame: number; code: number; isDown: boolean }>): boolean;
  togglePause(): void;
  events(): HeroScrubEvent[];
  setPreview(preview: {
    enabled?: boolean;
    seconds?: number;
    rate?: number;
    mode?: number;
  }): void;
}

type HeroPlayerWindow = Window & { __scrub?: HeroScrubSeam };

const HERO_URL = "examples/mario.fun";
const JUMP_SCRIPT_URL = "examples/mario.jump.script";
const OPEN = "// <editable>";
const CLOSE = "// </editable>";
const COLD_START_SETTLE_MS = 300;
const PREVIEW_SECONDS = 0.9;
const PREVIEW_RATE = 8;
const PREVIEW_MODE_BOTH = 3;

const delay = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));
const nextPaint = () => new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));

// The script remains the source of truth for both the deterministic desktop
// capture and this browser staging pass. The landing demo deliberately accepts
// only the two controls it needs, so an edited script fails loud instead of
// silently driving a different game.
const parseJumpScript = (source: string): ScriptedInput[] => {
  const keyCodes: Record<string, number> = {
    Right: 30,
    Up: 27,
  };
  const inputs: ScriptedInput[] = [];
  for (const [index, raw] of source.split(/\r?\n/).entries()) {
    const line = raw.split("#", 1)[0].trim();
    if (!line) continue;
    const parts = line.split(/\s+/);
    if (parts.length !== 3) {
      throw new Error(`${JUMP_SCRIPT_URL}:${index + 1}: expected frame, key, and down|up`);
    }
    const [frameText, key, direction] = parts;
    const scriptFrame = Number(frameText);
    const code = keyCodes[key];
    if (!Number.isInteger(scriptFrame) || scriptFrame < 0 || code === undefined) {
      throw new Error(`${JUMP_SCRIPT_URL}:${index + 1}: unsupported input \`${line}\``);
    }
    if (direction !== "down" && direction !== "up") {
      throw new Error(`${JUMP_SCRIPT_URL}:${index + 1}: expected down|up`);
    }
    inputs.push({
      frame: scriptFrame,
      code,
      label: `${key} ${direction}`,
      isDown: direction === "down",
    });
  }
  if (inputs.length === 0) throw new Error(`${JUMP_SCRIPT_URL}: no scripted inputs`);
  return inputs.sort((a, b) => a.frame - b.frame);
};

const frame = document.querySelector<HTMLIFrameElement>(".hero-scene")!;
const editorShell = document.getElementById("hero-editor")!;
const mount = editorShell.querySelector<HTMLElement>("[data-hero-editor]")!;

// A small, unobtrusive status dot in the excerpt label: green when the
// last edit is live, red on a broken edit (the old program keeps running).
// The full message lives in its tooltip and the __hero.status() seam.
const dot = document.querySelector<HTMLElement>("[data-hero-status]")!;

let statusState: HeroStatus = { state: "busy", message: "" };
const setStatus = (state: HeroState, message = "") => {
  statusState = { state, message };
  dot.dataset.state = state;
  dot.title = message || state;
};

// The boot loader (static markup in index.html) evaporates only once the card
// has BOTH its pixels and its final shape: the player's ready handshake AND a
// settled editor panel, which grows the card by its own fixed height.
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
let demoSettled = false;
let demoSeeded = false;
let staging = false;
let scriptedInputs: ScriptedInput[] | null = null;
const dismissBootLoader = () => {
  if (!playerReady || !editorSettled || !demoSettled) return;
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

const seedJumpHistory = async (inputs: ScriptedInput[]) => {
  const player = frame.contentWindow as HeroPlayerWindow | null;
  const scrub = player?.__scrub;
  if (!player || !scrub) throw new Error("the platformer timeline did not become ready");

  // Let first-frame compilation and asset setup finish before starting the
  // script's frame-relative schedule.
  await delay(COLD_START_SETTLE_MS);

  // Queue the desktop script at the runtime's fixed-step boundary. The runtime
  // applies every edge immediately before its designated simulation frame, so
  // low refresh rates and main-thread stalls cannot collapse the jump timing.
  if (!scrub.scheduleKeyInputs(inputs)) {
    throw new Error("the platformer rejected the checked-in input script");
  }

  // The event markers publish separately from the fixed steps. Count paints
  // rather than wall time so a backgrounded tab simply resumes this wait.
  let markerPaints = 0;
  while (
    markerPaints < 600 &&
    !inputs.every((input) => scrub.events().some((event) => event.label === input.label))
  ) {
    markerPaints += 1;
    await nextPaint();
  }
  const events = scrub.events();
  if (!inputs.every((input) => events.some((event) => event.label === input.label))) {
    throw new Error("the platformer did not record every scripted input");
  }
  const firstInput = inputs[0];
  const firstEvent = events.find((event) => event.label === firstInput.label);
  const preservesFrameOffsets =
    firstEvent !== undefined &&
    inputs.every((input) => {
      const event = events.find((candidate) => candidate.label === input.label);
      return (
        event !== undefined &&
        event.frame - firstEvent.frame === input.frame - firstInput.frame
      );
    });
  if (!preservesFrameOffsets) {
    throw new Error("the platformer input markers drifted from the checked-in script");
  }

  if (!scrub.paused()) scrub.togglePause();
  const pauseDeadline = performance.now() + 3000;
  while (performance.now() < pauseDeadline && !scrub.paused()) await delay(16);
  if (!scrub.paused()) throw new Error("the platformer timeline did not pause");

  const jump = events.find((event) => event.label === "Up down");
  if (!jump) throw new Error("the platformer timeline has no jump marker");

  // Configure, but do not enable, extrapolation. The visible 🔮 button remains
  // the one click that reveals the complete recorded jump.
  scrub.setPreview({
    enabled: false,
    seconds: PREVIEW_SECONDS,
    rate: PREVIEW_RATE,
    mode: PREVIEW_MODE_BOTH,
  });
  const anchor = Math.max(0, jump.frame - 2);
  scrub.seek(anchor);
  const seekDeadline = performance.now() + 3000;
  while (performance.now() < seekDeadline && scrub.frame() !== anchor) await delay(16);
  if (scrub.frame() !== anchor) throw new Error("the platformer timeline did not reach the jump");
};

const maybeStageDemo = () => {
  if (staging || demoSettled || !playerReady || !editorSettled || !scriptedInputs) return;
  staging = true;
  setStatus("busy", "recording the jump…");
  void seedJumpHistory(scriptedInputs)
    .then(() => {
      demoSeeded = true;
      setStatus("live", "live — click 🔮 to preview the jump");
    })
    .catch((err: unknown) => {
      setStatus("error", err instanceof Error ? err.message : String(err));
    })
    .finally(() => {
      demoSettled = true;
      dismissBootLoader();
    });
};

const bridge = new PlayerBridge(frame, {
  onReloading: () => setStatus("busy"),
  onLive: () => {
    playerReady = true;
    if (!demoSettled) setStatus("busy", "recording the jump…");
    else if (demoSeeded) setStatus("live", "live — click 🔮 to preview the jump");
    maybeStageDemo();
    dismissBootLoader();
  },
  onResult: (ok, message) => {
    playerReady = true;
    dismissBootLoader();
    ok ? setStatus("live", message) : setStatus("error", message);
  },
});

let editor: EditorView | null = null;

const boot = async () => {
  // Establish the card's final successful-path geometry behind the loader.
  // A fetch/parse failure collapses this to a compact red error header below.
  editorShell.hidden = false;
  try {
    const [sourceResponse, scriptResponse] = await Promise.all([
      fetch(HERO_URL),
      fetch(JUMP_SCRIPT_URL),
    ]);
    if (!sourceResponse.ok) {
      throw new Error(`cannot fetch ${HERO_URL}: HTTP ${sourceResponse.status}`);
    }
    if (!scriptResponse.ok) {
      throw new Error(`cannot fetch ${JUMP_SCRIPT_URL}: HTTP ${scriptResponse.status}`);
    }
    const [source, jumpScript] = await Promise.all([
      sourceResponse.text(),
      scriptResponse.text(),
    ]);
    scriptedInputs = parseJumpScript(jumpScript);

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

    editor = createMiniEditor({
      parent: mount,
      doc: region,
      onChange: (src) => {
        region = src;
        bridge.push(fullProgram());
      },
    });
  } catch (err) {
    editorShell.classList.add("is-error");
    setStatus("error", err instanceof Error ? err.message : String(err));
    demoSettled = true;
    return;
  }
};

// Settled means "the card will not change shape again", which every exit path
// of boot() reaches — including failures, where only the compact error header
// remains. Finalizing here rather than at each `return` makes that exhaustive.
void boot().finally(() => {
  editorSettled = true;
  maybeStageDemo();
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
  staged: () => demoSeeded,
};
