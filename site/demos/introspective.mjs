// Reproducible capture of the "Introspective" hero GIF.
//
// Drives the standalone demo editor (demo-editor.html) over the orbit scene: run
// it, pause, and scrub through time while the paused-inspector live values —
// every orb's angle, height, and hue, plus the model clock — course through the
// code inline. No editing: the point is that every value in a running game is
// inspectable, at any moment. Driven headless via window.__demoEditor and the
// player's window.__scrub seam, with the demo overlay for captions.
//
// Prereqs (both wasm bundles — the live-value overlay needs the analysis one):
//   - web runtime:  wasm-pack build runtime/functor-runtime-web --target=web
//   - lang analysis: npm run build:lang-wasm
//   - @playwright/test's chromium, and ffmpeg on PATH
//
//   npm run demo:introspective                 # -> site/demos/introspective.gif
//   node site/demos/introspective.mjs out.gif  # custom output path
import { spawn, spawnSync, execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, rmSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";
import { installOverlay } from "./lib/overlay.mjs";

const ROOT = fileURLToPath(new URL("../..", import.meta.url));
const PORT = 8233; // dedicated port (time-travel 8231, instant-feedback 8232)
const BASE = `http://127.0.0.1:${PORT}`;
const GAME = process.env.DEMO_GAME || "examples/orbit.fun";
const OUT = resolve(process.argv[2] || join(ROOT, "site/demos/introspective.gif"));
const WIDTH = 1120;
const HEIGHT = 640;
const GIF_WIDTH = 820;
const FPS = Number(process.env.DEMO_FPS || 14);

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

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

const playerFrame = () => page.frames().find((f) => f.url().includes("player.html"));
await sleep(1500);
await playerFrame()
  ?.addStyleTag({ content: '[title*="mouse"]{display:none!important}' })
  .catch(() => {});
await sleep(6000); // accumulate a timeline

// Scroll the editor to the value-rich orb function.
await page.evaluate(() => {
  const v = window.__demoEditor.view;
  const i = v.state.doc.toString().indexOf("let orb");
  v.dispatch({ selection: { anchor: i }, scrollIntoView: true });
});
await sleep(400);

const framesDir = mkdtempSync(join(tmpdir(), "functor-in-"));
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

// 0. Intro — a running game is just data.
await ov.caption("A running game is just <b>data</b> — every value inspectable.");
await hold(13, 60);

// 1. Pause (ripple the button).
const pauseBox = await playerFrame()
  .locator("#scrub-pause")
  .boundingBox()
  .catch(() => null);
if (pauseBox) await ov.click(pauseBox.x + pauseBox.width / 2, pauseBox.y + pauseBox.height / 2);
await hold(2, 60);
await playerFrame().evaluate(() => {
  if (window.__scrub && !window.__scrub.paused()) window.__scrub.togglePause();
});
await ov.caption("Pause — the <b>live values</b> appear inline in the code.");
await sleep(1200);
await hold(22, 65); // values visible: angle, height, hue, the clock

// 2. Scrub through time — the values course through as the orbs move.
await ov.caption("Scrub to <b>any moment</b> — watch the values course through.");
const [lo, hi] = await playerFrame().evaluate(() => window.__scrub.range());
const seq = [];
for (let f = hi; f >= lo + (hi - lo) * 0.05; f -= (hi - lo) / 18) seq.push(Math.round(f));
for (let f = lo + (hi - lo) * 0.05; f <= hi; f += (hi - lo) / 18) seq.push(Math.round(f));
for (const frame of seq) {
  await playerFrame().evaluate((v) => window.__scrub.seek(v), frame);
  await sleep(90);
  await snap();
}
await hold(14, 60);

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
