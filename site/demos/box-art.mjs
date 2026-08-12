// Reproducible capture of the landing page's GAMES CAROUSEL box art: one
// looping GIF + one poster PNG per shortlisted game, into site/media/box-art/.
//
//   npm run demo:box-art                       # every card in site/src/examples.ts
//   node site/demos/box-art.mjs tetris orbs    # just these
//   BOXART_OUT=/tmp/shots node site/demos/box-art.mjs tetris   # elsewhere (scratch runs)
//   DEMO_SKIP_BUILD=1 …                        # reuse site/dist (web backend only)
//   DEMO_FPS=12                                # override the derived gif framerate
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
//     reviewable next to the game. That one-process-per-frame trick REQUIRES the
//     game to be deterministic, so every native card is checked: the first frame
//     is shot twice and the bytes must match (see captureNative).
//
//   web — Playwright against the built site's sandbox, driving one pane. Used for
//     the games one process per frame cannot capture: the multiplayer pair
//     (netpong, orbs), which need an authoritative SERVER — stood up by the
//     sandbox's pane grid + in-page net coordinator — and asteroids, whose wave
//     comes from `Effect.random`, so each native process would seed a different
//     asteroid field. The shot is one pane's surface (`.mp-pane-body`), so a card
//     shows the game rather than the sandbox chrome. netpong plays itself (its
//     client boots `autopilot: true` — the attract mode); orbs and asteroids are
//     played from the keyboard (see DRIVES).
//
// Prereqs: `npm run build:cli` (RELEASE — a debug build's physics/raster is far
// too slow to shoot 300 frames), ffmpeg on PATH, and for the web backend the web
// runtime wasm bundle plus @playwright/test's chromium.
import { spawn, spawnSync, execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, rmSync, statSync, existsSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { GALLERY } from "../src/examples.ts";

const ROOT = fileURLToPath(new URL("../..", import.meta.url));
const OUT_DIR = resolve(process.env.BOXART_OUT || join(ROOT, "site/media/box-art"));
const FUNCTOR = join(ROOT, "target/release/functor");
/** The served site's origin — an OS-assigned port, filled in by `withSite`. */
let base = null;
// Playback rate. A card must play at the rate it was SAMPLED at or the game runs
// in slow motion / fast forward, so it is derived per recipe (see `fpsOf`) rather
// than fixed; DEMO_FPS overrides it for every card.
const FPS_OVERRIDE = process.env.DEMO_FPS ? Number(process.env.DEMO_FPS) : null;

// The card is 16:10 — every game here is authored for a landscape window, so a
// portrait "box" would crop breakout's play field and letterbox the rest. The
// GIF is quarter-size of the capture (and of the poster), which is what keeps it
// inside the ~250 KB budget; the poster stays crisp for HiDPI cards.
const CAPTURE = { width: 640, height: 400 };
// 96 colors is the budget knob that costs the least: these are flat neon scenes,
// so a shorter palette is invisible on the card and roughly halves the bytes a
// full 256-color one spends on dither noise.
// A card renders the GIF at 320 CSS px, so that is its default width. The LIT 3D
// scenes (shooting-range's muzzle flash, code-garden's
// moving daylight) change every pixel of every frame, which defeats GIF's
// inter-frame coding entirely — they override the encode down a notch to stay
// inside the budget, paying a little softness while animating (their POSTER is
// still full-size).
const GIF = { width: 320, height: 200, colors: 96 };
const SCRIPT_DT = 1 / 60;
const BUDGET_BYTES = 250 * 1024;

/**
 * The GIF's framerate: the rate the frames were SAMPLED at, so the card plays at
 * the speed the game ran. Native is exact — every `step` sim frames of `SCRIPT_DT`.
 * The web backend passes its MEASURED rate, because a beat costs `everyMs` plus a
 * screenshot, so `1000/everyMs` would play the card fast. A recipe may pin `fps`
 * to play back deliberately off-speed (code-garden is a timelapse).
 */
const fpsOf = (spec, measured) =>
  FPS_OVERRIDE ?? spec.fps ?? Math.round(measured ?? 1 / (spec.step * SCRIPT_DT));

/**
 * Per-game capture recipes. Native entries name a committed input script in the
 * game's own directory (`--input-script` resolves against `-d`, so it is bare)
 * and a frame window `from`, `from + step`, … `frames` shots long — chosen so the
 * loop lands mid-gameplay, never on a dead board or a title card. Web entries
 * name how many client panes to seat and how the captured pane is driven.
 */
const RECIPES = {
  // The three `Camera2D.create(32, 24)` games render a 4:3 logical space that the
  // runtime pillarboxes inside the 16:10 capture, and the bars take the GL clear
  // color — a visible steel-blue border on the card. `crop: 53` cuts exactly
  // those bars off each side (640 − 2·53 = 534 ≈ 400 · 4/3), so their art ships
  // 4:3 and the card mats it on its own background instead.
  //
  // 30 frames × 6 = 3s of scripted play from frame 170, by which point the script
  // has stacked and cleared rows: pieces are landing, not just falling. It ends
  // before frame ~440, where the blind drive finally tops the board out.
  tetris: { backend: "native", dir: "examples/tetris", script: "boxart.script", from: 170, step: 6, frames: 30, crop: 53 },
  // Turn-based: one keypress every 10 frames, so a 5-frame step reads as a
  // continuous crawl through the fog.
  roguelike: { backend: "native", dir: "examples/roguelike", script: "boxart.script", from: 60, step: 5, frames: 30, crop: 53 },
  // Well past the launch (frame 30): the ball is among the bricks and the score
  // is climbing, with the sweeping paddle keeping the rally going.
  breakout: { backend: "native", dir: "examples/breakout", script: "boxart.script", from: 160, step: 5, frames: 30, crop: 53 },
  // WEB, and not native, despite being single-player: asteroids builds its wave
  // with `Effect.random` on Enter, a REAL effect, so each of the 30 native
  // processes would seed a different asteroid field and the "loop" would be 30
  // unrelated runs spliced together (it was, before this moved). One browser
  // process is one continuous sim, so the motion is coherent.
  asteroids: { backend: "web", clients: 1, drive: "arcade", frames: 34, everyMs: 8, warmup: 40 },
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
    // Deliberately a TIMELAPSE: 15 sim frames per shot played back at 12 fps runs
    // the day about 3× real time, which is what makes the light move on a card.
    backend: "native", dir: "examples/code-garden", script: "idle.script",
    from: 1400, step: 15, frames: 20, fps: 12, gif: { width: 256, height: 160, colors: 64 },
  },
  // Both roles play themselves once a server is seated: the client's `autopilot`
  // starts true, so one client pane is a full AI rally.
  netpong: { backend: "web", clients: 1, networked: true, drive: "idle", frames: 40, everyMs: 8 },
  // Two rivals, shot from client 1: the flown ship AND the other pilot's.
  orbs: {
    backend: "web", clients: 2, networked: true, drive: "fly",
    frames: 40, everyMs: 8, warmup: 60,
  },
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
 *
 * A recipe's `crop` cuts that many pixels of runtime pillarbox off each SIDE of
 * the capture first, and the output then keeps the cropped (4:3) aspect instead
 * of being boxed back to 16:10 — the card `object-fit: contain`s it.
 */
function assemble(spec, framesDir, count, measuredFps) {
  const id = spec.id;
  const fps = fpsOf(spec, measuredFps);
  const out = { ...GIF, ...spec.gif };
  const bars = spec.crop
    ? `crop=${CAPTURE.width - 2 * spec.crop}:${CAPTURE.height}:${spec.crop}:0,`
    : "";
  const box = (w, h) =>
    spec.crop
      ? `${bars}scale=-1:${h}:flags=lanczos`
      : `scale=${w}:${h}:force_original_aspect_ratio=increase:flags=lanczos,crop=${w}:${h}`;
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
    "-framerate", String(fps),
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
  const over = statSync(gif).size > BUDGET_BYTES;
  console.log(
    `[box-art] ${id}: ${count} frames @ ${fps} fps → ${gif} (${kb(gif)})` +
      `${over ? "  ← OVER BUDGET" : ""}, ${poster} (${kb(poster)})`
  );
  return over;
}

/** One process per frame: `--capture-at-frame` shoots a single sim frame. */
function captureNative(spec, framesDir) {
  const dir = join(ROOT, spec.dir);
  const shootFrame = (frame, path) => {
    const result = spawnSync(
      FUNCTOR,
      [
        "-d", dir,
        "run", "native",
        "--input-script", spec.script,
        "--script-dt", String(SCRIPT_DT),
        "--capture-size", `${CAPTURE.width}x${CAPTURE.height}`,
        "--capture-frame", path,
        "--capture-at-frame", String(frame),
      ],
      { cwd: ROOT, stdio: ["ignore", "ignore", "pipe"] }
    );
    if (result.status !== 0 || !existsSync(path)) {
      throw new Error(
        `capture of ${spec.id} at frame ${frame} failed:\n${(result.stderr ?? "").toString().trim()}`
      );
    }
  };
  for (let i = 0; i < spec.frames; i++) {
    shootFrame(spec.from + i * spec.step, join(framesDir, `f${pad(i)}.png`));
    process.stdout.write(`\r[box-art] ${spec.id}: frame ${i + 1}/${spec.frames}`);
  }
  process.stdout.write("\n");
  // The whole native backend rests on one process = one reproducible sim, so
  // PROVE it: reshoot the first frame and demand the same bytes. A game whose
  // logic pulls real entropy (`Effect.random`) seeds a different world in every
  // process, and its "loop" would silently be N unrelated runs spliced together.
  const again = join(framesDir, "determinism-check.png");
  shootFrame(spec.from, again);
  if (!readFileSync(again).equals(readFileSync(join(framesDir, "f0000.png")))) {
    throw new Error(
      `${spec.id}: frame ${spec.from} differs between two identical runs — this game is ` +
        `not deterministic under --input-script (a real effect like Effect.random?), so it ` +
        `cannot be captured one-frame-per-process. Capture it with the web backend instead.`
    );
  }
  rmSync(again, { force: true });
}

/**
 * Serve site/dist once for every web-backed card in this run, on an OS-assigned
 * port read back from the server's own "serving" line. A hard-coded port would be
 * worse than inconvenient: if something else already held it, the readiness poll
 * could succeed against THAT server and the run would capture a plausible card
 * from someone else's build.
 */
async function withSite(run) {
  if (!process.env.DEMO_SKIP_BUILD) {
    const build = spawnSync("node", ["site/build.mjs"], { cwd: ROOT, stdio: "inherit" });
    if (build.status !== 0) process.exit(build.status ?? 1);
  }
  const server = spawn("node", ["site/serve.mjs", "--port", "0"], {
    cwd: ROOT,
    stdio: ["ignore", "pipe", "ignore"],
  });
  const kill = () => server.kill();
  process.on("exit", kill);
  let exited = null;
  server.on("exit", (code) => { exited = code ?? -1; });
  base = await new Promise((resolveBase, reject) => {
    let seen = "";
    const timer = setTimeout(() => reject(new Error("site server never announced a port")), 30000);
    server.stdout.on("data", (chunk) => {
      seen += chunk;
      const match = /http:\/\/127\.0\.0\.1:(\d+)/.exec(seen);
      if (!match) return;
      clearTimeout(timer);
      resolveBase(`http://127.0.0.1:${match[1]}`);
    });
    server.on("exit", () => {
      clearTimeout(timer);
      reject(new Error(`site server exited with ${exited} before serving`));
    });
  });
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

/**
 * How the captured pane is played, per `drive`. `hold` keys go down once and stay
 * down (a LEVEL the game samples); `boot` keys are one press before the warm-up;
 * `beat(page, i)` runs once per sampled beat, counting through the warm-up.
 * `null` is a game that plays itself.
 */
const DRIVES = {
  idle: null,
  // orbs: claiming is a level, so Space is HELD for the whole flight — every orb
  // the ship crosses turns this pilot's colour, which is the game the card shows.
  // Then a repeating turn-and-thrust so the ship visibly flies.
  fly: {
    hold: ["Space"],
    beat: async (page, i) => {
      if (i % 10 !== 0) return;
      const turn = i % 20 < 10 ? "KeyA" : "KeyD";
      await page.keyboard.up("KeyA").catch(() => {});
      await page.keyboard.up("KeyD").catch(() => {});
      await page.keyboard.down(turn);
      await page.keyboard.down("KeyW");
    },
  },
  // asteroids: Enter leaves the menu, then thrust in bursts, swap the rotation
  // every second, and fire on a cadence so rocks are always splitting.
  arcade: {
    boot: ["Enter"],
    beat: async (page, i) => {
      if (i % 15 === 0) {
        const turn = i % 30 < 15 ? "KeyA" : "KeyD";
        await page.keyboard.up("KeyA").catch(() => {});
        await page.keyboard.up("KeyD").catch(() => {});
        await page.keyboard.down(turn);
      }
      if (i % 8 === 0) await page.keyboard.down("KeyW");
      if (i % 8 === 4) await page.keyboard.up("KeyW").catch(() => {});
      await page.keyboard.press("Space");
    },
  },
};
const DRIVE_KEYS = ["KeyA", "KeyD", "KeyW", "Space", "Enter"];

async function captureWeb(spec, framesDir, browser) {
  // Sized for the SHOT, not for the page: 2× so the pane (which is smaller than
  // the 640x400 poster) downsamples into it rather than being upscaled, and no
  // wider than it has to be — a screenshot rasters the whole viewport, so a
  // 1760x900 page at 2× costs ~half a second per frame and the sampled rate
  // collapses to 2 fps. This is wide enough for the pane grid to lay out.
  const page = await browser.newPage({
    viewport: { width: 1180, height: 660 },
    deviceScaleFactor: 2,
  });
  const drive = DRIVES[spec.drive];
  try {
    await page.goto(`${base}/sandbox.html?example=${spec.id}#clients=${spec.clients}`, {
      waitUntil: "load",
    });
    await page.waitForFunction(() => window.__sandbox?.status().state === "live", null, {
      timeout: 120000,
    });
    // A networked sample must also be LINKED through the coordinator before the
    // camera rolls: shooting before the client is seated captures an empty field
    // waiting for its first snapshot. Its server pane is the extra one.
    const panes = spec.clients + (spec.networked ? 1 : 0);
    await page.waitForFunction(
      ([expected, networked]) =>
        document.querySelectorAll(".mp-pane").length === expected &&
        (!networked ||
          [...document.querySelectorAll(".mp-pane .mp-conn")].every(
            (conn) => conn.dataset.linked === "true"
          )),
      [panes, Boolean(spec.networked)],
      { timeout: 120000 }
    );
    const surface = page.locator(".mp-pane").first().locator(".mp-pane-body");
    if (drive) {
      // Hand the pane the real keyboard: mousedown's default action is what
      // moves focus into the iframe, and a synthetic dispatch has none. A
      // networked sample is clicked on its pane HEADER (the pane chrome that
      // selects a client); a solo one has no visible chrome, so click the
      // surface itself.
      await (spec.networked
        ? page.locator(".mp-pane").first().locator(".mp-pane-hd")
        : surface
      ).click();
      await sleep(300);
      for (const key of drive.boot ?? []) await page.keyboard.press(key);
      for (const key of drive.hold ?? []) await page.keyboard.down(key);
    }
    await sleep(1200); // let the rally / the arena settle into motion
    const beat = async (i) => {
      if (drive?.beat) await drive.beat(page, i);
    };
    // Beats before the camera rolls, so the loop opens on a game already in
    // progress — orbs needs a lap of flying for the first claims to land, and
    // asteroids a wave already broken up.
    const warmup = spec.warmup ?? 0;
    for (let i = 0; i < warmup; i++) {
      await beat(i);
      await sleep(spec.everyMs);
    }
    // Time the sampled run: a beat costs `everyMs` PLUS a screenshot (machine- and
    // scene-dependent), and encoding at the nominal rate would play the card fast.
    const started = Date.now();
    for (let i = 0; i < spec.frames; i++) {
      await beat(warmup + i);
      await surface.screenshot({ path: join(framesDir, `f${pad(i)}.png`) });
      process.stdout.write(`\r[box-art] ${spec.id}: frame ${i + 1}/${spec.frames}`);
      await sleep(spec.everyMs);
    }
    const measuredFps = (spec.frames * 1000) / (Date.now() - started);
    process.stdout.write("\n");
    for (const key of DRIVE_KEYS) await page.keyboard.up(key).catch(() => {});
    return measuredFps;
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
const overBudget = [];
const shoot = async (spec, capture) => {
  const framesDir = mkdtempSync(join(tmpdir(), `functor-boxart-${spec.id}-`));
  try {
    // A backend returns its MEASURED sample rate when it has one (the web
    // backend); native's is exact from the recipe, so it returns nothing.
    const measuredFps = await capture(framesDir);
    if (assemble(spec, framesDir, spec.frames, measuredFps)) overBudget.push(spec.id);
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

// The budget is the reason the whole carousel can lazily swap ten animations in,
// so blowing it FAILS the run — the media is already written, so the fix is a
// shorter loop or a `gif:` override, then rerun that card.
if (overBudget.length) {
  console.error(
    `\n[box-art] over the ${BUDGET_BYTES / 1024} KB GIF budget: ${overBudget.join(", ")} — ` +
      `shorten the loop (fewer frames) or add a \`gif: { width, colors }\` override`
  );
  process.exit(1);
}
