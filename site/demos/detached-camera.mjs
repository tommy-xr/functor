// Reproducible capture of the shell-owned 3D debug camera.
//
// Drives the bundled `orbit` scene through the player's `window.__scrub` seam:
// play -> FPS debug view -> pause and keep exploring -> resume in the same view
// -> reattach. The output is a deterministic interaction demo rather than an
// OS screen recording.
//
// Prereqs:
//   - the web runtime wasm bundle: wasm-pack build runtime/functor-runtime-web --target=web
//   - @playwright/test's chromium (or system Chrome)
//   - ffmpeg on PATH
//
//   npm run demo:detached-camera
//   node site/demos/detached-camera.mjs /tmp/demo.gif /tmp/still.png
//   DEMO_SKIP_BUILD=1 npm run demo:detached-camera
import { execFileSync, spawn, spawnSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const ROOT = fileURLToPath(new URL("../..", import.meta.url));
const PORT = 8232;
const BASE = `http://127.0.0.1:${PORT}`;
const OUT = resolve(process.argv[2] || join(ROOT, "site/demos/detached-camera.gif"));
const STILL = resolve(process.argv[3] || join(ROOT, "site/demos/detached-camera.png"));
const WIDTH = 960;
const HEIGHT = 640;
const GIF_WIDTH = 640;
const FPS = 16;
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

if (!process.env.DEMO_SKIP_BUILD) {
  const build = spawnSync("node", ["site/build.mjs"], { cwd: ROOT, stdio: "inherit" });
  if (build.status !== 0) process.exit(build.status ?? 1);
}

const server = spawn("node", ["site/serve.mjs", "--port", String(PORT)], {
  cwd: ROOT,
  stdio: "ignore",
});
process.on("exit", () => server.kill());
for (let attempt = 0; ; attempt += 1) {
  try {
    if ((await fetch(BASE)).ok) break;
  } catch {
    // Keep polling while the static server starts.
  }
  if (attempt > 50) throw new Error(`site server never came up on ${PORT}`);
  await sleep(100);
}

let browser;
try {
  browser = await chromium.launch();
} catch {
  browser = await chromium.launch({ channel: "chrome" });
}

const page = await browser.newPage({ viewport: { width: WIDTH, height: HEIGHT } });
await page.goto(`${BASE}/player.html?game=examples%2Forbit.fun`, {
  waitUntil: "load",
});
await page.waitForFunction(() => window.__scrub?.range().length === 2, { timeout: 20000 });
await sleep(900);

const framesDir = mkdtempSync(join(tmpdir(), "functor-detached-camera-"));
let frame = 0;
const snap = async () => {
  await page.screenshot({
    path: join(framesDir, `f${String(frame).padStart(4, "0")}.png`),
  });
  frame += 1;
};
const hold = async (count, ms = 35) => {
  for (let index = 0; index < count; index += 1) {
    await snap();
    await sleep(ms);
  }
};

// Establish the live authored view, then activate the universal debug camera
// without stopping the game.
await hold(6);
await page.locator("#scrub-camera").click();
await page.waitForFunction(() => window.__scrub.detached());
await hold(6);
for (let index = 0; index < 20; index += 1) {
  await page.evaluate((step) => {
    window.__scrub.lookDetached(2.5, -0.15);
    if (step < 4) window.__scrub.moveDetached(0.15, 0.25, 0.05, 0.035);
  }, index);
  await sleep(35);
  await snap();
}

// Keep a representative live-debug frame while the scene remains in view.
mkdirSync(dirname(STILL), { recursive: true });
copyFileSync(join(framesDir, `f${String(frame - 1).padStart(4, "0")}.png`), STILL);

// Pin the game without giving up the debug view, then keep flying.
await page.evaluate(() => window.__scrub.togglePause());
await page.waitForFunction(() => window.__scrub.paused() && window.__scrub.detached());
await hold(6);
for (let index = 0; index < 16; index += 1) {
  await page.evaluate((step) => {
    window.__scrub.lookDetached(-1.5, 0.2);
    if (step < 3) window.__scrub.moveDetached(-0.05, -0.2, 0.1, 0.035);
  }, index);
  await sleep(35);
  await snap();
}
await hold(7);

// Resume without reattaching, then explicitly return to the authored camera.
await page.evaluate(() => window.__scrub.togglePause());
await page.waitForFunction(() => !window.__scrub.paused() && window.__scrub.detached());
await hold(7);
await page.evaluate(() => document.exitPointerLock());
await page.locator("#scrub-camera").click();
await page.waitForFunction(() => !window.__scrub.detached());
await hold(7);

await browser.close();
server.kill();

mkdirSync(dirname(OUT), { recursive: true });
execFileSync(
  "ffmpeg",
  [
    "-y",
    "-framerate",
    String(FPS),
    "-i",
    join(framesDir, "f%04d.png"),
    "-vf",
    `scale=${GIF_WIDTH}:-1:flags=lanczos,split[s0][s1];[s0]palettegen=stats_mode=diff[p];[s1][p]paletteuse=dither=bayer:bayer_scale=3`,
    "-loop",
    "0",
    OUT,
  ],
  { stdio: "inherit" }
);
rmSync(framesDir, { recursive: true, force: true });
console.log(`wrote ${OUT} (${frame} frames)`);
console.log(`wrote ${STILL}`);
