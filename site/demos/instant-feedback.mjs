// Reproducible capture of the "Instant Feedback" hero GIF.
//
// Drives the standalone demo editor (demo-editor.html) over the synthwave hero
// scene: type a live edit to the emissive color → the grid hot-swaps and
// recolors while the wave keeps rolling (no reload) → pause the scene → the
// paused-inspector live values flow inline into the code. Everything runs
// headless through the editor's window.__demoEditor seam and the player's
// window.__scrub seam.
//
// Prereqs (both wasm bundles — the live-value overlay needs the analysis one):
//   - web runtime:  wasm-pack build runtime/functor-runtime-web --target=web
//   - lang analysis: npm run build:lang-wasm
//   - @playwright/test's chromium, and ffmpeg on PATH
//
//   npm run demo:instant-feedback                 # -> site/demos/instant-feedback.gif
//   node site/demos/instant-feedback.mjs out.gif  # custom output path
import { spawn, spawnSync, execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, rmSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";
import { installOverlay } from "./lib/overlay.mjs";

const ROOT = fileURLToPath(new URL("../..", import.meta.url));
const PORT = 8232; // dedicated port (time-travel uses 8231)
const BASE = `http://127.0.0.1:${PORT}`;
const GAME = process.env.DEMO_GAME || "examples/orbit.fun";
const OUT = resolve(process.argv[2] || join(ROOT, "site/demos/instant-feedback.gif"));
const WIDTH = 1120;
const HEIGHT = 640;
const GIF_WIDTH = 820;
const FPS = Number(process.env.DEMO_FPS || 14);

// The live edit: select `select` (a number, found by locating the unique `find`
// string it begins) and type `type` over it. Scene-specific — tied to
// examples/orbit.fun's orb scale, so the orbs visibly grow. The demo first
// shrinks the scale silently (SHRINK) so the orbs start small on screen and the
// recorded edit grows them dramatically — without touching orbit.fun itself.
const SHRINK_FROM = "0.55)";
const SHRINK_TO = "0.3";
const EDIT_FIND = "0.3)";
const EDIT_SELECT = "0.3";
const EDIT_TYPE = "1.0";

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// The analysis wasm is REQUIRED here (unlike the general site build, where it's
// optional): without it the editor has no live-value overlay and the demo shows
// nothing on pause. Fail loud rather than capture a broken GIF.
if (!existsSync(join(ROOT, "tools/functor-lang-wasm/pkg/functor_lang_wasm.js"))) {
  console.error(
    "missing the language-analysis wasm (the live-value overlay needs it):\n" +
      "  npm run build:lang-wasm"
  );
  process.exit(1);
}

if (!process.env.DEMO_SKIP_BUILD) {
  const build = spawnSync("node", ["site/build.mjs"], { cwd: ROOT, stdio: "inherit" });
  if (build.status !== 0) process.exit(build.status ?? 1);
}

const server = spawn("node", ["site/serve.mjs", "--port", String(PORT)], {
  cwd: ROOT,
  stdio: "ignore",
});
process.on("exit", () => server.kill());
for (let i = 0; ; i++) {
  try {
    if ((await fetch(BASE)).ok) break;
  } catch {
    /* not up yet */
  }
  if (i > 50) throw new Error(`site server never came up on ${PORT}`);
  await sleep(100);
}

let browser;
try {
  browser = await chromium.launch();
} catch {
  browser = await chromium.launch({ channel: "chrome" });
}
const page = await browser.newPage({ viewport: { width: WIDTH, height: HEIGHT } });
await page.goto(`${BASE}/demo-editor.html?game=${encodeURIComponent(GAME)}`, {
  waitUntil: "load",
});
await page.evaluate(() => window.__demoEditor.ready);
const ov = await installOverlay(page);

// The scene renders in a player iframe; the "mouse look" chip lives inside it,
// so the hide style has to be injected into the frame, not the parent page.
const playerFrame = () => page.frames().find((f) => f.url().includes("player.html"));
await sleep(1500);
await playerFrame()
  ?.addStyleTag({ content: '[title*="mouse"]{display:none!important}' })
  .catch(() => {});
await sleep(3500); // let the scene go live and roll a little

// Silently shrink the orbs first so they start small on screen — this push
// hot-reloads before recording, so the viewer only sees the deliberate growth.
await page.evaluate(
  ({ from, to }) => {
    const v = window.__demoEditor.view;
    const i = v.state.doc.toString().indexOf(from);
    if (i >= 0) v.dispatch({ changes: { from: i, to: i + from.length - 1, insert: to } });
  },
  { from: SHRINK_FROM, to: SHRINK_TO }
);
await sleep(900); // reload settles — orbs now small

const framesDir = mkdtempSync(join(tmpdir(), "functor-if-"));
let n = 0;
const snap = async () => {
  await page.screenshot({ path: join(framesDir, `f${String(n).padStart(4, "0")}.png`) });
  n++;
};
const hold = async (frames, ms = 25) => {
  for (let k = 0; k < frames; k++) {
    await snap();
    await sleep(ms);
  }
};

// 0. Intro: the scene is running.
await ov.caption("A live <b>Functor Lang</b> scene, running.");
await hold(12, 60);

// 1. Select the target number and name what we're about to change.
await page.evaluate(
  ({ find, select }) => {
    const v = window.__demoEditor.view;
    const i = v.state.doc.toString().indexOf(find);
    v.focus();
    v.dispatch({ selection: { anchor: i, head: i + select.length }, scrollIntoView: true });
  },
  { find: EDIT_FIND, select: EDIT_SELECT }
);
await ov.caption("Change a value — the orb <b>size</b>.");
await sleep(500);
await hold(10, 60);

// 2. Type the edit character by character, flashing each keystroke. Under the
//    300ms push debounce (keystrokes are closer than that) it coalesces to one
//    clean hot-swap, so the orbs grow while the ring keeps turning.
for (const ch of EDIT_TYPE) {
  await page.keyboard.type(ch);
  await ov.key(ch);
  await sleep(170);
  await snap();
}
await sleep(550); // debounce + hot reload settle
await ov.caption("The orbs grow <b>instantly</b> — no reload, still orbiting.");
await hold(30, 75); // orbs now large, still orbiting — hold on the result

await browser.close();
server.kill();

mkdirSync(dirname(OUT), { recursive: true });
execFileSync(
  "ffmpeg",
  [
    "-y",
    "-framerate", String(FPS),
    "-i", join(framesDir, "f%04d.png"),
    "-vf",
    `scale=${GIF_WIDTH}:-1:flags=lanczos,split[s0][s1];[s0]palettegen=stats_mode=diff[p];[s1][p]paletteuse=dither=bayer:bayer_scale=3`,
    "-loop", "0",
    OUT,
  ],
  { stdio: "inherit" }
);
rmSync(framesDir, { recursive: true, force: true });
console.log(`\nwrote ${OUT} (${n} frames)`);
