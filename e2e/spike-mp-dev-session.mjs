// SPIKE harness: drive the multiplayer dev-session experience end to end and
// capture it.
//
// This is the verification AND the capture script for the
// `spike/multiplayer-dev-session` branch. It exists because the thing being
// spiked is an EXPERIENCE — "does scrubbing a whole session feel exact, and can
// you see a laggy client trail the server" — and the only honest evidence for
// that is a real browser driving the real page.
//
// It asserts the spike's acceptance bar, through the RENDERED UI rather than
// internals wherever it can:
//
//   1. the dev-server page for examples/mp runs a server + 2 client panes live;
//   2. the chrono bar is docked TOP as a LAYOUT ROW — it sits entirely above the
//      canvas rather than overlaying it;
//   3. the pane chrome tiles the canvas exactly (no stale strip on the right);
//   4. the rail carries a lag comb per client, and dialling a client's link to
//      `mobile` makes THAT client's measured lag grow while the other's does not
//      — i.e. the impairment knob visibly changes divergence;
//   5. dragging the rail rewinds ALL panes together, via NetSim::seek, and is
//      non-destructive (the recorded future survives);
//   6. click-to-focus moves the loud chrome and the bar's ⌨ chip to one pane.
//
// Run manually (owns its own dev server on :8080):
//
//   npm run build:cli:debug      # target/debug/functor embeds this runtime
//   node e2e/spike-mp-dev-session.mjs
//   node e2e/spike-mp-dev-session.mjs --capture /tmp/spike   # + PNG/GIF
//
// `--capture <dir>` additionally writes `frames/*.png`, `still.png`, and (when
// ffmpeg is on PATH) `spike.gif`.
import { spawn, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, rmSync } from "node:fs";
import net from "node:net";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const ROOT = fileURLToPath(new URL("..", import.meta.url));
const BASE = "http://127.0.0.1:8080";
const PORT = 8080;
const PROJECT = "examples/mp";
const ENTRIES = ["server.fun", "client.fun", "client.fun"];

const captureIdx = process.argv.indexOf("--capture");
const CAPTURE = captureIdx > 0 ? process.argv[captureIdx + 1] : null;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function portInUse() {
  return new Promise((resolve) => {
    const sock = net
      .connect(PORT, "127.0.0.1")
      .on("connect", () => {
        sock.destroy();
        resolve(true);
      })
      .on("error", () => resolve(false));
  });
}

async function waitPortFree(timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (!(await portInUse())) return true;
    await sleep(200);
  }
  return false;
}

function cliPath() {
  for (const p of ["target/debug/functor", "target/release/functor"]) {
    if (existsSync(`${ROOT}${p}`)) return `${ROOT}${p}`;
  }
  throw new Error(
    "no functor binary — run `npm run build:cli:debug` first (the CLI embeds the web runtime)",
  );
}

const failures = [];
const pageLog = [];
function check(ok, label, detail = "") {
  if (ok) {
    console.log(`  PASS  ${label}`);
  } else {
    failures.push(label);
    console.log(`  FAIL  ${label}${detail ? ` — ${detail}` : ""}`);
  }
}

async function main() {
  if (!(await waitPortFree(10000))) {
    throw new Error(`:${PORT} is busy — a stale dev server would serve the wrong project`);
  }
  const server = spawn(cliPath(), ["-d", PROJECT, "run", "wasm", "--no-open"], {
    cwd: ROOT,
    stdio: ["ignore", "pipe", "pipe"],
  });
  let serverOutput = "";
  server.stdout.on("data", (d) => (serverOutput += d));
  server.stderr.on("data", (d) => (serverOutput += d));

  const browser = await chromium.launch({
    args: [
      "--use-gl=angle",
      "--use-angle=swiftshader",
      "--enable-unsafe-swiftshader",
      "--ignore-gpu-blocklist",
    ],
  });

  let shot = 0;
  try {
    // 1280x720 deliberately: the design's own stress case ("how a 1280x720
    // laptop uses this feature at all").
    const page = await browser.newPage({ viewport: { width: 1280, height: 720 } });
    page.on("console", (m) => pageLog.push(m.text()));
    page.on("pageerror", (e) => pageLog.push(`pageerror: ${e}`));

    let up = false;
    for (let i = 0; i < 60 && !up; i++) {
      up = await portInUse();
      if (!up) await sleep(500);
    }
    if (!up) {
      throw new Error(
        `the dev server never listened on :${PORT}. Its output was:\n${serverOutput || "(none)"}`,
      );
    }
    for (let i = 0; i < 60; i++) {
      try {
        await page.goto(BASE);
        break;
      } catch {
        await sleep(500);
      }
    }
    await page.waitForFunction(() => typeof window.__sim === "object", null, { timeout: 60000 });
    await sleep(1500); // let the single game finish booting

    if (CAPTURE) {
      rmSync(`${CAPTURE}/frames`, { recursive: true, force: true });
      mkdirSync(`${CAPTURE}/frames`, { recursive: true });
    }
    const grab = async (label) => {
      if (!CAPTURE) return;
      const n = String(shot++).padStart(3, "0");
      await page.screenshot({ path: `${CAPTURE}/frames/f${n}.png` });
      if (label) console.log(`        frame ${n}  ${label}`);
    };
    // Hold on a beat, capturing, so the GIF reads at ~10fps.
    const hold = async (frames, label) => {
      for (let i = 0; i < frames; i++) {
        await grab(i === 0 ? label : null);
        await sleep(90);
      }
    };

    // ---- 1. the session comes up -------------------------------------------
    const count = await page.evaluate((e) => window.__sim.start(e, 1), ENTRIES);
    check(count === 3, "server + 2 client panes start in one page", `got ${count}`);
    // The bar drives itself from rAF, so wait for it to notice the session.
    await page.waitForSelector("#chrono.on", { timeout: 15000 });
    await page.waitForFunction(() => document.querySelectorAll("#panes .pane").length === 3);
    // The bar mounts PAUSED so it never races a harness that steps by hand
    // (e2e/wasm-sim.mjs). Press play, then let the clients connect and the world
    // start moving.
    await page.locator("#c-pause").click();
    await sleep(2500);

    const panesCount = await page.locator("#panes .pane").count();
    check(panesCount === 3, "three pane chromes over three GL panes", `got ${panesCount}`);

    // The role comes from the sim's LIVE routing tables, so it only becomes
    // "server" once a client has connected to the bind. Chrome built from the
    // first frame's answer labels every pane CLIENT forever.
    const roles = await page.evaluate(() =>
      [...document.querySelectorAll("#panes .pane .role")].map((el) => el.textContent.trim()),
    );
    check(
      roles[0] === "server" && roles[1] === "client" && roles[2] === "client",
      "the pane chrome reflects the SETTLED roles, not the first frame's",
      JSON.stringify(roles),
    );
    check(
      (await page.locator("#c-links [data-pane]").count()) === 2,
      "the link row has a group per CLIENT and none for the server",
    );
    check(
      await page.locator("#panes .pane").nth(0).locator(".model").isVisible(),
      "the server pane shows its real model text",
    );
    await hold(6, "live, both clients on LAN");

    // ---- 2. the chrono bar is a TOP-DOCKED LAYOUT ROW ----------------------
    // The claim the design rests on: it reserves its own height above the stage
    // instead of floating over the bottom of the canvas. So its box must end at
    // or before the canvas's top edge — an overlay would fail this.
    const boxes = await page.evaluate(() => {
      const r = (sel) => {
        const el = document.querySelector(sel);
        if (!el) return null;
        const b = el.getBoundingClientRect();
        return { top: b.top, bottom: b.bottom, left: b.left, width: b.width };
      };
      return { chrono: r("#chrono"), canvas: r("#canvas") };
    });
    check(
      boxes.chrono && boxes.canvas && boxes.chrono.bottom <= boxes.canvas.top + 1,
      "the chrono bar is a layout row above the stage, not an overlay",
      `chrono.bottom=${boxes.chrono?.bottom} canvas.top=${boxes.canvas?.top}`,
    );
    check(
      boxes.chrono && Math.abs(boxes.chrono.width - 1280) < 2 && boxes.chrono.top < 2,
      "it is full-width and docked at the very top",
      `top=${boxes.chrono?.top} width=${boxes.chrono?.width}`,
    );

    // ---- 3. the pane chrome tiles the canvas exactly -----------------------
    // The parked branch floored `pane_w` and left a stale strip on the right.
    const tiling = await page.evaluate(() => {
      const canvas = document.querySelector("#canvas").getBoundingClientRect();
      const panes = [...document.querySelectorAll("#panes .pane")].map((el) => {
        const b = el.getBoundingClientRect();
        return { left: b.left - canvas.left, right: b.right - canvas.left };
      });
      return { width: canvas.width, panes };
    });
    const rightEdge = tiling.panes[tiling.panes.length - 1].right;
    check(
      Math.abs(rightEdge - tiling.width) < 1.5,
      "the last pane absorbs the remainder (no stale strip)",
      `right edge ${rightEdge.toFixed(2)} vs canvas ${tiling.width.toFixed(2)}`,
    );

    // ---- 4. lag combs, and the impairment knob -----------------------------
    // Read the lag the UI actually renders (the pane header's ⇅ tag), so this
    // asserts the picture rather than an internal.
    const lags = () =>
      page.evaluate(() =>
        [...document.querySelectorAll("#panes .pane")].map((el) => {
          const m = /\((\d+)f\)/.exec(el.querySelector(".lagtag").textContent || "");
          return m ? Number(m[1]) : null;
        }),
      );
    const combs = () => page.locator("#rail line.comb").count();
    // The comb's x must be INSIDE the pinned gauge, never at rail scale — the
    // whole reason the gauge exists is that 8 frames of lag is sub-pixel on a
    // rail showing thousands of frames.
    const combX = () =>
      page.evaluate(() =>
        [...document.querySelectorAll("#rail line.comb")].map((el) => ({
          pane: Number(el.getAttribute("data-pane")),
          behind: Number(el.getAttribute("data-behind")),
          x: Number(el.getAttribute("x1")),
        })),
      );
    const tlSpan = await page.evaluate(() => {
      const t = window.__sim.timeline();
      return Math.max(1, t.frame - (t.lo ?? 0));
    });
    const lanLags = await lags();
    check(
      lanLags[1] != null && lanLags[2] != null,
      "each client's displayed server frame is MEASURED, not modelled",
      `lags ${JSON.stringify(lanLags)}`,
    );
    check(
      lanLags[1] <= 2 && lanLags[2] <= 2,
      "on LAN the combs sit on the playhead (lag ~0 frames)",
      `lags ${JSON.stringify(lanLags)}`,
    );
    check((await combs()) === 2, "the rail carries one lag comb per client", `got ${await combs()}`);

    // Dial ONLY client 2 (pane 3) up to `mobile`.
    await page.locator("#c-links [data-pane='2'] button", { hasText: "mobile" }).click();
    await sleep(2500);
    await hold(6, "client 2 on `mobile` — combs separate");
    const mobileLags = await lags();
    check(
      mobileLags[2] > lanLags[2] + 2,
      "dialling a client to `mobile` visibly grows ITS lag",
      `client2 ${lanLags[2]}f -> ${mobileLags[2]}f`,
    );
    check(
      Math.abs(mobileLags[1] - lanLags[1]) <= 2,
      "the other client, still on LAN, is unaffected",
      `client1 ${lanLags[1]}f -> ${mobileLags[1]}f`,
    );

    // THE reason the pinned lag gauge exists: a comb's offset must be a fixed
    // ~4px per frame of lag, INDEPENDENT of how many frames the rail spans.
    // Otherwise the most valuable signal in the UI shrinks as the recording
    // grows — exactly when the user dialled latency up to look at it.
    const cx = await combX();
    const lagged = cx.find((c) => c.pane === 2);
    const clean = cx.find((c) => c.pane === 1);
    const railW = (await page.locator("#rail").boundingBox()).width;
    const gap = clean.x - lagged.x;
    const expected = (lagged.behind - clean.behind) * 4;
    check(
      Math.abs(gap - expected) <= 2,
      "the comb sits at GAUGE scale — a fixed 4px per frame of lag",
      `${lagged.behind - clean.behind}f apart -> ${gap.toFixed(1)}px (expected ~${expected})`,
    );
    // Context, not an assertion: what the same lag would be at rail scale. The
    // design quotes 0.9px for 8 frames across 8000 recorded frames; the history
    // ring caps a session well below that, so at THIS length the rail-scale comb
    // would be cramped rather than literally sub-pixel. The gauge still earns its
    // place (4x magnification plus fine-grained seek), but the figure in the doc
    // assumes a longer recording than the ring allows.
    console.log(
      `        (same lag at rail scale: ${(((lagged.behind - clean.behind) / tlSpan) * railW).toFixed(2)}px over ${tlSpan} frames)`,
    );

    // `awful` pushes the comb far enough back that the pane's own cubes are
    // visibly behind the server's, not just numerically behind.
    await page.locator("#c-links [data-pane='2'] button", { hasText: "awful" }).click();
    // Long enough that the coarse-zone drag below still lands on frames RECORDED
    // under `awful` — otherwise we park before the knob was turned and the panes
    // legitimately (but unhelpfully) realign.
    await sleep(4500);
    await hold(8, "client 2 on `awful` — the comb approaches the seam");
    const awfulLags = await lags();
    check(
      awfulLags[2] > mobileLags[2],
      "`awful` pushes the comb further back still",
      `client2 ${mobileLags[2]}f -> ${awfulLags[2]}f`,
    );

    // ---- 5. scrubbing rewinds every pane together -------------------------
    const states = () =>
      page.evaluate(() => [0, 1, 2].map((i) => window.__sim.state(i)));
    const before = await states();
    const tl = await page.evaluate(() => window.__sim.timeline());
    const railBox = await page.locator("#rail").boundingBox();
    // Drag from the middle of the coarse zone: a real pointer drag on the real
    // rail, not a scripted seek.
    await page.mouse.move(railBox.x + railBox.width * 0.9, railBox.y + railBox.height / 2);
    await page.mouse.down();
    for (const f of [0.86, 0.83, 0.81, 0.79]) {
      await page.mouse.move(railBox.x + railBox.width * f, railBox.y + railBox.height / 2);
      await sleep(160);
      await grab(null);
    }
    await page.mouse.up();
    await hold(8, "parked in the past — every pane rewound together");
    const parked = await page.evaluate(() => window.__sim.timeline());
    const after = await states();
    check(
      parked.scrubPos != null && parked.scrubPos < tl.frame,
      "dragging the rail parks the whole session in the past",
      `scrubPos=${parked.scrubPos} live frame=${tl.frame}`,
    );
    check(
      after.every((s, i) => s !== before[i]),
      "ALL THREE panes rewound, each to its own view of that frame",
    );
    check(
      parked.hi >= tl.hi,
      "the seek is non-destructive — the recorded future survives",
      `hi ${tl.hi} -> ${parked.hi}`,
    );
    check(
      (await page.locator("#mode-chip").textContent()).includes("PARKED"),
      "the mode chip says PARKED rather than implying live",
    );

    // The design's headline claim about the combs: parked, they show each
    // client's RECORDED frame for that moment, so the stagger stays — that
    // stagger is real lag, preserved. If the comb data only existed live, this
    // is the check that would catch it.
    const parkedLags = await lags();
    const parkedCombs = await combX();
    check(
      parkedCombs.length === 2 && parkedLags[2] > parkedLags[1] + 2,
      "parked, the combs stay STAGGERED — real lag, preserved",
      `lags ${JSON.stringify(parkedLags)}, combs ${JSON.stringify(parkedCombs)}`,
    );

    // Resume commits the branch and the session runs on.
    await page.locator("#c-pause").click();
    await sleep(1200);
    await hold(6, "resumed — the branch committed");
    const resumed = await page.evaluate(() => window.__sim.timeline());
    check(
      resumed.scrubPos == null && resumed.frame > parked.scrubPos,
      "resuming commits the branch and the session runs on",
      `frame=${resumed.frame} scrubPos=${resumed.scrubPos}`,
    );

    // ---- 6. click-to-focus -------------------------------------------------
    const paneBox = await page.locator("#panes .pane").nth(2).boundingBox();
    await page.mouse.click(paneBox.x + paneBox.width / 2, paneBox.y + paneBox.height * 0.7);
    await sleep(400);
    await hold(6, "click-to-focus — loud chrome on pane 3");
    const focus = await page.evaluate(() => ({
      focused: [...document.querySelectorAll("#panes .pane")].map((el) =>
        el.classList.contains("focused"),
      ),
      chip: document.querySelector("#focus-chip").textContent.trim(),
      owned: document.querySelector("#focus-chip").classList.contains("owned"),
    }));
    check(
      focus.focused.filter(Boolean).length === 1 && focus.focused[2],
      "exactly one pane owns the keyboard, and it is the one clicked",
      JSON.stringify(focus.focused),
    );
    check(
      focus.chip.includes("3") && focus.owned,
      "the bar's ⌨ chip names the owner and is filled",
      `chip="${focus.chip}" owned=${focus.owned}`,
    );

    // Digits jump; the chip follows.
    await page.keyboard.press("2");
    await sleep(300);
    const afterDigit = await page.evaluate(() =>
      document.querySelector("#focus-chip").textContent.trim(),
    );
    check(afterDigit.includes("2"), "a digit jumps focus to that pane", `chip="${afterDigit}"`);
    await hold(6, "digit 2 — focus moved");

    // Esc releases: the chip goes hollow, so "will my keys go to the game?" is
    // answerable at a glance.
    await page.keyboard.press("Escape");
    await sleep(300);
    const released = await page.evaluate(() =>
      document.querySelector("#focus-chip").classList.contains("owned"),
    );
    check(!released, "Esc releases the keyboard and the chip goes hollow");

    if (CAPTURE) {
      await page.screenshot({ path: `${CAPTURE}/still.png` });
      console.log(`        wrote ${CAPTURE}/still.png`);
      const ff = spawnSync("ffmpeg", [
        "-y",
        "-framerate", "10",
        "-pattern_type", "glob",
        "-i", `${CAPTURE}/frames/*.png`,
        "-vf", "scale=960:-1:flags=lanczos,split[a][b];[a]palettegen[p];[b][p]paletteuse",
        "-loop", "0",
        `${CAPTURE}/spike.gif`,
      ], { stdio: "inherit" });
      if (ff.status === 0) console.log(`        wrote ${CAPTURE}/spike.gif`);
      else console.log("        (no ffmpeg on PATH — PNG frames only)");
    }

    // LAST, so it covers everything above: nothing in the page's life may throw.
    const errors = pageLog.filter((l) => /pageerror|\[chrono\] /.test(l));
    check(errors.length === 0, "no page errors while driving the whole experience", errors.join(" | "));
  } finally {
    await browser.close();
    server.kill("SIGTERM");
  }

  console.log("");
  if (failures.length) {
    console.log(`${failures.length} check(s) failed:`);
    for (const f of failures) console.log(`  - ${f}`);
    console.log("\npage log:\n" + pageLog.join("\n"));
    process.exit(1);
  }
  console.log("all checks passed");
}

main().catch((e) => {
  console.error(e);
  console.error("\npage log:\n" + pageLog.join("\n"));
  process.exit(1);
});
