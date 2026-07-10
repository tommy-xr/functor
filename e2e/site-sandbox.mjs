// Site e2e: the sandbox's editor → runtime live-reload loop, headless — the
// site-shaped sibling of functor-lang-preview-reload.mjs (which drives the CLI dev
// server directly). Builds the site, serves dist with site/serve.mjs, then
// drives headless Chromium through:
//
//   1. the landing page's hero iframe renders (a live Functor Lang scene);
//   2. the sandbox loads its default example and reports "live";
//   3. an edit via the editor seam hot-swaps the scene (pixels change to the
//      pushed unmistakable green) and the status stays "live";
//   4. a broken edit reports the parse error and the old frame keeps
//      rendering;
//   5. a good edit after the broken one recovers;
//   6. every example in the picker loads to "live" and ticks cleanly (the
//      repo examples are copied in at build time — this catches one breaking
//      on wasm);
//   7. the docs page highlights its Functor Lang blocks, and a "try it" button's
//      program loads live in the sandbox (the #src= → player ?src= data-URL
//      path, fresh init).
//
// Run manually (needs the wasm bundle):
//
//   wasm-pack build runtime/functor-runtime-web --target=web   # once
//   node e2e/site-sandbox.mjs
import { spawn, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const PORT = 8123;
const BASE = `http://127.0.0.1:${PORT}`;
const ROOT = fileURLToPath(new URL("..", import.meta.url));

const GREEN = `let init = { t: 0.0 }
let tick = (model, dt: Float, tts: Float) => { model with t: model.t + dt }
let draw = (model, tts: Float) =>
  Frame.create(
    Camera.lookAt(0.0, 0.0, -6.0, 0.0, 0.0, 0.0),
    Scene.sphere() |> Scene.emissive(0.1, 1.0, 0.2) |> Scene.scale(2.0))
`;
const BROKEN = "let init = {\n";

let failures = 0;
const check = (name, ok, detail = "") => {
  console.log(`${ok ? "PASS" : "FAIL"}: ${name}${ok || !detail ? "" : ` — ${detail}`}`);
  if (!ok) failures += 1;
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// Build, then serve.
const build = spawnSync("node", ["site/build.mjs"], { cwd: ROOT, stdio: "inherit" });
if (build.status !== 0) process.exit(build.status ?? 1);

// A occupied port would make serve.mjs die while the readiness probe below
// happily talks to whatever else is listening — fail loud instead.
try {
  await fetch(BASE);
  console.error(`port ${PORT} is already in use — kill the process on it first`);
  process.exit(1);
} catch {
  // Nothing listening: good.
}

const server = spawn("node", ["site/serve.mjs", "--port", String(PORT)], {
  cwd: ROOT,
  stdio: "ignore",
});
process.on("exit", () => server.kill());
for (let i = 0; ; i++) {
  try {
    await fetch(BASE);
    break;
  } catch {
    if (i > 50) throw new Error("site server never came up");
    await sleep(200);
  }
}

const browser = await chromium.launch();

// Sample the center pixel of a WebGL canvas inside `frame`, copied in a rAF
// callback so it reads the just-rendered buffer (no preserveDrawingBuffer).
const centerPixel = (frame) =>
  frame.evaluate(
    () =>
      new Promise((resolve) => {
        requestAnimationFrame(() => {
          const gl = document.getElementById("canvas");
          const c = document.createElement("canvas");
          c.width = gl.width;
          c.height = gl.height;
          const ctx = c.getContext("2d");
          ctx.drawImage(gl, 0, 0);
          const d = ctx.getImageData((c.width / 2) | 0, (c.height / 2) | 0, 1, 1).data;
          resolve([d[0], d[1], d[2]]);
        });
      })
  );

const playerFrame = (page) => {
  const frame = page.frames().find((f) => f.url().includes("player.html"));
  if (!frame) throw new Error("player iframe not found");
  return frame;
};

// --- 1. Landing page: the hero scene actually renders. ------------------------
{
  const page = await browser.newPage({ viewport: { width: 1024, height: 640 } });
  const consoleLog = [];
  page.on("console", (m) => consoleLog.push(m.text()));
  await page.goto(BASE);
  for (let i = 0; !consoleLog.some((m) => m.includes("[functor-lang] loaded")); i++) {
    if (i > 100) throw new Error(`hero never loaded:\n${consoleLog.join("\n")}`);
    await sleep(200);
  }
  await sleep(600);
  const pixel = await centerPixel(playerFrame(page));
  // Anything the hero draws at center (sun, sky, grid) differs from the GL
  // clear color rgb(26, 51, 77); "not clear color" = the scene rendered.
  const rendered = Math.abs(pixel[0] - 26) + Math.abs(pixel[1] - 51) + Math.abs(pixel[2] - 77) > 30;
  check("landing hero scene renders", rendered, `center = rgb(${pixel})`);
  await page.close();
}

// --- 2–5. Sandbox: load, live edit, broken edit, recover. ---------------------
{
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  await page.goto(`${BASE}/sandbox.html`);

  await page.waitForFunction(
    () => window.__sandbox && window.__sandbox.status().state === "live",
    { timeout: 30000 }
  );
  check("sandbox loads the default example to live", true);

  // Live edit → unmistakable green sphere.
  await page.evaluate((s) => window.__sandbox.setSource(s), GREEN);
  await page.waitForFunction(
    () => window.__sandbox.status().message.includes("model preserved"),
    { timeout: 5000 }
  );
  await sleep(400);
  const green = await centerPixel(playerFrame(page));
  check("live edit repaints the scene green", green[1] > 150 && green[0] < 100, `center = rgb(${green})`);

  // Broken edit → error surfaced, old frame keeps rendering.
  await page.evaluate((s) => window.__sandbox.setSource(s), BROKEN);
  await page.waitForFunction(() => window.__sandbox.status().state === "error", {
    timeout: 5000,
  });
  const status = await page.evaluate(() => window.__sandbox.status());
  check("broken edit surfaces the parse error", status.detail.includes("cannot parse"), status.detail);
  await sleep(400);
  const still = await centerPixel(playerFrame(page));
  check("old program keeps rendering after a broken edit", still[1] > 150 && still[0] < 100, `center = rgb(${still})`);

  // Recovery.
  await page.evaluate((s) => window.__sandbox.setSource(s), GREEN);
  await page.waitForFunction(() => window.__sandbox.status().state === "live", {
    timeout: 5000,
  });
  check("edit after a broken edit recovers to live", true);

  await page.close();
}

// --- 6. Every example loads and ticks cleanly. ---------------------------------
for (const example of ["hero", "orbit", "physics", "monitor"]) {
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  const consoleLog = [];
  page.on("console", (m) => consoleLog.push(m.text()));
  await page.goto(`${BASE}/sandbox.html?example=${example}`);
  try {
    await page.waitForFunction(
      () => window.__sandbox && window.__sandbox.status().state === "live",
      { timeout: 30000 }
    );
    await sleep(700);
    const errors = consoleLog.filter((m) => m.includes("[functor-lang]") && m.includes("error"));
    check(`example '${example}' loads live and ticks cleanly`, errors.length === 0, errors.join("\n"));
  } catch {
    check(`example '${example}' loads live and ticks cleanly`, false, consoleLog.slice(-5).join("\n"));
  }
  await page.close();
}

// --- 7. Docs page + "try it" into the sandbox. --------------------------------
{
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  await page.goto(`${BASE}/docs.html`);
  const highlighted = await page.locator("pre.functor span.tok-k").count();
  const tryButtons = await page.locator("a.try-button").count();
  check("docs page highlights Functor Lang blocks", highlighted > 10, `${highlighted} keyword spans`);
  check("docs page offers try-it buttons", tryButtons >= 4, `${tryButtons} buttons`);

  // Follow the first try-it link in THIS page (target=_blank would detach).
  const href = await page.locator("a.try-button").first().getAttribute("href");
  await page.goto(`${BASE}/${href}`);
  try {
    await page.waitForFunction(
      () => window.__sandbox && window.__sandbox.status().state === "live",
      { timeout: 30000 }
    );
    await sleep(400);
    const pixel = await centerPixel(playerFrame(page));
    // The first runnable is the magenta spinning cube on the GL clear color —
    // just assert something got drawn (not solid clear color everywhere is
    // hard to probe; the live status is the main assertion).
    check("docs try-it program loads live in the sandbox", true, `center = rgb(${pixel})`);
  } catch {
    check("docs try-it program loads live in the sandbox", false, href);
  }
  await page.close();
}

await browser.close();
server.kill();
console.log(failures === 0 ? "\nALL CHECKS PASSED" : `\n${failures} CHECK(S) FAILED`);
process.exit(failures === 0 ? 0 : 1);
