// scripts/capture-hero.mjs
//
// Repeatable capture of the README / functor.games hero GIF + still.
//
// This drives the REAL landing hero (site/index.html): the 2D sprite platformer
// (examples/platformer/game.fun, served as examples/platformer.fun) running in
// the player iframe, the shared time-travel scrubber, AND the little code editor
// mounted over the card — so the GIF shows the game, the source, and a live
// edit. It boots headless (built site + headless Chromium / SwiftShader) and
// waits for the page's OWN staging pass, which replays the checked-in
// examples/platformer/jump.script into the timeline and parks two frames before
// takeoff. Then it scripts the hero STORY through the same seams the live page
// uses:
//
//   • the iframe's window.__scrub — scrub the RECORDED timeline forward through
//     the run-up, the jump, and the landing on the far platform (the game
//     playing, frame-exact instead of wall-clock), then back to the parked
//     takeoff frame.
//   • window.__scrub.setPreview — the 🔮 extrapolation (1.3s ahead, rate 8,
//     mode "both": recorded trail + strobed future), the hero-app's own tuned
//     defaults, so the predicted arc over the chasm is drawn ahead of the
//     parked character.
//   • window.__hero.setRegion(src) — swaps ONE line of the editable region
//     (`jumpVelocity`, 13.0 → a value that falls short) as a single valid
//     program. The edit is visible IN the editor, the timeline stays parked
//     (the model is preserved), and the predicted arc immediately redraws
//     shorter — into the chasm.
//
// It screenshots the `.hero-card` element (scene + scrubber + code panel) at
// each beat and assembles the GIF (ffmpeg two-pass palette) plus a still. Every
// beat is DETERMINISTIC: the whole story is explicit seeks, preview state, and
// one source push — nothing runs on wall-clock simulation.
//
// Prereqs: the web-runtime wasm bundle (runtime/functor-runtime-web/pkg, e.g.
// `npm run build:cli:debug`) plus tools/functor-lang-wasm/pkg, Chromium
// installed for Playwright, ffmpeg on PATH. The script rebuilds the static site
// (site/build.mjs) so dist always reflects the current platformer source.
//
//   node scripts/capture-hero.mjs [--out docs/media] [--keep-frames] [--no-build]
//
// Default out is a temp dir (printed at the end) so a run never clobbers the
// committed media unless you pass --out docs/media.

import { chromium } from "@playwright/test";
import { spawn, spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const REPO = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const argv = process.argv.slice(2);
const has = (f) => argv.includes(f);
const opt = (f, d) => { const i = argv.indexOf(f); return i >= 0 ? argv[i + 1] : d; };

const KEEP = has("--keep-frames");
const NO_BUILD = has("--no-build");
const OUT = resolve(opt("--out", mkdtempSync(join(tmpdir(), "hero-media-"))));
const PORT = 8123; // site/serve.mjs default
const BASE = `http://127.0.0.1:${PORT}`;

// The hero stacks single-column below 900px, so the card is nearly full-width
// (~852px, matching the committed 860px GIF). Capture the card, scale the GIF
// to its native width.
const VIEW = { width: 900, height: 1100 };
const FPS = 15;

// --- the live code edit -----------------------------------------------------
// The editable region holds the jump tunables. We weaken ONLY `jumpVelocity`,
// so the change reads as a single edited line in the small editor and its
// consequence is unmistakable: the extrapolated arc no longer reaches the far
// platform. It's pushed as ONE valid program (an instant swap, not
// keystroke-by-keystroke) so the edit never passes through a broken
// intermediate — no error state ever flashes.
const JUMP_FROM = "let jumpVelocity = 13.0";
const JUMP_TO = "let jumpVelocity = 8.0";

// The hero-app's tuned extrapolation defaults (see site/src/hero-app.ts):
// 1.3s reaches the landing, 8 strobe copies/second read as a trajectory, and
// mode 3 draws both the recorded trail and the predicted future.
const PREVIEW_SECONDS = 1.3;
const PREVIEW_RATE = 8;
const PREVIEW_MODE_BOTH = 3;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function waitForServer(url, timeoutMs = 60000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try { if ((await fetch(url)).ok) return; } catch {}
    await sleep(250);
  }
  throw new Error(`server never came up at ${url}`);
}

function sh(cmd, args) {
  const r = spawnSync(cmd, args, { cwd: REPO, stdio: "inherit" });
  if (r.status !== 0) throw new Error(`${cmd} ${args.join(" ")} failed`);
}

async function main() {
  // 1. Build the static site so dist reflects the CURRENT platformer + wasm.
  if (!NO_BUILD) { console.log("[capture-hero] building site → site/dist"); sh("node", ["site/build.mjs"]); }

  const frameDir = mkdtempSync(join(tmpdir(), "hero-frames-"));
  mkdirSync(OUT, { recursive: true });

  // 2. Serve site/dist.
  console.log(`[capture-hero] serving site/dist on :${PORT}`);
  const server = spawn("node", ["site/serve.mjs", "--port", String(PORT)], {
    cwd: REPO, stdio: ["ignore", "inherit", "inherit"],
  });
  const killServer = () => { try { server.kill("SIGTERM"); } catch {} };
  process.on("exit", killServer);
  // On a mid-capture failure the process exits via main().catch — sweep the
  // frame dir then too (Playwright reaps its own browser on exit).
  process.on("exit", () => { if (!KEEP) rmSync(frameDir, { recursive: true, force: true }); });
  await waitForServer(BASE + "/index.html");

  // 3. Headless Chromium, deterministic software WebGL2 (the wasm-golden flags).
  const browser = await chromium.launch({
    args: ["--use-gl=angle", "--use-angle=swiftshader", "--enable-unsafe-swiftshader"],
  });
  const page = await browser.newPage({ viewport: VIEW, deviceScaleFactor: 1 });
  page.on("pageerror", (e) => console.error("[page error]", String(e)));

  // Force recurring multi-step render frames while the hero stages: the baked
  // input script is scheduled inside the runtime's fixed-step loop, and under
  // headless SwiftShader a smooth rAF cadence would not step frames 18/19
  // reliably. (Same trick as e2e/site-sandbox.mjs; cleared once staged.)
  await page.addInitScript(() => {
    if (window !== window.top) return;
    window.__heroFrameStall = window.setInterval(() => {
      const until = performance.now() + 55;
      while (performance.now() < until) {
        // Deliberately occupy the page thread.
      }
    }, 80);
  });

  await page.goto(BASE + "/index.html");
  const card = page.locator(".hero-card");
  await card.waitFor({ state: "visible" });

  // The scrubber + __scrub live INSIDE the player iframe; __hero lives on the
  // top window. Resolve the iframe as a Frame we can evaluate against.
  const getFrame = () => page.frames().find((f) => f.url().includes("player.html"));
  const scrub = async (fn, arg) => (await getFrame()).evaluate(fn, arg);

  // 4. Wait for the page's own staging pass: it replays jump.script into the
  //    timeline, pauses, and parks two frames before takeoff.
  await page.waitForFunction(() => window.__hero && window.__hero.staged(), null, { timeout: 120000 });
  await page.evaluate(() => window.clearInterval(window.__heroFrameStall));
  await sleep(600);

  let n = 0;
  const shot = async () => card.screenshot({ path: join(frameDir, `f_${String(n++).padStart(4, "0")}.png`) });
  const record = async (count, everyMs = 66) => { for (let i = 0; i < count; i++) { await shot(); await sleep(everyMs); } };

  const [lo, hi] = await scrub(() => Array.from(window.__scrub.range()));
  const anchor = await scrub(() => window.__scrub.frame());
  const jumpFrame = await scrub(
    () => (window.__scrub.events().find((e) => e.label === "Up down") || {}).frame ?? null,
  );
  console.log(`[capture-hero] timeline range ${lo}..${hi}, parked at ${anchor}, jump at ${jumpFrame}`);

  const seekTo = async (frame) => {
    const target = Math.max(lo, Math.min(hi, Math.round(frame)));
    await scrub((f) => window.__scrub.seek(f), target);
    // The seek is acknowledged asynchronously by the runtime (requestSeek →
    // pendingSeek), so wait for the playhead to actually land — a wall-clock
    // sleep here would make the per-frame shots machine-dependent.
    await (await getFrame()).waitForFunction(
      (f) => window.__scrub.frame() === f,
      target,
      { timeout: 2000 },
    );
  };

  // --- Beat 1: play the recorded run-up, jump and landing ------------------
  // Frame-exact playback of the timeline the page recorded from jump.script:
  // the character runs off the left platform, jumps, and lands on the right.
  console.log("[capture-hero] beat 1: run-up + jump + landing");
  const runStart = Math.max(lo, (jumpFrame ?? anchor) - 22);
  const landing = Math.min(hi, (jumpFrame ?? anchor) + 62);
  await seekTo(runStart);
  await record(4, 60);
  for (let f = runStart; f <= landing; f += 2) {
    await seekTo(f);
    await shot();
  }
  await record(8, 60); // hold on the landing

  // --- Beat 2: rewind to the parked takeoff frame (exact) -----------------
  console.log("[capture-hero] beat 2: rewind to takeoff");
  const REWIND_STEPS = 18;
  for (let i = 1; i <= REWIND_STEPS; i++) {
    await seekTo(landing + ((anchor - landing) * i) / REWIND_STEPS);
    await shot();
  }
  await seekTo(anchor);
  await record(5, 60);

  // --- Beat 3: 🔮 extrapolate — the predicted arc over the chasm -----------
  console.log("[capture-hero] beat 3: extrapolate");
  await scrub(
    ([seconds, rate, mode]) => window.__scrub.setPreview({ enabled: true, seconds, rate, mode }),
    [PREVIEW_SECONDS, PREVIEW_RATE, PREVIEW_MODE_BOTH],
  );
  await record(18, 70);

  // --- Beat 4: edit jumpVelocity in the editor; the future redraws short ---
  console.log("[capture-hero] beat 4: live edit in the editor");
  const region = await page.evaluate(() => window.__hero.region());
  if (!region.includes(JUMP_FROM)) {
    // Fail loud: capturing without the defining edit would silently produce a
    // GIF whose final beat shows an unchanged arc.
    throw new Error(`"${JUMP_FROM}" not found in the editable region — update JUMP_FROM`);
  }
  const reloadsBefore = await scrub(
    () => window.__scrub.events().filter((e) => e.kind === "reload-ok").length,
  );
  await page.evaluate((src) => window.__hero.setRegion(src), region.replace(JUMP_FROM, JUMP_TO));
  await (await getFrame()).waitForFunction(
    (before) => window.__scrub.events().filter((e) => e.kind === "reload-ok").length > before,
    reloadsBefore,
    { timeout: 20000 },
  );
  await page.waitForFunction(() => window.__hero.status().state === "live", null, { timeout: 20000 });
  // The peek-on-edit animation can briefly override the preview state; re-pin
  // it so the held shot is the full trail + strobe under the edited code.
  await sleep(600);
  await scrub(
    ([seconds, rate, mode]) => window.__scrub.setPreview({ enabled: true, seconds, rate, mode }),
    [PREVIEW_SECONDS, PREVIEW_RATE, PREVIEW_MODE_BOTH],
  );
  await record(26, 70); // hold on the edited code + its shortened future

  // 5. High-res still: the money shot (scene + predicted arc + edited code).
  await card.screenshot({ path: join(OUT, "readme-hero.png") });

  await browser.close();
  killServer();

  // 6. Assemble the GIF (two-pass palette for clean gradients). Scale to the
  //    card's native width (no upscale).
  const dims = spawnSync("ffprobe", ["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width", "-of", "csv=p=0", join(frameDir, "f_0000.png")], { encoding: "utf8" });
  const gifW = Math.min(860, parseInt((dims.stdout || "860").trim(), 10) || 860);
  console.log(`[capture-hero] assembling GIF (${n} frames, ${gifW}px wide)`);
  const palette = join(frameDir, "palette.png");
  const seq = join(frameDir, "f_%04d.png");
  const ff = (args) => { const r = spawnSync("ffmpeg", args, { stdio: "inherit" }); if (r.status !== 0) throw new Error("ffmpeg failed"); };
  ff(["-y", "-i", seq, "-vf", `scale=${gifW}:-1:flags=lanczos,palettegen=stats_mode=diff`, palette]);
  ff(["-y", "-framerate", String(FPS), "-i", seq, "-i", palette,
      "-lavfi", `scale=${gifW}:-1:flags=lanczos [x]; [x][1:v] paletteuse=dither=bayer:bayer_scale=3:diff_mode=rectangle`,
      "-loop", "0", join(OUT, "readme-hero.gif")]);

  if (!KEEP) rmSync(frameDir, { recursive: true, force: true });

  console.log(`\n[capture-hero] done:\n  ${join(OUT, "readme-hero.gif")}\n  ${join(OUT, "readme-hero.png")}`);
  if (KEEP) console.log(`  frames: ${frameDir}`);
  if (OUT !== resolve(REPO, "docs/media")) console.log(`\n  (temp dir; pass --out docs/media to update the committed media)`);
}

main().catch((e) => { console.error(e); process.exit(1); });
