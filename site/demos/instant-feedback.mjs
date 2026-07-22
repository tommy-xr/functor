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

const ROOT = fileURLToPath(new URL("../..", import.meta.url));
const PORT = 8232; // dedicated port (time-travel uses 8231)
const BASE = `http://127.0.0.1:${PORT}`;
const GAME = process.env.DEMO_GAME || "examples/hero.fun";
const OUT = resolve(process.argv[2] || join(ROOT, "site/demos/instant-feedback.gif"));
const WIDTH = 1120;
const HEIGHT = 640;
const GIF_WIDTH = 820;
const FPS = Number(process.env.DEMO_FPS || 18);

// The live edit: replace `find` (a number in the served scene) with `insert`.
// Scene-specific — tied to examples/hero.fun's emissive green channel.
const EDIT_FIND = "0.15 + 0.1";
const EDIT_SELECT = "0.15";
const EDIT_TYPE = "0.95";

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

// The scene renders in a player iframe; the "mouse look" chip lives inside it,
// so the hide style has to be injected into the frame, not the parent page.
const playerFrame = () => page.frames().find((f) => f.url().includes("player.html"));
await sleep(1500);
await playerFrame()
  ?.addStyleTag({ content: '[title*="mouse"]{display:none!important}' })
  .catch(() => {});
await sleep(3500); // let the scene go live and roll a little

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

// 1. Select the target number and hold a beat (the scene is running, magenta).
await page.evaluate(
  ({ find, select }) => {
    const v = window.__demoEditor.view;
    const i = v.state.doc.toString().indexOf(find);
    v.focus();
    v.dispatch({ selection: { anchor: i, head: i + select.length }, scrollIntoView: true });
  },
  { find: EDIT_FIND, select: EDIT_SELECT }
);
await sleep(500);
await hold(8, 60);

// 2. Type the edit character by character (the new value appears). Under the
//    300ms push debounce it coalesces to one clean hot-swap, and the grid
//    recolors while the wave keeps rolling.
for (const ch of EDIT_TYPE) {
  await page.keyboard.type(ch);
  await sleep(110);
  await snap();
}
await sleep(550); // debounce + hot reload settle
await hold(16, 70); // grid now green, still rolling

// 3. Pause the scene — the paused-inspector live values flow inline.
await playerFrame().evaluate(() => {
  if (window.__scrub && !window.__scrub.paused()) window.__scrub.togglePause();
});
await sleep(1400);
await hold(30, 55); // inline live values populated — hold to read them

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
