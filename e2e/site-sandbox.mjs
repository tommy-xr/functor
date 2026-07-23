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
//      path, fresh init);
//   8. an inline #src= program with its OWN model shape truly fresh-inits (its
//      init runs — no model carried over from the default example) and ticks
//      cleanly.
//
// Run manually (needs the wasm bundle):
//
//   wasm-pack build runtime/functor-runtime-web --target=web   # once
//   node e2e/site-sandbox.mjs
import { spawn, spawnSync } from "node:child_process";
import { access } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const PORT = Number(process.env.FUNCTOR_SITE_PORT ?? 8123);
const BASE = `http://127.0.0.1:${PORT}`;
const ROOT = fileURLToPath(new URL("..", import.meta.url));

const GREEN = `let init = { t: 0.0 }
let tick = (model, dt: Float, tts: Float) => { model with t: model.t + dt }
let draw = (model, tts: Float) =>
  Frame.create(
    Camera.lookAt(Vec3.make(0.0, 0.0, -6.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.sphere() |> Scene.emissive(Color.rgb(0.1, 1.0, 0.2)) |> Scene.scale(2.0))
`;
const BROKEN = "let init = {\n";

// An inline program whose model shape matches NO served example (`spin` —
// read in both tick and draw): only a fresh `init` runs it cleanly, so this
// catches the sandbox hot-swapping an inline program onto a foreign model.
const INLINE_SPIN = `let init = { spin: 0.0 }
let tick = (model, dt: Float, tts: Float) => { model with spin: model.spin + dt }
let draw = (model, tts: Float) =>
  Frame.create(
    Camera.lookAt(Vec3.make(0.0, 0.0, -6.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.cube() |> Scene.rotateY(Angle.radians(model.spin)) |> Scene.emissive(Color.rgb(1.0, 0.2, 0.8)))
`;

// A model that deliberately retains a module-bound closure. Its old snapshots
// must not cross a hot reload, but the timeline should keep its frame/viewport
// and show the unavailable prefix rather than collapsing or disappearing.
const CLOSURE_HISTORY = `let offset = (k) => (x) => x + k
let init = { t: 0.0, behavior: offset(1.0) }
let tick = (model, dt: Float, tts: Float) => { model with t: model.t + dt }
let draw = (model, tts: Float) =>
  Frame.create(
    Camera.lookAt(Vec3.make(0.0, 0.0, -6.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.cube() |> Scene.rotateY(Angle.radians(model.t)) |> Scene.emissive(Color.rgb(0.2, 0.8, 1.0)))
`;

// A float-model program for the language-intelligence checks: every top-level
// def has a knowable type (no record-typed `init` — a record's type stays
// Unknown and earns no lens), so all four defs get a signature codelens; the
// two unannotated `model` params get inlay hints; and `speed` hovers to its
// type. Loaded via #src= so it fresh-inits (its float model runs cleanly —
// a hot-swap onto the record-model default would throw at draw).
const INTEL_SRC = `let speed = 2.0
let init = 0.0
let tick = (model, dt: Float, tts: Float) => model + dt
let draw = (model, tts: Float) =>
  Frame.create(
    Camera.lookAt(Vec3.make(0.0, 0.0, -6.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.sphere() |> Scene.emissive(Color.rgb(0.1, 1.0, 0.2))
      |> Scene.rotateY(Angle.radians(model)) |> Scene.scale(speed))
`;

let failures = 0;
const check = (name, ok, detail = "") => {
  console.log(`${ok ? "PASS" : "FAIL"}: ${name}${ok || !detail ? "" : ` — ${detail}`}`);
  if (!ok) failures += 1;
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// Build, then serve.
const build = spawnSync("node", ["site/build.mjs"], { cwd: ROOT, stdio: "inherit" });
if (build.status !== 0) process.exit(build.status ?? 1);

// The language-intel pkg is REQUIRED by this suite. build.mjs treats it as
// optional (a site can ship without analysis), but the checks below must not
// silently skip — that is exactly how the editor once shipped degraded while
// CI stayed green.
try {
  await access(`${ROOT}site/dist/pkg/functor_lang_wasm.js`);
} catch {
  console.error(
    "site/dist/pkg/functor_lang_wasm.js missing — build it first: npm run build:lang-wasm"
  );
  process.exit(1);
}

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

// Hash a 32×32 downscale of the whole player canvas (drawn in a rAF callback so
// it reads the just-rendered buffer). Two equal hashes ~300ms apart = the frame
// is frozen; a change = it's animating.
const regionHash = (frame) =>
  frame.evaluate(
    () =>
      new Promise((resolve) => {
        requestAnimationFrame(() => {
          const gl = document.getElementById("canvas");
          const c = document.createElement("canvas");
          c.width = 32;
          c.height = 32;
          const ctx = c.getContext("2d");
          ctx.drawImage(gl, 0, 0, 32, 32);
          const d = ctx.getImageData(0, 0, 32, 32).data;
          let h = 0;
          for (let i = 0; i < d.length; i++) h = (h * 31 + d[i]) >>> 0;
          resolve(h);
        });
      })
  );

// True if any pixel in the lower half of the player canvas is green-dominant
// (g > 150, r < 100) — the hero's dot-grid lives below the horizon, so a green
// recolor of the dots shows up here even though the sun/sky dominate the center.
const lowerHalfGreen = (frame) =>
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
          const y0 = Math.floor(c.height * 0.5);
          const d = ctx.getImageData(0, y0, c.width, c.height - y0).data;
          for (let i = 0; i < d.length; i += 4) {
            if (d[i] < 100 && d[i + 1] > 150) return resolve(true);
          }
          resolve(false);
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

  // The shared player carries the scrubber into the hero iframe too (hidden
  // until history, but the element is present).
  const heroHasScrubber = await playerFrame(page).evaluate(
    () => !!document.getElementById("scrubber")
  );
  check("landing hero player has the scrubber element", heroHasScrubber);

  // The hero mini-sandbox: a live editor over just the `dot` def.
  await page.waitForFunction(
    () => window.__hero && window.__hero.region().includes("let dot"),
    { timeout: 10000 }
  );
  const region = await page.evaluate(() => window.__hero.region());
  check(
    "hero editor shows only the dot region (not the whole file)",
    region.includes("let dot") && !region.includes("let init"),
    region.slice(0, 40)
  );

  // A green edit: recolor the dots' emissive to pure green. The scene must
  // hot-swap with the model preserved (the wave keeps rolling) — no reload.
  const greenRegion = region.replace(
    /Scene\.emissive\(Color\.rgb\([^)]*\)\)/,
    "Scene.emissive(Color.rgb(0.1, 1.0, 0.2))"
  );
  await page.evaluate((s) => window.__hero.setRegion(s), greenRegion);
  await page.waitForFunction(
    () =>
      window.__hero.status().state === "live" &&
      window.__hero.status().message.includes("model preserved"),
    { timeout: 8000 }
  );
  check("hero edit reaches an ok status mentioning model preserved", true);
  await sleep(500);
  const heroGreen = await lowerHalfGreen(playerFrame(page));
  check("hero edit recolors the grid green", heroGreen);

  // A broken edit (unbalanced paren): error surfaced, old frame keeps drawing.
  await page.evaluate((s) => window.__hero.setRegion(s), `${greenRegion}\n(`);
  await page.waitForFunction(() => window.__hero.status().state === "error", {
    timeout: 8000,
  });
  await sleep(300);
  const stillPixel = await centerPixel(playerFrame(page));
  const stillRendered =
    Math.abs(stillPixel[0] - 26) +
      Math.abs(stillPixel[1] - 51) +
      Math.abs(stillPixel[2] - 77) >
    30;
  check(
    "hero broken edit errors and the scene still renders",
    stillRendered,
    `center = rgb(${stillPixel})`
  );

  // Recover with a good edit; the scrubber keeps recording as it runs.
  await page.evaluate((s) => window.__hero.setRegion(s), greenRegion);
  await page.waitForFunction(() => window.__hero.status().state === "live", {
    timeout: 8000,
  });
  const heroPlayer = playerFrame(page);
  await heroPlayer.waitForFunction(
    () => window.__scrub && window.__scrub.range().length === 2,
    { timeout: 10000 }
  );
  const hr0 = await heroPlayer.evaluate(() => window.__scrub.range());
  await sleep(500);
  const hr1 = await heroPlayer.evaluate(() => window.__scrub.range());
  check(
    "hero scrubber still records after edits",
    hr1[1] > hr0[1],
    `${hr0} -> ${hr1}`
  );

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
  // The preview holds an error back (~4s grace) before surfacing it.
  await page.waitForFunction(() => window.__sandbox.status().state === "error", {
    timeout: 8000,
  });
  const status = await page.evaluate(() => window.__sandbox.status());
  check("broken edit surfaces the parse error", /cannot .*:\d+:\d+/.test(status.message), status.message);
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
// Derived from the live picker (not a hardcoded list) so a newly-added example
// is covered automatically — this is the guard that a repo example still runs
// on wasm once it's wired into the sandbox dropdown.
const examples = await (async () => {
  const page = await browser.newPage();
  await page.goto(`${BASE}/sandbox.html`);
  await page.waitForFunction(() => document.getElementById("example-picker")?.options.length > 0);
  const ids = await page.evaluate(() =>
    Array.from(document.getElementById("example-picker").options)
      .map((o) => o.value)
      .filter((v) => v !== "__inline")
  );
  await page.close();
  return ids;
})();
check("picker exposes the expanded example set", examples.length >= 10, examples.join(", "));
// Duplicate ids would silently overwrite each other's dist/examples/<id>.fun and
// under-test the set — a unique-count mismatch is a real drift bug, not a nit.
check("picker example ids are unique", new Set(examples).size === examples.length, examples.join(", "));
for (const example of examples) {
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

// --- 7. Manual + generated API reference. -------------------------------------
{
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  await page.goto(`${BASE}/manual/`);
  const highlighted = await page.locator("pre.functor-lang span.tok-k").count();
  const tryButtons = await page.locator("a.try-button").count();
  check("manual highlights Functor Lang blocks", highlighted > 10, `${highlighted} keyword spans`);
  check("manual offers try-it buttons", tryButtons >= 4, `${tryButtons} buttons`);

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
    check("manual try-it program loads live in the sandbox", true, `center = rgb(${pixel})`);
  } catch {
    check("manual try-it program loads live in the sandbox", false, href);
  }

  await page.goto(`${BASE}/docs/`);
  await page.waitForSelector(".api-item");
  const modules = await page.locator(".api-module").count();
  const declarations = await page.locator(".api-item").count();
  check("API reference renders every generated module", modules === 23, `${modules} modules`);
  check("API reference renders every generated declaration", declarations === 194, `${declarations} declarations`);
  await page.locator("#api-search").fill("Scene.rotateY");
  const visibleDeclarations = await page.locator(".api-item:visible").count();
  check("API reference search narrows declarations", visibleDeclarations === 1, `${visibleDeclarations} visible`);

  await page.goto(`${BASE}/docs.html#get-started`);
  await page.waitForURL(/\/manual\/#get-started$/);
  check("legacy docs.html preserves manual anchors", true, page.url());
  await page.close();
}

// --- 8. Inline #src= program with its OWN model shape fresh-inits. -------------
{
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  const consoleLog = [];
  page.on("console", (m) => consoleLog.push(m.text()));
  const b64u = Buffer.from(INLINE_SPIN).toString("base64url");
  await page.goto(`${BASE}/sandbox.html#src=${b64u}`);
  const name = "inline program with its own model shape fresh-inits and ticks cleanly";
  try {
    await page.waitForFunction(
      () => window.__sandbox && window.__sandbox.status().state === "live",
      { timeout: 30000 }
    );
    await sleep(700);
    // A hot-swap onto the default example's model would blow up on
    // `model.spin` every frame; a fresh init ticks with no runtime errors.
    const errors = consoleLog.filter((m) => m.includes("[functor-lang]") && m.includes("error"));
    check(name, errors.length === 0, errors.join("\n"));
  } catch {
    check(name, false, consoleLog.slice(-5).join("\n"));
  }
  await page.close();
}

// --- 9. Time-travel scrubber drives/observes the player via __scrub. ----------
{
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  const scrubConsole = [];
  page.on("console", (message) => scrubConsole.push(message.text()));
  page.on("pageerror", (error) => scrubConsole.push(`pageerror: ${error.message}`));
  await page.goto(`${BASE}/sandbox.html?example=bounce`);
  await page.waitForFunction(
    () => window.__sandbox && window.__sandbox.status().state === "live",
    { timeout: 30000 }
  );
  const player = playerFrame(page);

  // The seam appears once the scrubber is wired; history then accrues as the
  // scene ticks.
  await player.waitForFunction(() => window.__scrub, { timeout: 10000 });
  const sandboxHasScrubber = await player.evaluate(
    () => !!document.getElementById("scrubber")
  );
  check("sandbox player has the scrubber element", sandboxHasScrubber);
  const customHandles = await player.evaluate(() =>
    ["scrub-playhead", "scrub-preview-handle"].every((id) => {
      const handle = document.getElementById(id);
      return handle?.getAttribute("role") === "slider" && handle.tabIndex === 0;
    })
  );
  check("scrubber exposes two keyboard-focusable slider handles", customHandles);
  const handleColors = await player.evaluate(() => ({
    playhead: getComputedStyle(document.getElementById("scrub-playhead")).backgroundColor,
    preview: getComputedStyle(document.getElementById("scrub-preview-handle")).backgroundColor,
  }));
  check(
    "scrubber handles keep their solid cyan and pink fills",
    handleColors.playhead === "rgb(65, 216, 230)" &&
      handleColors.preview === "rgb(232, 88, 184)",
    JSON.stringify(handleColors)
  );

  await player.waitForFunction(() => window.__scrub.range().length === 2, {
    timeout: 10000,
  });

  // The recorded range grows while running.
  const r0 = await player.evaluate(() => window.__scrub.range());
  await sleep(500);
  const r1 = await player.evaluate(() => window.__scrub.range());
  check("scrubber range grows while running", r1[1] > r0[1], `${r0} -> ${r1}`);

  // Extrapolation is a live mode: its second handle follows the advancing tail
  // by a fixed logical window. Pausing freezes the anchor; it does not enable
  // the control or the renderer.
  await player.evaluate(() => window.__scrub.setPreview({ enabled: true, seconds: 2 }));
  await player.waitForFunction(
    () => getComputedStyle(document.getElementById("scrub-preview-handle")).display === "block",
    { timeout: 3000 }
  );
  const livePreview0 = await player.evaluate(() => window.__scrub.view());
  await sleep(300);
  const livePreview1 = await player.evaluate(() => ({
    view: window.__scrub.view(),
    handleVisible:
      getComputedStyle(document.getElementById("scrub-preview-handle")).display === "block",
    endpointClipped: document
      .getElementById("scrub-preview-handle")
      .classList.contains("fully-clipped"),
  }));
  check(
    "live extrapolation keeps its pink endpoint tracking the live tail",
    !livePreview0.paused &&
      !livePreview1.view.paused &&
      livePreview1.handleVisible &&
      livePreview1.endpointClipped &&
      livePreview1.view.selectedFrame > livePreview0.selectedFrame &&
      livePreview0.previewEndFrame - livePreview0.selectedFrame === 120 &&
      livePreview1.view.previewEndFrame - livePreview1.view.selectedFrame === 120,
    JSON.stringify({ livePreview0, livePreview1 })
  );

  // Markers come from the authoritative runtime log: a real recorded key edge,
  // followed by a real hot-reload boundary from the editor bridge.
  await player.evaluate(() => {
    window.dispatchEvent(new KeyboardEvent("keydown", { code: "Space" }));
    window.dispatchEvent(new KeyboardEvent("keyup", { code: "Space" }));
  });
  await player.waitForFunction(
    () => window.__scrub.events().some((event) => event.kind === "key-down"),
    { timeout: 3000 }
  );
  const inputMarker = await player.evaluate(
    () => !!document.querySelector("#scrub-events .scrub-event.input")
  );
  check("timeline renders recorded input markers", inputMarker);
  const accessibleInputMarkers = await player
    .getByRole("button", { name: /frame \d+, Space down/ })
    .count();
  check(
    "timeline markers are present in the accessibility tree",
    accessibleInputMarkers > 0
  );

  const rangeBeforeSafeReload = await player.evaluate(() => window.__scrub.range());
  await page.evaluate(() =>
    window.__sandbox.setSource(`${window.__sandbox.getSource()}\n// timeline reload marker`)
  );
  await player.waitForFunction(
    () => window.__scrub.events().some((event) => event.kind === "reload-ok"),
    { timeout: 5000 }
  );
  const reloadMarker = await player.evaluate(
    () => !!document.querySelector("#scrub-events .scrub-event.reload")
  );
  check("timeline renders successful hot-reload boundaries", reloadMarker);
  const rangeAfterSafeReload = await player.evaluate(() => window.__scrub.range());
  check(
    "plain-data history remains seekable across a hot reload",
    rangeAfterSafeReload[0] === rangeBeforeSafeReload[0] &&
      rangeAfterSafeReload[1] >= rangeBeforeSafeReload[1],
    `${rangeBeforeSafeReload} -> ${rangeAfterSafeReload}`
  );
  await player.waitForFunction(
    () => {
      const range = window.__scrub.range();
      return range.length === 2 && range[1] - range[0] >= 30;
    },
    { timeout: 3000 }
  );

  // Pause freezes both the frame counter AND the pixels.
  await player.evaluate(() => window.__scrub.togglePause());
  await player.waitForFunction(() => window.__scrub.paused(), { timeout: 3000 });
  const f0 = await player.evaluate(() => window.__scrub.frame());
  const h0 = await regionHash(player);
  await sleep(300);
  const f1 = await player.evaluate(() => window.__scrub.frame());
  const h1 = await regionHash(player);
  check("pause freezes the frame counter", f0 === f1, `${f0} -> ${f1}`);
  check("pause freezes the pixels", h0 === h1, `hash ${h0} -> ${h1}`);

  // Preview duration changes the logical second endpoint, but never the paused
  // viewport. At the live tail the endpoint is clipped and advertised as such.
  const previewBefore = await player.evaluate(() => window.__scrub.view());
  await player.evaluate(() => window.__scrub.setPreview({ enabled: true, seconds: 5 }));
  const previewAfter = await player.evaluate(() => window.__scrub.view());
  check(
    "preview changes keep the paused timeline domain fixed",
    previewBefore.viewport.lo === previewAfter.viewport.lo &&
      previewBefore.viewport.hi === previewAfter.viewport.hi,
    `${JSON.stringify(previewBefore.viewport)} -> ${JSON.stringify(previewAfter.viewport)}`
  );
  check(
    "off-rail extrapolation is clipped without shortening the logical preview",
    previewAfter.previewEndFrame > previewAfter.viewport.hi && previewAfter.previewClippedFrames > 0,
    JSON.stringify(previewAfter)
  );
  const transportAccessibility = await player.evaluate(() => ({
    pause: document.getElementById("scrub-pause").getAttribute("aria-label"),
    extrapolating: document.getElementById("scrub-extrapolate").getAttribute("aria-pressed"),
  }));
  check(
    "transport and extrapolation expose their current state accessibly",
    transportAccessibility.pause === "Resume" && transportAccessibility.extrapolating === "true",
    JSON.stringify(transportAccessibility)
  );
  const clippedHandlesRemainIndependent = await player.evaluate(() => {
    const playhead = document.getElementById("scrub-playhead");
    const preview = document.getElementById("scrub-preview-handle");
    const rect = playhead.getBoundingClientRect();
    return (
      preview.classList.contains("fully-clipped") &&
      document.elementFromPoint(rect.left + rect.width / 2, rect.top + rect.height / 2) === playhead
    );
  });
  check(
    "a fully clipped preview endpoint does not cover the playhead",
    clippedHandlesRemainIndependent
  );

  const frozenBeforeStep = await player.evaluate(() => window.__scrub.view());
  await player.evaluate(() => window.__scrub.step());
  await player.waitForFunction(
    (frame) => window.__scrub.frame() === frame + 1,
    frozenBeforeStep.selectedFrame,
    { timeout: 3000 }
  );
  const frozenAfterStep = await player.evaluate(() => window.__scrub.view());
  check(
    "step advances logically without moving the frozen paused endpoint",
    frozenAfterStep.selectedFrame === frozenBeforeStep.selectedFrame + 1 &&
      frozenAfterStep.viewport.hi === frozenBeforeStep.viewport.hi &&
      frozenAfterStep.playheadClippedAfter,
    JSON.stringify(frozenAfterStep)
  );

  // Markers have generous invisible hit targets, expose hover detail, and seek
  // when selected.
  const markerDetail = await player.evaluate(() => {
    const marker = document.querySelector("#scrub-events .scrub-event-hit");
    marker.dispatchEvent(new MouseEvent("mouseenter"));
    return document.getElementById("scrub-event-detail").textContent;
  });
  check("hovering a marker exposes its frame and label", markerDetail.includes("frame"), markerDetail);
  const selectedMarkerFrame = await player.evaluate(() => {
    const marker = document.querySelector("#scrub-events .scrub-event-hit");
    marker.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    return window.__scrub.model().selectedEventId !== null
      ? Number(marker.getAttribute("aria-label").match(/frame (\d+)/)[1])
      : -1;
  });
  await player.waitForFunction(
    (frame) => Math.abs(window.__scrub.frame() - frame) <= 1,
    selectedMarkerFrame,
    { timeout: 3000 }
  );
  check("selecting a marker seeks to its frame", selectedMarkerFrame >= 0);

  // Seek snaps to a frame within the range.
  const rng = await player.evaluate(() => window.__scrub.range());
  const target = Math.round((rng[0] + rng[1]) / 2);
  await player.evaluate((f) => window.__scrub.seek(f), target);
  await sleep(150);
  const seeked = await player.evaluate(() => window.__scrub.frame());
  check(
    "seek snaps to a frame within range",
    seeked >= rng[0] && seeked <= rng[1] && Math.abs(seeked - target) <= 1,
    `target ${target}, got ${seeked}, range ${rng}`
  );

  // Step advances the frame by exactly 1 while paused.
  const before = await player.evaluate(() => window.__scrub.frame());
  await player.evaluate(() => window.__scrub.step());
  await sleep(150);
  const after = await player.evaluate(() => window.__scrub.frame());
  const afterStepView = await player.evaluate(() => window.__scrub.view());
  check(
    "step advances the frame by exactly 1 while paused",
    after === before + 1,
    `${before} -> ${after}`
  );
  check(
    "stepping from history marks the discarded future as unrecorded",
    afterStepView.recordedEndUnit < 1,
    JSON.stringify(afterStepView)
  );

  // Resume: frames advance again.
  await player.evaluate(() => window.__scrub.togglePause());
  const rf0 = await player.evaluate(() => window.__scrub.frame());
  await sleep(400);
  const rf1 = await player.evaluate(() => window.__scrub.frame());
  check("resume advances frames again", rf1 > rf0, `${rf0} -> ${rf1}`);

  // Rewinding and resuming replaces the first frame of the discarded future.
  // Its markers must be rebuilt from the new branch, not retained from the old
  // history or skipped by the publication cursor.
  await player.evaluate(() => {
    window.dispatchEvent(new KeyboardEvent("keydown", { code: "Space" }));
  });
  await player.waitForFunction(
    () => window.__scrub.events().some((event) => event.label === "Space down"),
    { timeout: 3000 }
  );
  const oldBranchFrame = await player.evaluate(
    () => window.__scrub.events().findLast((event) => event.label === "Space down").frame
  );
  await player.waitForFunction(
    (frame) => window.__scrub.range()[1] >= frame + 4,
    oldBranchFrame,
    { timeout: 3000 }
  );
  await player.evaluate(() => window.__scrub.togglePause());
  await player.waitForFunction(() => window.__scrub.paused(), { timeout: 3000 });
  await player.evaluate((frame) => window.__scrub.seek(frame - 1), oldBranchFrame);
  await player.waitForFunction(
    (frame) => window.__scrub.frame() === frame - 1,
    oldBranchFrame,
    { timeout: 3000 }
  );
  await player.evaluate(() => {
    window.__scrub.togglePause();
    window.dispatchEvent(new KeyboardEvent("keyup", { code: "Space" }));
  });
  await player.waitForFunction(
    (frame) =>
      window.__scrub.events().some((event) => event.frame === frame && event.label === "Space up"),
    oldBranchFrame,
    { timeout: 3000 }
  );
  const branchMarkersAreAuthoritative = await player.evaluate(
    (frame) => {
      const atBranch = window.__scrub.events().filter((event) => event.frame === frame);
      return (
        atBranch.some((event) => event.label === "Space up") &&
        !atBranch.some((event) => event.label === "Space down")
      );
    },
    oldBranchFrame
  );
  check(
    "branching replaces discarded-future markers with authoritative inputs",
    branchMarkersAreAuthoritative
  );

  // A safe reload while scrubbed is non-destructive: it keeps the selected
  // cursor AND the complete recorded future. Step/Resume branches later.
  await player.waitForFunction(() => !window.__scrub.paused(), { timeout: 3000 });
  await player.waitForFunction(() => {
    const range = window.__scrub.range();
    return range.length === 2 && range[1] - range[0] >= 4;
  });
  await player.evaluate(() => window.__scrub.togglePause());
  await player.waitForFunction(() => window.__scrub.paused(), { timeout: 3000 });
  // Capture the domain only after Pause has taken effect. Frames can still be
  // published between the earlier running-state probe and this boundary.
  const reloadWhileScrubbed = await player.evaluate(() => ({
    hi: window.__scrub.range()[1],
    viewportHi: window.__scrub.view().viewport.hi,
    hadUnavailableHistory: window.__scrub.view().hasUnavailableHistory,
    lastId: Math.max(-1, ...window.__scrub.events().map((event) => event.id)),
  }));
  await player.evaluate((hi) => window.__scrub.seek(hi - 2), reloadWhileScrubbed.hi);
  await player.waitForFunction(
    (hi) => window.__scrub.frame() === hi - 2,
    reloadWhileScrubbed.hi,
    { timeout: 3000 }
  );
  const selectedBeforeReload = reloadWhileScrubbed.hi - 2;
  await page.evaluate(() =>
    window.__sandbox.setSource(`${window.__sandbox.getSource()}\n// reload while scrubbed marker`)
  );
  await player.waitForFunction(
    (lastId) =>
      window.__scrub
        .events()
        .some((event) => event.id > lastId && event.kind === "reload-ok"),
    reloadWhileScrubbed.lastId,
    { timeout: 5000 }
  );
  const scrubbedReloadMarker = await player.evaluate(
    (lastId) =>
      window.__scrub
        .events()
        .find((event) => event.id > lastId && event.kind === "reload-ok"),
    reloadWhileScrubbed.lastId
  );
  const reloadTransportIsVisible = await player.evaluate(() => {
    const scrubber = document.getElementById("scrubber");
    const step = document.getElementById("scrub-step");
    return (
      getComputedStyle(scrubber).display === "flex" &&
      getComputedStyle(step).visibility !== "hidden" &&
      !step.disabled
    );
  });
  check("reload boundary keeps the visible Step/Resume transport", reloadTransportIsVisible);
  const safeReloadView = await player.evaluate(() => window.__scrub.view());
  check(
    "paused plain-data reload keeps its selected frame and complete future",
    safeReloadView.selectedFrame === selectedBeforeReload &&
      safeReloadView.recorded.lo < selectedBeforeReload &&
      safeReloadView.recorded.hi === reloadWhileScrubbed.hi &&
      safeReloadView.viewport.hi === reloadWhileScrubbed.viewportHi &&
      safeReloadView.hasUnavailableHistory === reloadWhileScrubbed.hadUnavailableHistory,
    JSON.stringify(safeReloadView)
  );
  await player.locator("#scrub-step").click();
  await sleep(500);
  const postReloadStep = await player.evaluate(() => ({
    paused: window.__scrub.paused(),
    frame: window.__scrub.frame(),
    range: window.__scrub.range(),
    view: window.__scrub.view(),
  }));
  check(
    "stepping after a safe reload branches without shrinking the visual total",
      postReloadStep.range.length === 2 &&
      postReloadStep.range[1] === selectedBeforeReload + 1 &&
      postReloadStep.view.viewport.hi === reloadWhileScrubbed.viewportHi &&
      postReloadStep.view.hasUnavailableHistory &&
      postReloadStep.view.unavailableAfterStartUnit < 1,
    JSON.stringify({ postReloadStep, scrubConsole: scrubConsole.slice(-8) })
  );
  const preservedRailStartsAtHistoryFloor = await player.evaluate(
    () => Number(document.getElementById("scrub-played").getAttribute("x")) === 0
  );
  check(
    "preserved history keeps its cyan rail before the reload boundary",
    preservedRailStartsAtHistoryFloor
  );
  check(
    "reload while scrubbed marks the selected frame without branching",
    scrubbedReloadMarker.frame === selectedBeforeReload,
    JSON.stringify({ reloadWhileScrubbed, scrubbedReloadMarker })
  );

  await page.close();
}

// --- 10. Closure-bearing reloads retain UI continuity at a safe boundary. ---
{
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  const b64u = Buffer.from(CLOSURE_HISTORY).toString("base64url");
  await page.goto(`${BASE}/sandbox.html#src=${b64u}`);
  await page.waitForFunction(
    () => window.__sandbox && window.__sandbox.status().state === "live",
    { timeout: 30000 }
  );
  const player = playerFrame(page);
  await player.waitForFunction(
    () => window.__scrub?.range().length === 2 && window.__scrub.range()[1] >= 30,
    { timeout: 10000 }
  );
  await player.evaluate(() => window.__scrub.togglePause());
  await player.waitForFunction(() => window.__scrub.paused(), { timeout: 3000 });
  await player.evaluate(() => window.__scrub.setPreview({ enabled: true, seconds: 2 }));
  const reloadTarget = await player.evaluate(() => {
    const [lo, hi] = window.__scrub.range();
    return Math.round(lo + (hi - lo) * 0.4);
  });
  await player.evaluate((frame) => window.__scrub.seek(frame), reloadTarget);
  await player.waitForFunction(
    (frame) => window.__scrub.frame() === frame,
    reloadTarget,
    { timeout: 3000 }
  );
  const beforeReload = await player.evaluate(() => ({
    frame: window.__scrub.frame(),
    view: window.__scrub.view(),
    lastId: Math.max(-1, ...window.__scrub.events().map((event) => event.id)),
  }));

  await page.evaluate(() =>
    window.__sandbox.setSource(`${window.__sandbox.getSource()}\n// closure history boundary`)
  );
  await player.waitForFunction(
    (lastId) =>
      window.__scrub.events().some(
        (event) => event.id > lastId && event.kind === "reload-ok"
      ),
    beforeReload.lastId,
    { timeout: 5000 }
  );
  const afterReload = await player.evaluate(() => {
    const view = window.__scrub.view();
    return {
      frame: window.__scrub.frame(),
      range: window.__scrub.range(),
      view,
      stripeWidth: Number(document.getElementById("scrub-unavailable").getAttribute("width")),
      stripeAfterWidth: Number(
        document.getElementById("scrub-unavailable-after").getAttribute("width")
      ),
      playheadVisible: getComputedStyle(document.getElementById("scrub-playhead")).display,
      previewVisible: getComputedStyle(document.getElementById("scrub-preview-handle")).display,
      playheadValueText: document.getElementById("scrub-playhead").getAttribute("aria-valuetext"),
      label: document.getElementById("scrub-count").textContent,
      reloadFrame: window.__scrub.events().findLast((event) => event.kind === "reload-ok").frame,
    };
  });
  check(
    "closure reload keeps the paused frame and frozen viewport",
    afterReload.frame === beforeReload.frame &&
      afterReload.view.selectedFrame === beforeReload.view.selectedFrame &&
      afterReload.view.viewport.lo === beforeReload.view.viewport.lo &&
      afterReload.view.viewport.hi === beforeReload.view.viewport.hi,
    JSON.stringify({ beforeReload, afterReload })
  );
  check(
    "closure reload seeds a one-frame seekable generation at the boundary",
    afterReload.range[0] === beforeReload.frame &&
      afterReload.range[1] === beforeReload.frame &&
      afterReload.reloadFrame === beforeReload.frame,
    JSON.stringify(afterReload)
  );
  check(
    "unavailable history stays striped without cluttering the frame counter",
    afterReload.view.hasUnavailableHistory &&
      afterReload.stripeWidth > 0 &&
      afterReload.stripeAfterWidth > 0 &&
      afterReload.playheadVisible === "block" &&
      afterReload.previewVisible === "block" &&
      afterReload.playheadValueText.includes(
        `recorded frames ${beforeReload.frame} to ${beforeReload.frame}`
      ) &&
      afterReload.playheadValueText.includes("striped history") &&
      afterReload.playheadValueText.includes("unavailable") &&
      afterReload.label.includes(String(beforeReload.frame)) &&
      !afterReload.label.includes("reload boundary"),
    JSON.stringify(afterReload)
  );

  await player.evaluate((frame) => window.__scrub.seek(frame), beforeReload.view.viewport.lo);
  await sleep(150);
  const refusedOldSeek = await player.evaluate(() => window.__scrub.frame());
  check(
    "striped pre-reload frames are not seekable",
    refusedOldSeek === beforeReload.frame,
    `${beforeReload.view.viewport.lo} -> ${refusedOldSeek}`
  );
  await page.close();
}

// --- 11. The editor language-intelligence wasm analyzes source in-browser. -----
// Commits 7-8 wire this into the CodeMirror editor (diagnostics/hover); here we
// just smoke-test the bundle loads and `functor_lang_analyze` reports errors on
// a bad source and none on a clean one.
{
  const page = await browser.newPage({ viewport: { width: 800, height: 600 } });
  await page.goto(BASE); // any same-origin page; we only need /pkg/ reachable
  const result = await page.evaluate(async () => {
    let mod;
    try {
      mod = await import("/pkg/functor_lang_wasm.js");
    } catch {
      return null; // fails below — the pkg is guaranteed present (startup check)
    }
    await mod.default(); // init the wasm
    // A type error: adding a string to a float.
    const bad = JSON.parse(mod.functor_lang_analyze('let bad = 1.0 + "x"\n'));
    // A clean program using prelude names.
    const clean = JSON.parse(
      mod.functor_lang_analyze(
        "let draw = (model, tts: Float) =>\n" +
          "  Frame.create(Camera.lookAt(Vec3.make(0.0, 0.0, -6.0), Vec3.make(0.0, 0.0, 0.0)), Scene.cube())\n"
      )
    );
    return { bad, clean };
  });
  if (!result) {
    check("language wasm analyzes source in-browser", false, "the pkg failed to import/init");
    await page.close();
  } else {
  const d = result.bad.diagnostics;
  const sane =
    Array.isArray(d) &&
    d.length >= 1 &&
    Number.isInteger(d[0].from) &&
    Number.isInteger(d[0].to) &&
    d[0].from < d[0].to;
  check(
    "language wasm analyzes source in-browser (error on bad, none on clean)",
    sane && result.clean.diagnostics.length === 0,
    `bad=${JSON.stringify(d)} clean=${result.clean.diagnostics.length}`
  );
  await page.close();
  }
}

// --- 11. Live diagnostics: the linter underlines a type error, clears on fix. --
// A valid MVU program (loads & runs live — type diagnostics are advisory in the
// dev loop) with ONE unused function whose body is a type error the checker
// flags. The `.cm-lintRange-error` underline must appear, then clear when the
// bad def is removed — all while the push loop keeps the status pill live.
{
  const CLEAN = GREEN;
  const TYPE_ERROR = `${GREEN}let oops = (x: Float) => x + "type error"\n`;

  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  await page.goto(`${BASE}/sandbox.html`);
  await page.waitForFunction(
    () => window.__sandbox && window.__sandbox.status().state === "live",
    { timeout: 30000 }
  );

  // Await the analysis wasm's readiness. The pkg is guaranteed present (the
  // startup check), so not-ready here means it failed to load/init — a failure,
  // never a skip.
  const langAvailable = await page.evaluate(
    () => window.__lang && window.__lang.ready
  );
  if (!langAvailable) {
    check("live diagnostics: language analysis is ready", false, "__lang not ready");
    await page.close();
  } else {
    const lintCount = () => page.locator(".cm-lintRange-error").count();
    // Poll for the count to reach a predicate (covers the 300ms lint delay).
    const waitLint = async (pred, timeout = 6000) => {
      const t0 = Date.now();
      for (;;) {
        if (pred(await lintCount())) return true;
        if (Date.now() - t0 > timeout) return false;
        await sleep(150);
      }
    };

    await page.evaluate((s) => window.__sandbox.setSource(s), TYPE_ERROR);
    const gotError = await waitLint((n) => n > 0);
    check("type error draws a lint underline", gotError, `count=${await lintCount()}`);
    // Await the hot-swap RESULT before reading liveness: the debounced push
    // (busy → live) can be mid-flight right when the underline appears, so a
    // bare status() read would intermittently catch the transient "reloading".
    // TYPE_ERROR still loads and runs (type diagnostics are advisory), so the
    // push reports "model preserved".
    await page.waitForFunction(
      () => window.__sandbox.status().message.includes("model preserved"),
      { timeout: 8000 }
    );
    check(
      "diagnostics keep the sandbox live",
      (await page.evaluate(() => window.__sandbox.status().state)) === "live"
    );

    await page.evaluate((s) => window.__sandbox.setSource(s), CLEAN);
    const cleared = await waitLint((n) => n === 0);
    check("fixing the type error clears the underline", cleared, `count=${await lintCount()}`);
    // Same as above: wait for the fix's push to round-trip before asserting live.
    await page.waitForFunction(
      () => window.__sandbox.status().message.includes("model preserved"),
      { timeout: 8000 }
    );
    check(
      "sandbox returns/stays live after the fix",
      (await page.evaluate(() => window.__sandbox.status().state)) === "live"
    );

    await page.close();
  }
}

// --- 12. Hover types + inlay hints + codelens (commit 8). ---------------------
// The intel program loads fresh via #src=; once the analysis pkg is ready the
// editor grows inline `: float` inlays, a signature codelens above each def,
// and a hover tooltip — all while the push loop keeps the status pill live.
{
  const b64u = Buffer.from(INTEL_SRC).toString("base64url");
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  await page.goto(`${BASE}/sandbox.html#src=${b64u}`);
  await page.waitForFunction(
    () => window.__sandbox && window.__sandbox.status().state === "live",
    { timeout: 30000 }
  );

  const langAvailable = await page.evaluate(() => window.__lang && window.__lang.ready);
  if (!langAvailable) {
    check("hover/inlay/codelens: language analysis is ready", false, "__lang not ready");
    await page.close();
  } else {
    // The signature lens appears once per top-level def; count them in the source.
    const topDefs = INTEL_SRC.split("\n").filter((l) => l.startsWith("let ")).length;

    // Inlays and lenses lag the doc by the lint debounce (they read the cache
    // the lint pass fills), so poll rather than sampling once.
    const poll = async (fn, pred, timeout = 8000) => {
      const t0 = Date.now();
      for (;;) {
        const v = await fn();
        if (pred(v)) return v;
        if (Date.now() - t0 > timeout) return v;
        await sleep(150);
      }
    };

    const inlays = await poll(() => page.locator(".cm-inlay").count(), (n) => n > 0);
    check("inlay hints decorate unannotated params", inlays > 0, `count=${inlays}`);

    const lenses = await poll(() => page.locator(".cm-lens").count(), (n) => n >= topDefs);
    check(
      "codelens shows a signature above every top-level def",
      lenses >= topDefs,
      `lenses=${lenses}, defs=${topDefs}`
    );

    // Hover a REAL code token (skip the lens/inlay widget text) and rest the
    // mouse over it — a jiggle would keep resetting the hover timer.
    const coord = await page.evaluate(() => {
      const content = document.querySelector(".cm-content");
      const walker = document.createTreeWalker(content, NodeFilter.SHOW_TEXT);
      let node;
      while ((node = walker.nextNode())) {
        if (node.parentElement.closest(".cm-lens, .cm-inlay")) continue; // widget text
        const idx = node.textContent.indexOf("speed");
        if (idx >= 0) {
          const range = document.createRange();
          range.setStart(node, idx);
          range.setEnd(node, idx + 5);
          const r = range.getBoundingClientRect();
          return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
        }
      }
      return null;
    });
    check("found a hoverable token in the editor", !!coord, JSON.stringify(coord));
    if (coord) {
      await page.mouse.move(coord.x - 40, coord.y);
      await sleep(100);
      await page.mouse.move(coord.x, coord.y);
      const tip = await poll(
        async () => {
          const el = page.locator(".cm-tooltip-hover");
          return (await el.count()) ? (await el.first().textContent()) || "" : "";
        },
        (t) => t.includes(":")
      );
      check("hover shows a type tooltip", tip.includes(":"), `tooltip=${JSON.stringify(tip)}`);
    }

    check(
      "language intelligence keeps the sandbox live",
      (await page.evaluate(() => window.__sandbox.status().state)) === "live"
    );

    await page.close();
  }
}

// --- 12b. Status bar: Problems + Output. ---------------------------------------
// The bottom strip's Problems tab mirrors the lint pass (count + clickable
// rows that jump the editor), and the Output panel receives runtime console
// traces (`Debug.log`, forwarded from the player iframe) plus reload results.
{
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  await page.goto(`${BASE}/sandbox.html`);
  await page.waitForFunction(
    () => window.__sandbox && window.__sandbox.status().state === "live",
    { timeout: 30000 }
  );

  const problemsTab = page.locator('.statusbar-tab[data-tab="problems"]');
  const outputTab = page.locator('.statusbar-tab[data-tab="output"]');
  const tabText = async (tab) => ((await tab.textContent()) || "").trim();
  const waitFor = async (fn, pred, timeout = 8000) => {
    const t0 = Date.now();
    for (;;) {
      const v = await fn();
      if (pred(v)) return v;
      if (Date.now() - t0 > timeout) return v;
      await sleep(150);
    }
  };

  // A type error fills the Problems tab and panel.
  const BAD = `${GREEN}let oops = (x: Float) => x + "status bar"\n`;
  await page.evaluate((s) => window.__sandbox.setSource(s), BAD);
  const flagged = await waitFor(() => tabText(problemsTab), (t) => t.includes("1 problem"));
  check("problems tab counts the type error", flagged.includes("1 problem"), flagged);

  await problemsTab.click();
  const row = page.locator(".problem-row");
  const rowText = await waitFor(
    async () => ((await row.count()) ? await row.first().textContent() : ""),
    (t) => t.includes("game.fun")
  );
  check(
    "problems panel lists the diagnostic with its location",
    rowText.includes("float") && rowText.includes("game.fun"),
    rowText
  );

  // Clicking the row jumps + focuses the editor.
  await row.first().click();
  const focused = await page.evaluate(() =>
    document.activeElement ? document.activeElement.classList.contains("cm-content") : false
  );
  check("clicking a problem focuses the editor", focused);

  // Fixing the error empties the tab back out.
  await page.evaluate((s) => window.__sandbox.setSource(s), GREEN);
  const cleared = await waitFor(() => tabText(problemsTab), (t) => t.includes("0 problems"));
  check("fixing the error resets the problems tab", cleared.includes("0 problems"), cleared);

  // A top-level Debug.log fires on the hot-swap and lands in Output (the
  // player forwards its console), alongside the reload-result lines.
  await page.evaluate(
    (s) => window.__sandbox.setSource(s),
    `${GREEN}let boot = Debug.log("status-probe", 42.0)\n`
  );
  await outputTab.click();
  const outputLines = await waitFor(
    () => page.locator(".output-line").allTextContents(),
    (lines) => lines.some((l) => l.includes("status-probe"))
  );
  check(
    "Debug.log reaches the Output panel",
    outputLines.some((l) => l.includes("status-probe")),
    JSON.stringify(outputLines.slice(-4))
  );
  check(
    "reload results reach the Output panel",
    outputLines.some((l) => l.includes("model preserved")),
    JSON.stringify(outputLines.slice(-4))
  );
  // Runtime lines carry a `[Frame N | HH:MM:SS]` preamble (the game was
  // already running when the hot-swap re-evaluated the Debug.log).
  const probeLine = outputLines.find((l) => l.includes("status-probe")) || "";
  check(
    "output lines carry a [Frame N | time] preamble",
    /^\[Frame \d+ \| \d{2}:\d{2}:\d{2}\]/.test(probeLine),
    probeLine
  );

  await page.close();
}

// --- 12c. Live values while paused (the inspector overlay). --------------------
// Pausing via the player's scrubber relays the trace to the page; the editor
// grows cyan `= value` live inlays next to binders AND variable reads, the
// executions tab lists the frame's entry-point runs (tick + the synthesized
// draw), and any edit clears the overlay instantly (hash gate).
{
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  await page.goto(`${BASE}/sandbox.html`);
  await page.waitForFunction(
    () => window.__sandbox && window.__sandbox.status().state === "live",
    { timeout: 30000 }
  );
  await page.waitForFunction(() => window.__lang && window.__lang.ready, { timeout: 15000 });

  const waitFor = async (fn, pred, timeout = 10000) => {
    const t0 = Date.now();
    for (;;) {
      const v = await fn();
      if (pred(v)) return v;
      if (Date.now() - t0 > timeout) return v;
      await sleep(150);
    }
  };

  // Let a few frames run, then pause via the real scrubber button.
  await sleep(800);
  await playerFrame(page).evaluate(() => document.getElementById("scrub-pause")?.click());

  // Live inlays appear (the relay → setLiveTrace → hash gate → overlay path).
  const liveCount = await waitFor(() => page.locator(".cm-live-value").count(), (n) => n > 0);
  check("pausing shows live-value inlays in the editor", liveCount > 0, `count=${liveCount}`);
  const liveTexts = await page.locator(".cm-live-value").allTextContents();
  check(
    "live inlays carry `= value` previews",
    liveTexts.every((t) => t.startsWith("= ")) && liveTexts.length > 0,
    JSON.stringify(liveTexts.slice(0, 4))
  );
  // The hero's dot-grid loop sites (×120) sweep numerically — a multi-hit
  // numeric site renders its RANGE, not the last sample: `= 0…11 (×120)`.
  check(
    "numeric loop sites render min…max ranges",
    liveTexts.some((t) => /^= -?[\d.]+…-?[\d.]+ \(×\d+\)$/.test(t)),
    JSON.stringify(liveTexts.filter((t) => t.includes("×")).slice(0, 4))
  );

  // Position invariant: every hint's name span slices to exactly its name.
  // hero.fun's comments contain multibyte characters (em dashes) BEFORE the
  // bindings, so this fails loudly if the trace's UTF-8 byte offsets ever
  // reach the editor unconverted.
  const misplaced = await page.evaluate(() => {
    const doc = window.__sandbox.source();
    return window.__lang
      .liveHints()
      .filter((h) => doc.slice(h.nameStart, h.nameEnd) !== h.name)
      .map((h) => `${h.name}@${h.nameStart}=${JSON.stringify(doc.slice(h.nameStart, h.nameEnd))}`);
  });
  check("live hints sit exactly on their names (byte→UTF-16)", misplaced.length === 0, JSON.stringify(misplaced));

  // The executions picker lists the frame's runs, draw included.
  const execTab = page.locator('.statusbar-tab[data-tab="executions"]');
  const tabText = ((await execTab.textContent()) || "").trim();
  check("executions tab counts the paused frame's runs", /⏸ \d+ executions/.test(tabText), tabText);
  await execTab.click();
  const rows = await waitFor(
    () => page.locator(".exec-row").allTextContents(),
    (rs) => rs.some((r) => r.startsWith("draw"))
  );
  check(
    "executions list includes tick and the synthesized draw",
    rows.some((r) => r.startsWith("tick")) && rows.some((r) => r.startsWith("draw")),
    JSON.stringify(rows)
  );

  // Resuming play clears the overlay (the runtime's unpaused stub bumps the
  // trace generation; stale inlays over a running game would be lies).
  await playerFrame(page).evaluate(() => document.getElementById("scrub-pause")?.click());
  const resumed = await waitFor(() => page.locator(".cm-live-value").count(), (n) => n === 0, 6000);
  check("resuming clears the live overlay", resumed === 0, `count=${resumed}`);

  // Pause again: the overlay returns, then an edit clears it instantly —
  // stale values must never drift over moved text (hash gate).
  await playerFrame(page).evaluate(() => document.getElementById("scrub-pause")?.click());
  await waitFor(() => page.locator(".cm-live-value").count(), (n) => n > 0);
  await page.evaluate((s) => window.__sandbox.setSource(s), `${GREEN}// paused edit\n`);
  const cleared = await waitFor(() => page.locator(".cm-live-value").count(), (n) => n === 0, 4000);
  check("editing clears the live overlay (hash gate)", cleared === 0, `count=${cleared}`);

  await page.close();
}

// --- 12d. The execution-recency gutter. ----------------------------------------
// A parity-conditional program makes every gutter state deterministic: the
// even/odd arms alternate per frame, and a never-true branch stays dark.
// Pausing shows green (ran this frame) vs cyan (ran a frame before); scrubbing
// BACK one frame swaps the arms' colors and turns pink on (ran after).
{
  // A frame-counter threshold: the EARLY arm runs on frames n<60, the LATE
  // arm after; `never` requires hp < 0 — unreachable (statically runnable →
  // dark). Unique arm texts so line lookup can't collide with init.
  const PARITY = `let init = { n: 0.0, hp: 1.0 }
let tick = (model, dt: Float, tts: Float) =>
  match model.hp < 0.0 with
  | true => { n: model.n, hp: 0.0 }
  | false =>
    match model.n < 60.0 with
    | true => { n: model.n + 1.0, hp: 1.0 }
    | false => { n: model.n + 1.0, hp: 2.0 }
let draw = (model, tts: Float) =>
  Frame.create(
    Camera.lookAt(0.0, 0.0, -6.0, 0.0, 0.0, 0.0),
    Scene.sphere() |> Scene.emissive(Color.rgb(0.1, 1.0, 0.2)))
`;
  const b64u = Buffer.from(PARITY).toString("base64url");
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  await page.goto(`${BASE}/sandbox.html#src=${b64u}`);
  await page.waitForFunction(
    () => window.__sandbox && window.__sandbox.status().state === "live",
    { timeout: 30000 }
  );
  await page.waitForFunction(() => window.__lang && window.__lang.ready, { timeout: 15000 });

  const waitFor = async (fn, pred, timeout = 10000) => {
    const t0 = Date.now();
    for (;;) {
      const v = await fn();
      if (pred(v)) return v;
      if (Date.now() - t0 > timeout) return v;
      await sleep(150);
    }
  };
  const lineOf = (needle) => PARITY.slice(0, PARITY.indexOf(needle)).split("\n").length;
  const earlyLine = lineOf("{ n: model.n + 1.0, hp: 1.0 }"); // frames n<60
  const lateLine = lineOf("{ n: model.n + 1.0, hp: 2.0 }"); // frames n>=60
  const neverLine = lineOf("{ n: model.n, hp: 0.0 }");

  // Run past the threshold, then pause: the late arm is CURRENT (green),
  // the early arm history (cyan), the unreachable arm dark.
  const player = playerFrame(page);
  await player.waitForFunction(
    () => window.__scrub && window.__scrub.range().length === 2 && window.__scrub.range()[1] > 80,
    { timeout: 30000 }
  );
  await player.evaluate(() => document.getElementById("scrub-pause")?.click());
  const cov = await waitFor(
    () => page.evaluate(() => window.__lang.coverage()),
    (c) => c[lateLine] === "now"
  );
  check("current arm is green", cov[lateLine] === "now", JSON.stringify(cov));
  check("pre-threshold arm is cyan (ran before)", cov[earlyLine] === "before", JSON.stringify(cov));
  check("never-taken branch is dark", cov[neverLine] === "dark", JSON.stringify(cov));
  // Gutter markers are real DOM (the viewport shows them all here).
  const domStates = await page.evaluate(() =>
    [...document.querySelectorAll(".cm-cov")].map((el) => el.className)
  );
  check(
    "gutter renders now/before/dark markers",
    ["now", "before", "dark"].every((s) => domStates.some((c) => c.includes(`cm-cov-${s}`))),
    JSON.stringify(domStates.slice(0, 6))
  );

  // Scrub back BEFORE the threshold (frame 10): the early arm becomes this
  // frame's (green — its coverage comes from the ring, the scrubbed-frame
  // path) and the late arm ran only in frames AFTER the paused one → pink.
  await player.evaluate(() => window.__scrub.seek(10));
  const scrubbed = await waitFor(
    () => page.evaluate(() => window.__lang.coverage()),
    (c) => c[earlyLine] === "now"
  );
  check(
    "scrubbed back: the early arm is green from the ring",
    scrubbed[earlyLine] === "now",
    JSON.stringify(scrubbed)
  );
  check(
    "scrubbed back: the post-threshold arm is pink (ran after)",
    scrubbed[lateLine] === "after",
    JSON.stringify(scrubbed)
  );

  await page.close();
}

// --- 13. Scope-aware autocomplete in the editor (commit 8b). ------------------
// The completion source is backed by the wasm's scope-aware `complete`, driven
// through the __sandbox.triggerComplete seam (insert text + set cursor + open
// the popup). That seam is guarded to NOT push, so the status pill stays live
// throughout.
{
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  await page.goto(`${BASE}/sandbox.html`);
  await page.waitForFunction(
    () => window.__sandbox && window.__sandbox.status().state === "live",
    { timeout: 30000 }
  );

  const langAvailable = await page.evaluate(() => window.__lang && window.__lang.ready);
  if (!langAvailable) {
    check("autocomplete: language analysis is ready", false, "__lang not ready");
    await page.close();
  } else {
    // NO cache priming: the lint heartbeat's analyze pass primes the
    // completion cache on its own — dot-completion on a mid-edit (broken)
    // buffer must answer from the last clean analyze, same as real typing.

    // Open the popup and wait until its option labels satisfy `pred`; retries
    // the trigger — under load a popup open can be swallowed by a lagging
    // transaction (a lint pass landing mid-open), so a single-shot wait flakes.
    const openCompletion = async (source, cursor, pred) => {
      for (let attempt = 0; attempt < 4; attempt++) {
        await page.evaluate(
          ({ s, c }) => window.__sandbox.triggerComplete(s, c),
          { s: source, c: cursor }
        );
        const t0 = Date.now();
        while (Date.now() - t0 < 2500) {
          const labels = await page.evaluate(() =>
            [...document.querySelectorAll(".cm-tooltip-autocomplete .cm-completionLabel")].map(
              (el) => el.textContent
            )
          );
          if (pred(labels)) return labels;
          await sleep(150);
        }
      }
      return [];
    };

    // A) Member popup: cursor right after `Scene.` (empty partial) surfaces many
    // members. `triggerComplete` is guarded (no push), so status stays live.
    const memberCursor = GREEN.indexOf("Scene.") + "Scene.".length;
    const opts = await openCompletion(
      GREEN,
      memberCursor,
      (labels) => labels.length > 3 && labels.includes("sphere")
    );
    check(
      "Scene. opens the completion popup with >3 members",
      opts.length > 3,
      `options=${JSON.stringify(opts.slice(0, 8))}`
    );
    check(
      "completion offers a known Scene member (sphere)",
      opts.includes("sphere"),
      JSON.stringify(opts.slice(0, 8))
    );

    // B) Applying a completion inserts its label: a typo'd member `spher` offers
    // the sole `sphere`; accepting it fixes the program (still valid → the push
    // keeps the loop live), and the label is now in the doc.
    const GREEN_TYPO = GREEN.replace("Scene.sphere()", "Scene.spher()");
    const typoCursor = GREEN_TYPO.indexOf("Scene.spher") + "Scene.spher".length;
    const typoOpts = await openCompletion(
      GREEN_TYPO,
      typoCursor,
      (labels) => labels[0] === "sphere"
    );
    // Accept via the editor's own apply path (deterministic — no key focus).
    const accepted = await page.evaluate(() => window.__sandbox.acceptCompletion());
    await sleep(150);
    const afterAccept = await page.evaluate(() => window.__sandbox.getSource());
    check(
      "applying a completion inserts its label",
      afterAccept.includes("Scene.sphere()"),
      `accepted=${accepted}, popup=${JSON.stringify(typoOpts)}, line=${JSON.stringify(
        afterAccept.split("\n").find((l) => l.includes("Scene.")) || afterAccept.slice(0, 60)
      )}`
    );
    // The accept pushed the fixed (valid) program. Wait for the push RESULT
    // (not just state === "live": the pill is already live before the debounced
    // push fires, so that would pass early and the final live check below could
    // catch the transient "reloading").
    await page.waitForFunction(
      () => window.__sandbox.status().message.includes("model preserved"),
      { timeout: 8000 }
    );

    // C) Top-level partial `le` → the `let` keyword (guarded — no push).
    const topOpts = await openCompletion("le", 2, (labels) => labels.includes("let"));
    check(
      "top-level `le` offers the `let` keyword",
      topOpts.includes("let"),
      JSON.stringify(topOpts.slice(0, 8))
    );

    check(
      "autocomplete keeps the sandbox live",
      (await page.evaluate(() => window.__sandbox.status().state)) === "live"
    );

    await page.close();
  }
}

await browser.close();
server.kill();
console.log(failures === 0 ? "\nALL CHECKS PASSED" : `\n${failures} CHECK(S) FAILED`);
process.exit(failures === 0 ? 0 : 1);
