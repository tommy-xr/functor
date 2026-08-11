// Reproducible capture of the landing page's GAMES CAROUSEL box art: one
// looping GIF + one poster PNG per shortlisted game, into site/media/box-art/.
//
//   npm run demo:box-art                       # every card in site/src/examples.ts
//   node site/demos/box-art.mjs tetris orbs    # just these
//   BOXART_OUT=/tmp/shots node site/demos/box-art.mjs tetris   # elsewhere (scratch runs)
//   DEMO_SKIP_BUILD=1 …                        # reuse site/dist (web backend only)
//   DEMO_PORT / DEMO_FPS                       # override the serve port / gif framerate
//
// The card set and its order come from `GALLERY` in site/src/examples.ts — the
// same list build.mjs renders — so this script cannot capture a game the page
// does not show, or miss one it does. What lives HERE is only the per-game
// capture RECIPE (below): which backend shoots it, and which frames.
//
// Two backends:
//
//   native — the desktop runtime's scripted-input path. Determinism comes from
//     `--input-script` + `--script-dt` (NOT `--fixed-time`, which conflicts with
//     it and would freeze a model-driven game at `init`), and `--capture-at-frame N`
//     shoots one exact sim frame. That is ONE frame per process, so a GIF is N
//     invocations at frames from, from+step, … — each replaying the same script
//     from frame 0, so every frame of the GIF is byte-reproducible. The drive
//     itself is a committed `<game>/*.script`, so the gameplay in the box art is
//     reviewable next to the game.
//
//   web — Playwright against the built site's sandbox, for the multiplayer
//     cards. netpong and orbs both need an authoritative SERVER, which the
//     sandbox's pane grid + in-page net coordinator already stand up; the shot
//     is one CLIENT pane's surface (`.mp-pane-body`), so the card shows the game
//     rather than the sandbox chrome. netpong plays itself (its client boots
//     `autopilot: true` — the attract mode), orbs is flown from the keyboard.
//
// Prereqs: `npm run build:cli` (RELEASE — a debug build's physics/raster is far
// too slow to shoot 300 frames), ffmpeg on PATH, and for the web backend the web
// runtime wasm bundle plus @playwright/test's chromium. Also `npm run
// fetch:assets` once: examples/asteroids gitignores its ship model, and a
// missing asset renders as the empty fallback — a shipless Asteroids card.
import { spawn, spawnSync, execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, rmSync, statSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { GALLERY } from "../src/examples.ts";

const ROOT = fileURLToPath(new URL("../..", import.meta.url));
const OUT_DIR = resolve(process.env.BOXART_OUT || join(ROOT, "site/media/box-art"));
const FUNCTOR = join(ROOT, "target/release/functor");
const PORT = Number(process.env.DEMO_PORT || 8236); // dedicated port (… multiplayer 8235)
const BASE = `http://127.0.0.1:${PORT}`;
const FPS = Number(process.env.DEMO_FPS || 12);

// The card is 16:10 — every game here is authored for a landscape window, so a
// portrait "box" would crop breakout's play field and letterbox the rest. The
// GIF is quarter-size of the capture (and of the poster), which is what keeps it
// inside the ~250 KB budget; the poster stays crisp for HiDPI cards.
const CAPTURE = { width: 640, height: 400 };
// 96 colors is the budget knob that costs the least: these are flat neon scenes,
// so a shorter palette is invisible on the card and roughly halves the bytes a
// full 256-color one spends on dither noise.
// A card renders the GIF at 320 CSS px, so that is its default width. The LIT 3D
// scenes (shooting-range's muzzle flash, synthwave's scrolling grid, code-garden's
// moving daylight) change every pixel of every frame, which defeats GIF's
// inter-frame coding entirely — they override the encode down a notch to stay
// inside the budget, paying a little softness while animating (their POSTER is
// still full-size).
const GIF = { width: 320, height: 200, colors: 96 };
const SCRIPT_DT = 1 / 60;
const BUDGET_BYTES = 250 * 1024;

/**
 * Per-game capture recipes. Native entries name a committed input script in the
 * game's own directory (`--input-script` resolves against `-d`, so it is bare)
 * and a frame window `from`, `from + step`, … `frames` shots long — chosen so the
 * loop lands mid-gameplay, never on a dead board or a title card. Web entries
 * name how many client panes to seat and how the captured pane is driven.
 */
const RECIPES = {
  // 30 frames × 6 = 3s of scripted play from frame 170, by which point the script
  // has stacked and cleared rows: pieces are landing, not just falling. It ends
  // before frame ~440, where the blind drive finally tops the board out.
  tetris: { backend: "native", dir: "examples/tetris", script: "boxart.script", from: 170, step: 6, frames: 30 },
  // Turn-based: one keypress every 10 frames, so a 5-frame step reads as a
  // continuous crawl through the fog.
  roguelike: { backend: "native", dir: "examples/roguelike", script: "boxart.script", from: 60, step: 5, frames: 30 },
  // Well past the launch (frame 30): the ball is among the bricks and the score
  // is climbing, with the sweeping paddle keeping the rally going.
  breakout: { backend: "native", dir: "examples/breakout", script: "boxart.script", from: 160, step: 5, frames: 30 },
  // Past the menu (Enter at frame 12) and into wave 1: rotate, thrust, fire.
  asteroids: { backend: "native", dir: "examples/asteroids", script: "boxart.script", from: 150, step: 4, frames: 30 },
  // The committed verification drive: run off the left platform, clear the
  // chasm, land on the right. Exactly the jump the landing hero parks on.
  platformer: { backend: "native", dir: "examples/platformer", script: "jump.script", from: 6, step: 3, frames: 30 },
  // Held full-auto fire down a fixed lane: targets drop, the gun climbs.
  "shooting-range": {
    backend: "native", dir: "examples/shooting-range", script: "boxart.script",
    from: 60, step: 6, frames: 20, gif: { width: 256, height: 160, colors: 64 },
  },
  // The garden's own no-input script (its state accrues in the model, so time
  // is the only drive it needs). Dawn onwards, when the plants are grown.
  "code-garden": {
    backend: "native", dir: "examples/code-garden", script: "idle.script",
    from: 1400, step: 15, frames: 20, gif: { width: 256, height: 160, colors: 64 },
  },
  // No model at all — a pure function of time, so the script is empty too.
  synthwave: {
    backend: "native", dir: "examples/synthwave", script: "boxart.script",
    from: 60, step: 5, frames: 24, gif: { width: 256, height: 160, colors: 64 },
  },
  // Both roles play themselves once a server is seated: the client's `autopilot`
  // starts true, so one client pane is a full AI rally.
  netpong: { backend: "web", clients: 1, drive: "idle", frames: 40, everyMs: 60 },
  // Two rivals, shot from client 1: the flown ship AND the other pilot's.
  orbs: { backend: "web", clients: 2, drive: "fly", frames: 40, everyMs: 70, warmup: 40 },
};

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const pad = (n) => String(n).padStart(4, "0");

const ids = process.argv.slice(2);
const unknown = ids.filter((id) => !RECIPES[id]);
if (unknown.length) {
  console.error(`unknown game id(s): ${unknown.join(", ")}\nknown: ${Object.keys(RECIPES).join(", ")}`);
  process.exit(1);
}
// The gallery is the source of truth for WHICH cards exist; a recipe without a
// gallery entry (or the reverse) is a drift bug, not a silently skipped card.
const missing = GALLERY.filter((example) => !RECIPES[example.id]).map((example) => example.id);
if (missing.length) {
  console.error(`gallery entries with no capture recipe: ${missing.join(", ")}`);
  process.exit(1);
}
const orphans = Object.keys(RECIPES).filter((id) => !GALLERY.some((example) => example.id === id));
if (orphans.length) {
  console.error(`capture recipes with no gallery entry: ${orphans.join(", ")}`);
  process.exit(1);
}
const targets = (ids.length ? GALLERY.filter((example) => ids.includes(example.id)) : GALLERY).map(
  (example) => ({ id: example.id, ...RECIPES[example.id] })
);

/**
 * GIF + poster from a directory of `f0000.png…` frames. `increase`+`crop`
 * normalizes whatever the backend handed us to the card's exact 16:10 box (a
 * no-op for the native captures, a hair off the sandbox pane's own aspect), and
 * the two-pass palette is the same clean-color recipe as the feature demos.
 */
function assemble(spec, framesDir, count) {
  const id = spec.id;
  const out = { ...GIF, ...spec.gif };
  const box = (w, h) =>
    `scale=${w}:${h}:force_original_aspect_ratio=increase:flags=lanczos,crop=${w}:${h}`;
  const seq = join(framesDir, "f%04d.png");
  const palette = join(framesDir, "palette.png");
  const gif = join(OUT_DIR, `${id}.gif`);
  const poster = join(OUT_DIR, `${id}.png`);
  mkdirSync(OUT_DIR, { recursive: true });
  const ff = (args) => execFileSync("ffmpeg", ["-y", "-v", "error", ...args], { stdio: "inherit" });
  ff([
    "-i", seq,
    "-vf", `${box(out.width, out.height)},palettegen=stats_mode=diff:max_colors=${out.colors}`,
    palette,
  ]);
  ff([
    "-framerate", String(FPS),
    "-i", seq,
    "-i", palette,
    "-lavfi", `${box(out.width, out.height)} [x]; [x][1:v] paletteuse=dither=bayer:bayer_scale=3:diff_mode=rectangle`,
    "-loop", "0",
    gif,
  ]);
  // The poster is the frame the card shows before it animates, so take one from
  // the MIDDLE of the loop: frame 0 of a scripted run is often the calm before
  // anything has happened.
  ff([
    "-i", join(framesDir, `f${pad(Math.floor(count / 2))}.png`),
    "-vf", box(CAPTURE.width, CAPTURE.height),
    poster,
  ]);
  const kb = (path) => `${(statSync(path).size / 1024).toFixed(0)} KB`;
  const over = statSync(gif).size > BUDGET_BYTES ? "  ← OVER BUDGET" : "";
  console.log(`[box-art] ${id}: ${count} frames → ${gif} (${kb(gif)})${over}, ${poster} (${kb(poster)})`);
}

/** One process per frame: `--capture-at-frame` shoots a single sim frame. */
function captureNative(spec, framesDir) {
  const dir = join(ROOT, spec.dir);
  for (let i = 0; i < spec.frames; i++) {
    const frame = spec.from + i * spec.step;
    const result = spawnSync(
      FUNCTOR,
      [
        "-d", dir,
        "run", "native",
        "--input-script", spec.script,
        "--script-dt", String(SCRIPT_DT),
        "--capture-size", `${CAPTURE.width}x${CAPTURE.height}`,
        "--capture-frame", join(framesDir, `f${pad(i)}.png`),
        "--capture-at-frame", String(frame),
      ],
      { cwd: ROOT, stdio: ["ignore", "ignore", "pipe"] }
    );
    if (result.status !== 0 || !existsSync(join(framesDir, `f${pad(i)}.png`))) {
      throw new Error(
        `capture of ${spec.id} at frame ${frame} failed:\n${(result.stderr ?? "").toString().trim()}`
      );
    }
    process.stdout.write(`\r[box-art] ${spec.id}: frame ${i + 1}/${spec.frames}`);
  }
  process.stdout.write("\n");
}

/** Serve site/dist once for every web-backed card in this run. */
async function withSite(run) {
  if (!process.env.DEMO_SKIP_BUILD) {
    const build = spawnSync("node", ["site/build.mjs"], { cwd: ROOT, stdio: "inherit" });
    if (build.status !== 0) process.exit(build.status ?? 1);
  }
  const server = spawn("node", ["site/serve.mjs", "--port", String(PORT)], {
    cwd: ROOT,
    stdio: "ignore",
  });
  const kill = () => server.kill();
  process.on("exit", kill);
  let exited = null;
  server.on("exit", (code) => { exited = code ?? -1; });
  for (let i = 0; ; i++) {
    if (exited !== null) throw new Error(`site server exited with ${exited} — is :${PORT} in use?`);
    try {
      if ((await fetch(BASE)).ok) break;
    } catch {
      /* not up yet */
    }
    if (i > 100) throw new Error(`site server never came up on ${PORT}`);
    await sleep(100);
  }
  const { chromium } = await import("@playwright/test");
  let browser;
  try {
    browser = await chromium.launch();
  } catch {
    browser = await chromium.launch({ channel: "chrome" }); // fall back to system Chrome
  }
  try {
    await run(browser);
  } finally {
    await browser.close();
    kill();
  }
}

async function captureWeb(spec, framesDir, browser) {
  // Wide enough that the sandbox lays out its pane grid at full size; only the
  // captured pane's body is ever in shot.
  const page = await browser.newPage({ viewport: { width: 1760, height: 900 } });
  try {
    await page.goto(`${BASE}/sandbox.html?example=${spec.id}#clients=${spec.clients}`, {
      waitUntil: "load",
    });
    await page.waitForFunction(() => window.__sandbox?.status().state === "live", null, {
      timeout: 120000,
    });
    // Every pane linked through the coordinator: shooting before the client is
    // seated captures an empty field waiting for its first snapshot.
    const panes = spec.clients + 1; // + the authoritative server pane
    await page.waitForFunction(
      (expected) =>
        document.querySelectorAll(".mp-pane").length === expected &&
        [...document.querySelectorAll(".mp-pane .mp-conn")].every(
          (conn) => conn.dataset.linked === "true"
        ),
      panes,
      { timeout: 120000 }
    );
    const surface = page.locator(".mp-pane").first().locator(".mp-pane-body");
    if (spec.drive === "fly") {
      // Hand the pane the real keyboard: mousedown's default action is what
      // moves focus into the iframe, and a synthetic dispatch has none.
      await page.locator(".mp-pane").first().locator(".mp-pane-hd").click();
      await sleep(300);
      // Space is HELD for the whole flight: claiming is a level, not an edge, so
      // holding it turns every orb the ship crosses into this pilot's colour —
      // which is the game the card has to show.
      await page.keyboard.down("Space");
    }
    await sleep(1200); // let the rally / the arena settle into motion
    /**
     * One beat of the drive. For `fly`, a repeating turn-and-thrust: the ship
     * visibly flies (and its intent stream carries changing values) across the
     * whole loop instead of drifting.
     */
    const beat = async (i) => {
      if (spec.drive === "fly" && i % 10 === 0) {
        const turn = i % 20 < 10 ? "KeyA" : "KeyD";
        await page.keyboard.up("KeyA").catch(() => {});
        await page.keyboard.up("KeyD").catch(() => {});
        await page.keyboard.down(turn);
        await page.keyboard.down("KeyW");
      }
    };
    // Beats before the camera rolls, so the loop opens on a game already in
    // progress — orbs needs a lap of flying for the first claims to land.
    const warmup = spec.warmup ?? 0;
    for (let i = 0; i < warmup; i++) {
      await beat(i);
      await sleep(spec.everyMs);
    }
    for (let i = 0; i < spec.frames; i++) {
      await beat(warmup + i);
      await surface.screenshot({ path: join(framesDir, `f${pad(i)}.png`) });
      process.stdout.write(`\r[box-art] ${spec.id}: frame ${i + 1}/${spec.frames}`);
      await sleep(spec.everyMs);
    }
    process.stdout.write("\n");
    for (const key of ["KeyA", "KeyD", "KeyW", "Space"]) {
      await page.keyboard.up(key).catch(() => {});
    }
  } finally {
    await page.close();
  }
}

const nativeTargets = targets.filter((spec) => spec.backend === "native");
const webTargets = targets.filter((spec) => spec.backend === "web");

if (nativeTargets.length && !existsSync(FUNCTOR)) {
  console.error(`no release binary at ${FUNCTOR} — build it with \`npm run build:cli\``);
  process.exit(1);
}

/**
 * A scratch frame directory per card — a failed run leaves no PNGs behind.
 * `BOXART_KEEP_FRAMES=1` keeps (and prints) it, which is how you re-encode a
 * card against a size budget without paying for the capture again.
 */
const shoot = async (spec, capture) => {
  const framesDir = mkdtempSync(join(tmpdir(), `functor-boxart-${spec.id}-`));
  try {
    await capture(framesDir);
    assemble(spec, framesDir, spec.frames);
    if (process.env.BOXART_KEEP_FRAMES) console.log(`[box-art] ${spec.id}: frames kept in ${framesDir}`);
  } finally {
    if (!process.env.BOXART_KEEP_FRAMES) rmSync(framesDir, { recursive: true, force: true });
  }
};

for (const spec of nativeTargets) {
  await shoot(spec, (framesDir) => captureNative(spec, framesDir));
}

if (webTargets.length) {
  await withSite(async (browser) => {
    for (const spec of webTargets) {
      await shoot(spec, (framesDir) => captureWeb(spec, framesDir, browser));
    }
  });
}
