// scripts/capture-hero.mjs
//
// Repeatable capture of the README / functor.games hero GIF + still.
//
// This drives the REAL landing hero (site/index.html): the synthwave scene
// (site/examples/hero.fun) running live in the player iframe, the shared
// time-travel scrubber, AND the little code editor mounted over the card — so
// the GIF shows the source and the edit, exactly like the hand-recorded
// original it replaces. It boots headless (built site + headless Chromium /
// SwiftShader), then scripts the whole hero STORY through the same seams the
// live page uses:
//
//   • window.__hero.setRegion(src)  — types into the code editor and hot-pushes
//     the edited `dot` def (the change is visible IN the editor; the wave keeps
//     rolling because the model is preserved).
//   • the iframe's window.__scrub    — pause + scrub back through the recorded
//     timeline, then toggle the 🔮 extrapolate preview (mode "both": trail +
//     strobe of every dot's future).
//
// It screenshots the `.hero-card` element (scene + scrubber + code panel) at
// each beat and assembles the GIF (ffmpeg two-pass palette) plus a still. The
// scrub + extrapolate beats are DETERMINISTIC (explicit seek / preview state);
// only the short "roll" and "type the edit" beats run in real time.
//
// Prereqs: `npm run build:cli:debug` (or the wasm bundle at
// runtime/functor-runtime-web/pkg), Chromium installed for Playwright, ffmpeg
// on PATH. The script rebuilds the static site (site/build.mjs) so dist always
// reflects the current hero.fun.
//
//   node scripts/capture-hero.mjs [--out docs/media] [--keep-frames] [--no-build]
//
// Default out is a temp dir (printed at the end) so a run never clobbers the
// committed media unless you pass --out docs/media.

import { chromium } from "@playwright/test";
import { spawn, spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
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

// --- the live color edit ----------------------------------------------------
// The editable region is the `dot` def. We recolor ONLY its emissive line so the
// change reads as a single edited line in the small editor (a magenta→cyan-green
// sweep), and the rest of the region is whatever is on disk today. It's pushed
// as ONE valid program (an instant swap, not keystroke-by-keystroke) so the edit
// never passes through a broken intermediate — no error state ever flashes.
const DOT_EMISSIVE_FROM =
  "Scene.emissive(Color.rgb(1.0 - 0.4 * depth, 0.15 + 0.1 * depth, 0.85 - 0.2 * depth))";
const DOT_EMISSIVE_TO =
  "Scene.emissive(Color.rgb(0.15 + 0.15 * depth, 1.0 - 0.35 * depth, 0.75 - 0.25 * depth))";

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
  // 1. Build the static site so dist reflects the CURRENT hero.fun + wasm.
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
  await waitForServer(BASE + "/index.html");

  // 3. Headless Chromium, deterministic software WebGL2 (the wasm-golden flags).
  const browser = await chromium.launch({
    args: ["--use-gl=angle", "--use-angle=swiftshader", "--enable-unsafe-swiftshader"],
  });
  const page = await browser.newPage({ viewport: VIEW, deviceScaleFactor: 1 });
  page.on("pageerror", (e) => console.error("[page error]", String(e)));

  await page.goto(BASE + "/index.html");
  const card = page.locator(".hero-card");
  await card.waitFor({ state: "visible" });

  // The scrubber + __scrub live INSIDE the player iframe; __hero lives on the
  // top window. Resolve the iframe as a Frame we can evaluate against.
  const getFrame = () => page.frames().find((f) => f.url().includes("player.html"));
  const scrub = async (fn, arg) => (await getFrame()).evaluate(fn, arg);

  // 4. Wait until the editor is live AND the iframe is recording a timeline.
  await page.waitForFunction(
    () => window.__hero && window.__hero.status && window.__hero.status().state === "live",
    { timeout: 40000 },
  );
  for (let i = 0; i < 160; i++) {
    const ok = await scrub(() => !!(window.__scrub && window.__scrub.range && window.__scrub.range().length === 2)).catch(() => false);
    if (ok) break;
    await sleep(250);
  }
  await sleep(400);

  let n = 0;
  const shot = async () => card.screenshot({ path: join(frameDir, `f_${String(n++).padStart(4, "0")}.png`) });
  const record = async (count, everyMs = 66) => { for (let i = 0; i < count; i++) { await shot(); await sleep(everyMs); } };

  // --- Beat 1: the wave rolls, original magenta, code on screen ------------
  console.log("[capture-hero] beat 1: rolling + code");
  await record(20);

  // --- Beat 2: edit the emissive line in the editor; scene recolors --------
  console.log("[capture-hero] beat 2: live edit in the editor");
  const region = await page.evaluate(() => window.__hero.region());
  const edited = region.includes(DOT_EMISSIVE_FROM)
    ? region.replace(DOT_EMISSIVE_FROM, DOT_EMISSIVE_TO)
    : (console.warn("[capture-hero] WARNING: emissive line not found in the editable region — update DOT_EMISSIVE_FROM"), region);
  await page.evaluate((src) => window.__hero.setRegion(src), edited);
  // Wait for the (single, valid) hot-swap to go live, then hold on the new
  // code + color.
  await page.waitForFunction(() => window.__hero.status().state === "live", { timeout: 15000 }).catch(() => {});
  await record(26);

  // --- Beat 3: pause + scrub back through the recorded timeline (exact) ----
  console.log("[capture-hero] beat 3: scrub back");
  const [lo, hi] = await scrub(() => window.__scrub.range());
  await scrub(() => { if (!window.__scrub.paused()) window.__scrub.togglePause(); });
  await sleep(80);
  const SCRUB_STEPS = 32;
  const backTo = Math.max(lo, Math.round(lo + (hi - lo) * 0.15));
  for (let i = 0; i <= SCRUB_STEPS; i++) {
    const frame = Math.round(hi - ((hi - backTo) * i) / SCRUB_STEPS);
    await scrub((f) => window.__scrub.seek(f), frame);
    await sleep(45);
    await shot();
  }
  await sleep(120);

  // --- Beat 4: extrapolate — project the future (🔮, mode "both") ----------
  console.log("[capture-hero] beat 4: extrapolate");
  const RATE = 8;
  for (let s = 3; s <= 26; s++) {
    const seconds = s / 10; // grow the forward window 0.3 → 2.6s ahead
    await scrub(([sec, rate]) => window.__scrub.setPreview({ enabled: true, seconds: sec, rate }), [seconds, RATE]);
    await sleep(55);
    await shot();
  }
  await record(14, 70); // hold the full extrapolation

  // 5. High-res still: the extrapolation money shot (card = scene + code + timeline).
  await card.screenshot({ path: join(OUT, "readme-hero.png") });

  await browser.close();
  killServer();

  // 6. Assemble the GIF (two-pass palette for clean neon gradients). Scale to
  //    the card's native width (no upscale).
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
