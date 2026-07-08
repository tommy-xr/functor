// wasm smoke test: every MLE sample must LOAD → UPDATE (a pushed reload) → run
// several frames in the browser wasm runtime WITHOUT a persistent error.
//
// This is the guard for the whole class of "works native, broken in wasm" bugs
// — a missing sibling module (hello-cubes's `Pieces.*`), a stale interpreter
// dropping record fields, a contract regression — that a native-only test can't
// see. Each sample:
//
//   1. serves with the built CLI (`functor run wasm`) and loads in headless
//      Chromium (asserts the game reached "[mle] loaded");
//   2. runs a few frames, then PUSHES its own entry source back through the
//      `mle-set-source` seam (the VSCode live-preview / hot-reload path) and
//      asserts the reload succeeds — for a multi-file game this re-links the
//      siblings kept from the initial fetch;
//   3. runs a few more frames and asserts the red DRAW-error overlay is NOT
//      showing (a persistent broken `draw` — the blank-screen symptom — leaves
//      it up; a benign first-frame transient auto-clears and passes).
//
// A visible overlay or a failed load/push fails that sample. Per-frame `[mle]`
// error console lines are printed as info (they may be benign transients) but
// don't by themselves fail the run.
//
// Run manually (owns its own server on :8080, one sample at a time — not part
// of `playwright test`):
//
//   npm run build:cli        # once, so target/debug/functor embeds the runtime
//   npm run fetch:assets     # glTF/textures some samples load (optional)
//   node e2e/wasm-smoke.mjs  # all samples, or: node e2e/wasm-smoke.mjs lighting hello-cubes
import { spawn } from "node:child_process";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const ROOT = fileURLToPath(new URL("..", import.meta.url));
const BASE = "http://127.0.0.1:8080";

// Samples that stand alone (no companion network server). The multiplayer /
// websocket demos need a running server to do anything, so they're out of a
// pure client smoke.
const EXCLUDE = new Set([
  "mpclient",
  "mpserver",
  "wsdemo",
  "wsserverdemo",
]);

// The sample list: CLI args if given, else every examples/* with a functor.json
// declaring `"language": "mle"`, minus the network demos.
const allSamples = readdirSync(`${ROOT}/examples`, { withFileTypes: true })
  .filter((d) => d.isDirectory())
  .map((d) => d.name)
  .filter((name) => {
    const cfg = `${ROOT}/examples/${name}/functor.json`;
    return (
      existsSync(cfg) &&
      JSON.parse(readFileSync(cfg, "utf8")).language === "mle" &&
      !EXCLUDE.has(name)
    );
  })
  .sort();
const samples = process.argv.slice(2).length ? process.argv.slice(2) : allSamples;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// Read the sample's entry source, so we can push it back through the reload seam
// (the "update" step) exactly as an editor would.
function entrySource(sample) {
  const cfg = JSON.parse(
    readFileSync(`${ROOT}/examples/${sample}/functor.json`, "utf8"),
  );
  const entry = cfg.entry || "game.mle";
  return readFileSync(`${ROOT}/examples/${sample}/${entry}`, "utf8");
}

async function smoke(sample) {
  const server = spawn(
    "./target/debug/functor",
    ["-d", `examples/${sample}`, "run", "wasm", "--no-open"],
    { cwd: ROOT, stdio: "ignore" },
  );
  // Software WebGL2 (swiftshader) so the runtime's GL context comes up on any
  // runner — headless CI included — without a real GPU. Same flags as the wasm
  // golden config; a smoke run compares no pixels, so software rendering is fine.
  const browser = await chromium.launch({
    args: [
      "--use-gl=angle",
      "--use-angle=swiftshader",
      "--enable-unsafe-swiftshader",
      "--ignore-gpu-blocklist",
    ],
  });
  const mleErrors = [];
  try {
    const page = await browser.newPage({ viewport: { width: 640, height: 480 } });
    const log = [];
    page.on("console", (m) => {
      const line = `${m.type()}: ${m.text()}`;
      log.push(line);
      if (/\[mle\].*error/.test(m.text())) mleErrors.push(m.text());
    });
    page.on("pageerror", (e) => log.push(`pageerror: ${e}`));

    // Wait for the dev server, then the game to load.
    for (let i = 0; i < 120; i++) {
      try {
        await page.goto(BASE);
        break;
      } catch {
        await sleep(500);
      }
    }
    for (let i = 0; !log.some((m) => m.includes("[mle] loaded")); i++) {
      if (i > 60) {
        return { sample, ok: false, reason: "never loaded", mleErrors, log };
      }
      await sleep(250);
    }

    // Run a few frames.
    await sleep(1500);

    // UPDATE: push the entry source back through the reload seam and await its
    // result (the VSCode hot-reload path). Multi-file games re-link the siblings
    // kept from the initial fetch.
    await page.evaluate(() => {
      window.__smokeResults = [];
      window.addEventListener("message", (e) => {
        if (e.data && e.data.type === "mle-set-source-result")
          window.__smokeResults.push(e.data);
      });
    });
    const src = entrySource(sample);
    const before = await page.evaluate(() => window.__smokeResults.length);
    await page.evaluate(
      (s) => window.postMessage({ type: "mle-set-source", source: s }, "*"),
      src,
    );
    await page
      .waitForFunction((n) => window.__smokeResults.length > n, before, {
        timeout: 5000,
      })
      .catch(() => {});
    const result = await page.evaluate(
      () => window.__smokeResults[window.__smokeResults.length - 1],
    );
    if (!result || result.ok !== true) {
      return {
        sample,
        ok: false,
        reason: `reload push failed: ${result ? result.error : "no result"}`,
        mleErrors,
        log,
      };
    }

    // Run a few more frames, then read the draw-error overlay state.
    await sleep(1500);
    const overlay = await page.evaluate(() => {
      const el = document.getElementById("mle-error");
      if (!el) return { visible: false, text: "" };
      return {
        visible: getComputedStyle(el).display !== "none",
        text: el.textContent || "",
      };
    });
    if (overlay.visible) {
      return {
        sample,
        ok: false,
        reason: `draw-error overlay visible: ${overlay.text.replace(/\s+/g, " ").trim()}`,
        mleErrors,
        log,
      };
    }

    return { sample, ok: true, mleErrors };
  } finally {
    await browser.close();
    server.kill();
    await sleep(400);
  }
}

let failures = 0;
console.log(`wasm smoke: ${samples.length} sample(s)\n`);
for (const sample of samples) {
  let r;
  try {
    r = await smoke(sample);
  } catch (e) {
    r = { sample, ok: false, reason: `harness error: ${e}`, mleErrors: [] };
  }
  const note = r.mleErrors?.length
    ? ` (${r.mleErrors.length} transient [mle] error line(s))`
    : "";
  if (r.ok) {
    console.log(`PASS  ${sample}${note}`);
  } else {
    failures++;
    console.log(`FAIL  ${sample} — ${r.reason}${note}`);
    for (const e of (r.mleErrors || []).slice(0, 3)) console.log(`        ${e}`);
  }
}

console.log(
  failures === 0
    ? `\nALL ${samples.length} SAMPLES PASSED`
    : `\n${failures} SAMPLE(S) FAILED`,
);
process.exit(failures === 0 ? 0 : 1);
