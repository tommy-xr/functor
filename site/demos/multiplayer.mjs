// Reproducible capture of the "Multiplayer" feature GIF: the sandbox's NETWORK
// view, with real packets on real wires.
//
// It opens the sandbox on `examples/orbs` — the multiplayer reference, whose
// `client` and `server` roles are two inline modules of ONE file — at two
// clients, so the pane grid mounts two client panes plus the authoritative
// server pane, all wired through the in-page NetCoordinator (site/src/
// mp-panes.ts, mp-network.ts, net-coordinator.ts).
//
// Then it tells the story the network view exists to tell:
//
//   1. switch to NETWORK: the server becomes the hub, with a measured wire to
//      each client and REAL packets flying along them — client-coloured intent
//      dots up, slate snapshot dots down, each dot's flight time being that
//      packet's actual delay;
//   2. hand client 1 the browser keyboard (a REAL click on its header — a
//      synthetic one has no default action, so focus would never move) and fly
//      it, so intent traffic is the player's, not just the idle stream;
//   3. flip client 1's link chip to `mobile` — 132ms + jitter — and keep
//      flying: that wire's dots visibly crawl beside client 2's LAN wire, and
//      the mid-edge chips say why;
//   4. pin client 1's WIRE LOG by clicking its wire: the same packets, decoded
//      as `Steer {…}` / `Snapshot(…)` values with sent → delivered frames.
//
// Everything is driven through the shipped chrome (the view buttons, the link
// chip's menu, the wire) — the same path e2e/sandbox-mp.mjs asserts — so the
// capture is reproducible and always shows the real product.
//
// Prereqs:
//   - the web runtime wasm bundle:  wasm-pack build runtime/functor-runtime-web --target=web
//   - @playwright/test's chromium (installed with the repo's playwright dep)
//   - ffmpeg on PATH (GIF assembly)
//
//   npm run demo:multiplayer                      # -> site/media/feature-multiplayer.gif (+ .png)
//   node site/demos/multiplayer.mjs my/out.gif    # custom output path
//   DEMO_SKIP_BUILD=1 npm run demo:multiplayer    # reuse an existing site/dist
//   DEMO_PORT / DEMO_FPS                          # override the serve port / gif framerate
import { spawn, spawnSync, execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, rmSync, copyFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const ROOT = fileURLToPath(new URL("../..", import.meta.url));
const PORT = Number(process.env.DEMO_PORT || 8235); // dedicated port (time-travel 8231 … batteries 8234)
const BASE = `http://127.0.0.1:${PORT}`;
const OUT = resolve(process.argv[2] || join(ROOT, "site/media/feature-multiplayer.gif"));
if (!OUT.endsWith(".gif")) {
  console.error(`the output path must end in .gif (got ${OUT})`);
  process.exit(1);
}
const STILL = OUT.replace(/\.gif$/, ".png");
// A wide window so the preview column — the part that is captured — is a
// generous canvas for the graph; the editor column is simply never in shot.
const WIDTH = 1760;
const HEIGHT = 840;
const GIF_WIDTH = 760; // mid-range of the committed feature GIFs (640–900)
const FPS = Number(process.env.DEMO_FPS || 18);

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
// If the child dies (e.g. EADDRINUSE with stdio ignored), fail loud instead of
// letting the readiness poll succeed against some OTHER server on the port —
// that would capture a plausible gif from a stale build.
let serverExit = null;
server.on("exit", (code) => { serverExit = code ?? -1; });
for (let i = 0; ; i++) {
  if (serverExit !== null) {
    throw new Error(`site server exited with code ${serverExit} — is :${PORT} already in use?`);
  }
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
  browser = await chromium.launch({ channel: "chrome" }); // fall back to system Chrome
}
const page = await browser.newPage({ viewport: { width: WIDTH, height: HEIGHT } });
const framesDir = mkdtempSync(join(tmpdir(), "functor-mp-"));
let n = 0;
let stillFrame = null;

try {
  await page.goto(`${BASE}/sandbox.html?example=orbs#clients=2`, { waitUntil: "load" });
  await page.waitForFunction(() => window.__sandbox?.status().state === "live", {
    timeout: 60000,
  });
  // Three panes linked through the coordinator, and the session actually
  // running: a capture that starts before the clients are seated shows empty
  // wires for its first second.
  await page.waitForFunction(
    () =>
      document.querySelectorAll(".mp-pane").length === 3 &&
      [...document.querySelectorAll(".mp-pane .mp-conn")].every(
        (conn) => conn.dataset.linked === "true"
      ),
    null,
    { timeout: 60000 }
  );

  // --- the shot list's own vocabulary, all through the shipped chrome --------
  const shot = async () => {
    const path = join(framesDir, `f${String(n).padStart(4, "0")}.png`);
    await page.locator(".preview-pane").screenshot({ path });
    n++;
    return path;
  };
  /** Hold the shot while the session keeps running — dots live 600ms, so
   * sampling faster than that is what makes them read as motion. */
  const hold = async (frames, ms = 45) => {
    for (let k = 0; k < frames; k++) {
      await shot();
      await sleep(ms);
    }
  };
  /**
   * Fly client 1 while capturing: a repeating turn-and-thrust so its intent
   * stream carries changing values (orbs reads A/D/W off the sampled snapshot),
   * and so the ship visibly moves in both client panes.
   */
  const fly = async (frames, ms = 45) => {
    for (let k = 0; k < frames; k++) {
      const key = k % 24 < 12 ? "KeyA" : "KeyD";
      if (k % 12 === 0) {
        await page.keyboard.up("KeyA").catch(() => {});
        await page.keyboard.up("KeyD").catch(() => {});
        await page.keyboard.down(key);
        await page.keyboard.down("KeyW");
      }
      await shot();
      await sleep(ms);
    }
    await page.keyboard.up("KeyA").catch(() => {});
    await page.keyboard.up("KeyD").catch(() => {});
    await page.keyboard.up("KeyW").catch(() => {});
  };
  const settle = () =>
    page.waitForFunction(
      () =>
        [...document.querySelectorAll(".mp-pane")].every(
          (node) => node.getAnimations().length === 0
        ),
      null,
      { timeout: 5000 }
    );

  // --- 1. Into the NETWORK view: the hub, its wires, its traffic -------------
  await page.click("#mp-view-network");
  await settle();
  // Wait for authority traffic to actually reach the wires before rolling.
  await page.waitForFunction(
    () => document.querySelectorAll(".mp-packet.down").length > 0,
    null,
    { timeout: 30000 }
  );
  await hold(10);

  // --- 2. Give client 1 the real keyboard, and fly it ------------------------
  // A REAL click: mousedown's default action is what re-points focus into the
  // pane's iframe, and a synthetic dispatch has no default action.
  await page.locator(".mp-pane").first().locator(".mp-pane-hd").click();
  await sleep(200);
  await fly(30);

  // --- 3. Degrade client 1's link to `mobile` — the money shot ---------------
  // Through the chip's own menu, the reader's path: open the chip, press the
  // preset, close it again.
  const degraded = await page.evaluate(() => {
    const chip = document.querySelector(".mp-pane .mp-link-chip");
    if (!chip) return false;
    chip.click();
    const preset = [
      ...chip.closest(".mp-link-host").querySelectorAll(".mp-link-presets button"),
    ].find((one) => one.dataset.name === "mobile");
    if (!preset) return false;
    preset.click();
    chip.click();
    return true;
  });
  if (!degraded) throw new Error("could not set client 1's link to `mobile` — the chip menu moved");
  // The chip re-labels itself with what it now does; wait for the graph to pick
  // the new profile up so no frame shows a stale "LAN" on a slowed wire.
  await page.waitForFunction(
    () => /^mobile · 132ms/.test(document.querySelector(".mp-edge-chip")?.textContent.trim() ?? ""),
    null,
    { timeout: 10000 }
  );
  // Re-focus the pane: the chip took the keyboard when it was clicked.
  await page.locator(".mp-pane").first().locator(".mp-pane-hd").click();
  await sleep(200);
  await fly(48);

  // --- 4. Pin the slowed wire's LOG: the same packets, as decoded values -----
  await page.evaluate(() => document.querySelectorAll(".mp-edge-chip")[0].click());
  await sleep(400);
  const pinned = await page.evaluate(() => {
    const panel = document.querySelector(".mp-wirelog");
    return !!panel && !panel.hidden && panel.querySelectorAll(".mp-wl").length > 0;
  });
  if (!pinned) throw new Error("clicking the wire did not pin a populated wire log");
  await page.locator(".mp-pane").first().locator(".mp-pane-hd").click();
  await sleep(150);
  await fly(34);
  // The poster still: the degraded wire AND the pinned log, both in shot.
  stillFrame = join(framesDir, `f${String(n - 12).padStart(4, "0")}.png`);
  await hold(6);

  // 5. Assemble the GIF (palettegen for clean color, lanczos downscale).
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
  copyFileSync(stillFrame, STILL);
  console.log(`\nwrote ${OUT} (${n} frames) and ${STILL}`);
} finally {
  // A failed run must not leave a browser, a server, or a few hundred
  // full-size PNGs behind.
  await browser.close();
  server.kill();
  rmSync(framesDir, { recursive: true, force: true });
}
