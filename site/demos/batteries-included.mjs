// Reproducible capture of the "Batteries Included" hero GIF.
//
// Drives the demo editor over the batteries scene: a rigged character streamed
// straight from a CDN, skinned and animated (idle → walk → run, blended) with no
// asset pipeline to set up. The scene animates itself; this just holds while the
// overlay captions name the batteries. Driven headless via window.__demoEditor.
//
// Prereqs: the web runtime wasm bundle (wasm-pack build
// runtime/functor-runtime-web --target=web), @playwright/test's chromium, and
// ffmpeg on PATH. Needs network access (the model streams from jsDelivr).
//
//   npm run demo:batteries-included                 # -> site/demos/batteries-included.gif
//   node site/demos/batteries-included.mjs out.gif  # custom output path
import { spawn, spawnSync, execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";
import { installOverlay } from "./lib/overlay.mjs";

const ROOT = fileURLToPath(new URL("../..", import.meta.url));
const PORT = 8234; // dedicated port
const BASE = `http://127.0.0.1:${PORT}`;
const GAME = process.env.DEMO_GAME || "examples/batteries.fun";
const OUT = resolve(process.argv[2] || join(ROOT, "site/demos/batteries-included.gif"));
const WIDTH = 1120;
const HEIGHT = 640;
const GIF_WIDTH = 820;
const FPS = Number(process.env.DEMO_FPS || 14);

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

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
await sleep(8000); // stream the model from the CDN and let it start animating

// Show the code that does it — the CDN URL and the clip blend.
await page.evaluate(() => {
  const v = window.__demoEditor.view;
  const i = v.state.doc.toString().indexOf("let hero");
  v.dispatch({ selection: { anchor: i }, scrollIntoView: true });
});
await sleep(400);

const framesDir = mkdtempSync(join(tmpdir(), "functor-bi-"));
let n = 0;
const snap = async () => {
  await page.screenshot({ path: join(framesDir, `f${String(n).padStart(4, "0")}.png`) });
  n++;
};
const hold = async (frames, ms = 70) => {
  for (let k = 0; k < frames; k++) {
    await snap();
    await sleep(ms);
  }
};

// The character auto-cycles idle → walk → run the whole time; the captions just
// name what's happening. (hold() lets real time pass, so it keeps animating.)
await ov.caption("A rigged character, streamed straight from a <b>URL</b>.");
await hold(20, 70);
await ov.caption("Skinned animation — <b>idle → walk → run</b>, blended.");
await hold(20, 70);
await ov.caption("Assets, animation, physics, audio — <b>batteries included</b>.");
await hold(24, 70);

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
