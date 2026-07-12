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
import { readFileSync } from "node:fs";
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

// An inline program whose model shape matches NO served example (`spin` —
// read in both tick and draw): only a fresh `init` runs it cleanly, so this
// catches the sandbox hot-swapping an inline program onto a foreign model.
const INLINE_SPIN = `let init = { spin: 0.0 }
let tick = (model, dt: Float, tts: Float) => { model with spin: model.spin + dt }
let draw = (model, tts: Float) =>
  Frame.create(
    Camera.lookAt(0.0, 0.0, -6.0, 0.0, 0.0, 0.0),
    Scene.cube() |> Scene.rotateY(Angle.radians(model.spin)) |> Scene.emissive(1.0, 0.2, 0.8))
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
    Camera.lookAt(0.0, 0.0, -6.0, 0.0, 0.0, 0.0),
    Scene.sphere() |> Scene.emissive(0.1, 1.0, 0.2)
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
    /Scene\.emissive\([^)]*\)/,
    "Scene.emissive(0.1, 1.0, 0.2)"
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
  await page.waitForFunction(() => window.__sandbox.status().state === "error", {
    timeout: 5000,
  });
  const status = await page.evaluate(() => window.__sandbox.status());
  check("broken edit surfaces the parse error", /cannot .*:\d+:\d+/.test(status.detail), status.detail);
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
for (const example of ["hero", "primitives", "bounce", "monitor"]) {
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

// --- 7. Docs index page + "try it" into the sandbox. --------------------------
// The docs are now a build-time markdown pipeline (site/docs/*.md → dist/docs/).
// The nav lives in site/docs/manifest.json; slug `index` is the docs root
// (/docs/), every other slug nests (/docs/<slug>/).
const manifest = JSON.parse(readFileSync(`${ROOT}site/docs/manifest.json`, "utf8"));
const docsUrl = (slug) => (slug === "index" ? `${BASE}/docs/` : `${BASE}/docs/${slug}/`);
const docsSlugs = manifest.groups.flatMap((g) => g.entries.map((e) => e.slug));

{
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  await page.goto(`${BASE}/docs/`);
  const highlighted = await page.locator("pre.functor-lang span.tok-k").count();
  const tryButtons = await page.locator("a.try-button").count();
  check("docs page highlights Functor Lang blocks", highlighted > 10, `${highlighted} keyword spans`);
  check("docs page offers try-it buttons", tryButtons >= 4, `${tryButtons} buttons`);

  // Follow the first try-it link in THIS page (target=_blank would detach); the
  // href is relative to /docs/, so resolve it against the page URL.
  const href = await page.locator("a.try-button").first().getAttribute("href");
  await page.goto(new URL(href, `${BASE}/docs/`).href);
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

// --- 7b. Every try-it program across ALL docs pages loads live and clean. ------
// Enumerate every page in the manifest, collect every try-button's #src= across
// all of them, then load each into a fresh sandbox and assert it reaches live
// with no `[functor-lang]` error consoles. This is the regression gate for the
// migrated runnables (and any content commits grow later).
{
  const programs = []; // { fromSlug, b64 }
  for (const slug of docsSlugs) {
    const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
    await page.goto(docsUrl(slug));
    const hrefs = await page.locator("a.try-button").evaluateAll((els) =>
      els.map((el) => el.getAttribute("href"))
    );
    for (const href of hrefs) {
      const frag = new URL(href, docsUrl(slug)).hash; // "#src=<b64>"
      const b64 = frag.replace(/^#src=/, "");
      // Decodable base64url (the sandbox's fragment contract) — fail loud if not.
      let decoded = "";
      try {
        decoded = Buffer.from(b64, "base64url").toString("utf8");
      } catch {
        decoded = "";
      }
      check(`try-button on '${slug}' has a decodable #src=`, decoded.length > 0, href);
      programs.push({ fromSlug: slug, b64 });
    }
    await page.close();
  }
  check("docs try-buttons found across all pages", programs.length >= 4, `${programs.length} programs`);

  for (let i = 0; i < programs.length; i++) {
    const { fromSlug, b64 } = programs[i];
    const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
    const consoleLog = [];
    page.on("console", (m) => consoleLog.push(m.text()));
    await page.goto(`${BASE}/sandbox.html#src=${b64}`);
    const name = `docs try-it #${i + 1} (from '${fromSlug}') loads live and ticks cleanly`;
    try {
      await page.waitForFunction(
        () => window.__sandbox && window.__sandbox.status().state === "live",
        { timeout: 30000 }
      );
      await sleep(600);
      const errors = consoleLog.filter((m) => m.includes("[functor-lang]") && m.includes("error"));
      check(name, errors.length === 0, errors.join("\n"));
    } catch {
      check(name, false, consoleLog.slice(-5).join("\n"));
    }
    await page.close();
  }
}

// --- 7c. The old /docs.html link still lands on the docs index (redirect). -----
{
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  await page.goto(`${BASE}/docs.html`);
  // The stub is a 0-delay meta refresh to docs/; Playwright follows it. Give it a
  // moment, then assert we're on the docs index (its h1 + sidebar are present).
  let landed = false;
  for (let i = 0; i < 40; i++) {
    const h1 = await page.locator(".docs-main h1").count();
    if (h1 > 0 && page.url().includes("/docs/")) {
      landed = true;
      break;
    }
    await sleep(100);
  }
  check("old /docs.html redirects to the docs index", landed, `url=${page.url()}`);
  await page.close();
}

// --- 7c-nav. Getting started renders with the sidebar marking it current. ------
{
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  await page.goto(`${BASE}/docs/getting-started/`);
  await page.waitForFunction(() => !!document.querySelector(".docs-main h1"), { timeout: 10000 });
  const heading = await page.locator(".docs-main h1").first().textContent();
  check("getting-started page renders its heading", /getting started/i.test(heading || ""), heading);
  // The sidebar link for this page carries aria-current="page" (the current mark).
  const current = await page.locator('.docs-nav a[aria-current="page"]');
  const currentText = (await current.count()) ? await current.first().textContent() : "";
  const currentHref = (await current.count()) ? await current.first().getAttribute("href") : "";
  check(
    "sidebar marks getting-started current",
    /getting started/i.test(currentText) && /getting-started/.test(currentHref || ""),
    `text=${currentText} href=${currentHref}`
  );
  await page.close();
}

// --- 7d. Docs search (Pagefind): typing surfaces results linking to docs. ------
// The docs pages load ONE script (the stock Pagefind UI) that fetches the index
// built into dist/pagefind/. Type a term, await async results, and assert a
// result both links under /docs and navigates there when followed.
{
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  await page.goto(`${BASE}/docs/`);

  // The search input mounts once pagefind-ui.js loads and PagefindUI inits.
  const input = page.locator(".pagefind-ui__search-input");
  await input.waitFor({ state: "visible", timeout: 15000 });
  check("docs search UI mounts in the sidebar", true);

  await input.fill("pipeline");
  // Results are async (fetch + wasm decode); poll for at least one result card.
  const results = page.locator(".pagefind-ui__result");
  try {
    await results.first().waitFor({ state: "visible", timeout: 15000 });
  } catch {
    // fall through — the count check reports the failure with detail
  }
  const count = await results.count();
  check("docs search 'pipeline' returns >=1 result", count >= 1, `${count} results`);

  const href = await page.locator(".pagefind-ui__result-link").first().getAttribute("href");
  const underDocs = !!href && new URL(href, `${BASE}/docs/`).pathname.startsWith("/docs");
  check("first search result links to a docs page", underDocs, `href=${href}`);

  // Following the result navigates to a docs page (its .docs-main renders).
  await page.goto(new URL(href, `${BASE}/docs/`).href);
  await page.waitForFunction(() => !!document.querySelector(".docs-main"), { timeout: 10000 });
  check(
    "following a search result lands on a docs page",
    new URL(page.url()).pathname.startsWith("/docs"),
    `url=${page.url()}`
  );

  await page.close();
}

// --- 7e. The language reference renders its major sections and is indexed. ------
// The full language guide is a long, section-heavy page; assert it renders a
// healthy number of h2 sections and that Pagefind indexes its distinctive
// content (a search for a term unique to this page returns a docs result).
{
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  await page.goto(`${BASE}/docs/language/`);
  await page.waitForFunction(() => !!document.querySelector(".docs-main h1"), { timeout: 10000 });
  const h2s = await page.locator(".docs-main h2").count();
  check("language reference renders its major sections", h2s >= 12, `${h2s} h2 sections`);

  // Pagefind indexes the new content: a term unique to the language guide
  // ("polymorphism", from the generics section) returns a docs result.
  const input = page.locator(".pagefind-ui__search-input");
  await input.waitFor({ state: "visible", timeout: 15000 });
  await input.fill("polymorphism");
  const results = page.locator(".pagefind-ui__result");
  try {
    await results.first().waitFor({ state: "visible", timeout: 15000 });
  } catch {
    // fall through — the count check reports the failure with detail
  }
  const count = await results.count();
  check("docs search 'polymorphism' returns >=1 result", count >= 1, `${count} results`);
  const href = await page.locator(".pagefind-ui__result-link").first().getAttribute("href");
  const underDocs = !!href && new URL(href, `${BASE}/docs/`).pathname.startsWith("/docs");
  check("'polymorphism' result links to a docs page", underDocs, `href=${href}`);

  await page.close();
}

// --- 7f. Time-travel guide renders, is marked current, and is indexed. ---------
// The new time-travel & hot-reload page renders with the sidebar marking it
// current, and Pagefind indexes a term distinctive to it ("reload boundary")
// returning a result that links to /docs/time-travel/.
{
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  await page.goto(`${BASE}/docs/time-travel/`);
  await page.waitForFunction(() => !!document.querySelector(".docs-main h1"), { timeout: 10000 });
  const heading = await page.locator(".docs-main h1").first().textContent();
  check("time-travel page renders its heading", /time travel/i.test(heading || ""), heading);

  // The sidebar link for this page carries aria-current="page".
  const current = await page.locator('.docs-nav a[aria-current="page"]');
  const currentText = (await current.count()) ? await current.first().textContent() : "";
  const currentHref = (await current.count()) ? await current.first().getAttribute("href") : "";
  check(
    "sidebar marks time-travel current",
    /time travel/i.test(currentText) && /time-travel/.test(currentHref || ""),
    `text=${currentText} href=${currentHref}`
  );

  // Pagefind finds a term distinctive to this page and links to it.
  const input = page.locator(".pagefind-ui__search-input");
  await input.waitFor({ state: "visible", timeout: 15000 });
  await input.fill("reload boundary");
  const results = page.locator(".pagefind-ui__result");
  try {
    await results.first().waitFor({ state: "visible", timeout: 15000 });
  } catch {
    // fall through — the count check reports the failure with detail
  }
  const count = await results.count();
  check("docs search 'reload boundary' returns >=1 result", count >= 1, `${count} results`);
  const href = await page.locator(".pagefind-ui__result-link").first().getAttribute("href");
  const toTimeTravel =
    !!href && new URL(href, `${BASE}/docs/`).pathname.startsWith("/docs/time-travel");
  check("'reload boundary' result links to the time-travel page", toTimeTravel, `href=${href}`);

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
  await page.goto(`${BASE}/sandbox.html`);
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

  await player.waitForFunction(() => window.__scrub.range().length === 2, {
    timeout: 10000,
  });

  // The recorded range grows while running.
  const r0 = await player.evaluate(() => window.__scrub.range());
  await sleep(500);
  const r1 = await player.evaluate(() => window.__scrub.range());
  check("scrubber range grows while running", r1[1] > r0[1], `${r0} -> ${r1}`);

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
  check(
    "step advances the frame by exactly 1 while paused",
    after === before + 1,
    `${before} -> ${after}`
  );

  // Resume: frames advance again.
  await player.evaluate(() => window.__scrub.togglePause());
  await player.waitForFunction(() => !window.__scrub.paused(), { timeout: 3000 });
  const rf0 = await player.evaluate(() => window.__scrub.frame());
  await sleep(400);
  const rf1 = await player.evaluate(() => window.__scrub.frame());
  check("resume advances frames again", rf1 > rf0, `${rf0} -> ${rf1}`);

  await page.close();
}

// --- 10. The editor language-intelligence wasm analyzes source in-browser. -----
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
      return null; // pkg not built — degraded config, skip below
    }
    await mod.default(); // init the wasm
    // A type error: adding a string to a float.
    const bad = JSON.parse(mod.functor_lang_analyze('let bad = 1.0 + "x"\n'));
    // A clean program using prelude names.
    const clean = JSON.parse(
      mod.functor_lang_analyze(
        "let draw = (model, tts: Float) =>\n" +
          "  Frame.create(Camera.lookAt(0.0, 0.0, -6.0, 0.0, 0.0, 0.0), Scene.cube())\n"
      )
    );
    return { bad, clean };
  });
  if (!result) {
    console.log("SKIP: language wasm analyzes source in-browser — pkg not built");
    await page.close();
    // Nothing further to assert in the degraded config.
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

  // Await the analysis wasm's readiness (resolves to false when the pkg is
  // absent — then the whole lint block is skipped so the suite stays green in
  // both configurations).
  const langAvailable = await page.evaluate(
    () => window.__lang && window.__lang.ready
  );
  if (!langAvailable) {
    console.log("SKIP: live diagnostics — language analysis pkg not built (__lang not ready)");
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
// SKIPs (like the lint block) when the analysis pkg isn't built.
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
    console.log("SKIP: hover/inlay/codelens — language analysis pkg not built (__lang not ready)");
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

// --- 13. Scope-aware autocomplete in the editor (commit 8b). ------------------
// The completion source is backed by the wasm's scope-aware `complete`, driven
// through the __sandbox.triggerComplete seam (insert text + set cursor + open
// the popup). That seam is guarded to NOT push, so the status pill stays live
// throughout. SKIPs (like the lint/hover blocks) when the analysis pkg is absent.
{
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  await page.goto(`${BASE}/sandbox.html`);
  await page.waitForFunction(
    () => window.__sandbox && window.__sandbox.status().state === "live",
    { timeout: 30000 }
  );

  const langAvailable = await page.evaluate(() => window.__lang && window.__lang.ready);
  if (!langAvailable) {
    console.log("SKIP: autocomplete — language analysis pkg not built (__lang not ready)");
    await page.close();
  } else {
    // Prime the wasm last-good cache with a valid program (via the __lang seam —
    // no doc change, no push), so dot-completion works even while the live
    // buffer is mid-edit.
    await page.evaluate((s) => window.__lang.complete(s, 0), GREEN);

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
