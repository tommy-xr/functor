// Reproducible capture of the "Travel Through Time" hero GIF.
//
// Plays the `bounce` physics scene in the web player, pauses it, rewinds to the
// very start (time-travel — clear because the objects fly back up), and THEN
// switches on extrapolation so the predicted future arc of every object fans
// out. Everything is driven deterministically through the player's
// `window.__scrub` seam (see runtime/functor-runtime-web/scrubber.js), so the
// capture is repeatable — no pixel-hunting on the scrubber rail.
//
// Prereqs:
//   - the web runtime wasm bundle:  wasm-pack build runtime/functor-runtime-web --target=web
//   - @playwright/test's chromium (installed with the repo's playwright dep)
//   - ffmpeg on PATH (GIF assembly)
//
//   npm run demo:time-travel                      # -> site/demos/time-travel.gif
//   node site/demos/time-travel.mjs my/out.gif    # custom output path
//   DEMO_GAME=toss npm run demo:time-travel       # a different bundled scene
//   DEMO_SKIP_BUILD=1 npm run demo:time-travel    # reuse an existing site/dist
import { spawn, spawnSync, execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const ROOT = fileURLToPath(new URL("../..", import.meta.url));
const PORT = 8231; // dedicated port, so a dev `site:serve` on 8123 can keep running
const BASE = `http://127.0.0.1:${PORT}`;
const GAME = process.env.DEMO_GAME || "bounce";
const OUT = resolve(process.argv[2] || join(ROOT, "site/demos/time-travel.gif"));
const WIDTH = 900;
const HEIGHT = 560;
const GIF_WIDTH = 640;
const FPS = Number(process.env.DEMO_FPS || 18);

// Extrapolation preview settings (tweakable): window in seconds of predicted
// future, strobe samples per second, and mode (1 trail / 2 strobe / 3 both /
// 4 ghost). A short window + low rate reads as a clean "where it's headed next"
// prediction rather than a busy full-arc fan.
const WIN = Number(process.env.DEMO_WIN || 1);
const RATE = Number(process.env.DEMO_RATE || 2);
const MODE = Number(process.env.DEMO_MODE || 3);

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// 1. Build the static site (build.mjs errors with the wasm-bundle hint if the
//    pkg is missing). Skippable when dist is already fresh.
if (!process.env.DEMO_SKIP_BUILD) {
  const build = spawnSync("node", ["site/build.mjs"], { cwd: ROOT, stdio: "inherit" });
  if (build.status !== 0) process.exit(build.status ?? 1);
}

// 2. Serve dist on a dedicated port and wait for it to answer.
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

// 3. Drive the player through the scrub seam.
let browser;
try {
  browser = await chromium.launch();
} catch {
  browser = await chromium.launch({ channel: "chrome" }); // fall back to system Chrome
}
const page = await browser.newPage({ viewport: { width: WIDTH, height: HEIGHT } });
await page.goto(`${BASE}/player.html?game=examples%2F${GAME}.fun`, { waitUntil: "load" });
// hide the "mouse look" chip for a cleaner frame
await page.addStyleTag({ content: '[title*="mouse"]{display:none!important}' }).catch(() => {});

// wait for the seam + a recorded range, then let history accumulate
await page.waitForFunction(() => window.__scrub && window.__scrub.range().length === 2, {
  timeout: 20000,
});
await sleep(7000);

const framesDir = mkdtempSync(join(tmpdir(), "functor-tt-"));
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

// pause with extrapolation OFF, so the rewind reads as pure time-travel
await page.evaluate(() => {
  if (!window.__scrub.paused()) window.__scrub.togglePause();
  window.__scrub.setPreview({ enabled: false });
});
await sleep(150);
const [lo, hi] = await page.evaluate(() => window.__scrub.range());

// PHASE 1 — rewind present -> start (extrapolate OFF)
await hold(4);
const REWIND_STEPS = 28;
for (let i = 1; i <= REWIND_STEPS; i++) {
  const frame = Math.round(hi - (hi - lo) * (i / REWIND_STEPS));
  await page.evaluate((f) => window.__scrub.seek(f), frame);
  await sleep(45);
  await snap();
}
await hold(5);

// PHASE 2 — at the start, switch extrapolation ON (wide window, dense strobe)
// so the whole predicted future arc reveals, then creep the anchor forward so
// the prediction visibly tracks.
await page.evaluate(
  ({ win, rate, mode }) => {
    // mode (trail/strobe/both/ghost) is driven by the select; the window and
    // strobe rate go through the scrub seam so they apply authoritatively
    // (setting #scrub-win by hand raced with the seam and could land on the
    // default window instead).
    const sel = document.querySelector("#scrub-mode");
    if (sel) {
      sel.value = String(mode);
      sel.dispatchEvent(new Event("change", { bubbles: true }));
    }
    window.__scrub.setPreview({ enabled: true, seconds: win, rate });
  },
  { win: WIN, rate: RATE, mode: MODE }
);
await sleep(250);
await hold(7, 55); // let the reveal land
const CREEP_STEPS = 18;
for (let i = 1; i <= CREEP_STEPS; i++) {
  const frame = Math.round(lo + (hi - lo) * 0.5 * (i / CREEP_STEPS));
  await page.evaluate((f) => window.__scrub.seek(f), frame);
  await sleep(50);
  await snap();
}
await hold(6, 45);

await browser.close();
server.kill();

// 4. Assemble the GIF (palettegen for clean color, lanczos downscale).
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
