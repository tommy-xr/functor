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
import net from "node:net";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const ROOT = fileURLToPath(new URL("..", import.meta.url));
const BASE = "http://127.0.0.1:8080";
const PORT = 8080;

// Is :PORT accepting connections? Used to serialize the per-sample servers: a
// lingering dev server from a previous sample would make the next one connect to
// the WRONG project (the stale-:8080 hazard that produces phantom errors), so we
// wait for the port to be FREE before spawning and after killing.
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

// Samples that stand alone (no companion network server). The multiplayer /
// websocket demos need a running server to do anything, so they're out of a
// pure client smoke.
const EXCLUDE = new Set([
  "mpclient",
  "mpserver",
  "wsdemo",
  "wsserverdemo",
]);

// A sample "needs assets" if its directory carries fetched binary assets
// (glTF/textures/audio via `npm run fetch:assets`, gitignored). When those
// aren't present — the default on CI, which mirrors golden.yml's asset-free
// stance — such a sample would 404 its assets, so we SKIP it (loudly) rather
// than fail. Local runs (assets fetched) still cover it. NOTE: a missing asset
// currently PANICS the wasm runtime instead of falling back to the empty asset
// (docs/mle.md says it should degrade) — tracked as a follow-up; until then the
// skip keeps this CI check meaningful for the asset-free samples.
function needsMissingAssets(sample) {
  const dir = `${ROOT}/examples/${sample}`;
  const refsAssets = /\.(glb|png|jpg|wav)\b/i.test(
    readFileSync(`${dir}/game.mle`, "utf8"),
  );
  const hasAssets = readdirSync(dir).some((f) =>
    /\.(glb|png|jpg|wav)$/i.test(f),
  );
  return refsAssets && !hasAssets;
}

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
  // A previous sample's dev server must have fully released :8080, or this
  // sample would connect to the wrong project (stale-server false results).
  if (!(await waitPortFree(15000))) {
    return {
      sample,
      ok: false,
      reason: `:${PORT} still in use by a previous server — cannot serve this sample cleanly`,
      mleErrors: [],
    };
  }
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

    // Guard against a stale server: the entry the page fetched must match this
    // sample's entry on disk. A mismatch means we connected to a leftover dev
    // server for a different project — fail loud instead of reporting phantom
    // errors from the wrong game.
    const served = await page.evaluate(async () => {
      const path = window.__mleGamePath;
      const res = await fetch(path, { cache: "no-store" });
      return res.text();
    });
    if (served.trim() !== entrySource(sample).trim()) {
      return {
        sample,
        ok: false,
        reason: "served entry source does not match this sample (stale/wrong dev server?)",
        mleErrors,
        log,
      };
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
    // The runtime QUEUES the pushed source and applies it at the top of a frame
    // (never mid-frame), then posts `mle-set-source-result`. On a heavy sample
    // under software GL a frame can take several seconds, so wait generously.
    const before = await page.evaluate(() => window.__smokeResults.length);
    const pushStart = Date.now();
    await page.evaluate(
      (s) => window.postMessage({ type: "mle-set-source", source: s }, "*"),
      src,
    );
    await page
      .waitForFunction((n) => window.__smokeResults.length > n, before, {
        timeout: 30000,
      })
      .catch(() => {});
    const result = await page.evaluate(
      () => window.__smokeResults[window.__smokeResults.length - 1],
    );
    if (!result || result.ok !== true) {
      const waited = ((Date.now() - pushStart) / 1000).toFixed(1);
      // Did the runtime log that it applied the reload? (Distinguishes a frame
      // loop that never got to it from a result that wasn't delivered.)
      const applied = log.filter((m) => /\[mle\] (reloaded|reload error)/.test(m));
      const tail = log.slice(-8).map((m) => m.replace(/\s+/g, " ").slice(0, 100));
      return {
        sample,
        ok: false,
        reason:
          `reload push failed after ${waited}s: ${result ? result.message : "no mle-set-source-result received"}` +
          ` | runtime applied-reload log: ${applied.length ? applied.join(" ; ") : "NONE"}` +
          ` | last console: ${tail.join(" | ")}`,
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
    server.kill("SIGKILL");
    // Wait for :PORT to actually release before the next sample spawns, so the
    // next one can't connect to this server.
    await waitPortFree(10000);
  }
}

let failures = 0;
console.log(`wasm smoke: ${samples.length} sample(s)\n`);
let skipped = 0;
for (const sample of samples) {
  if (needsMissingAssets(sample)) {
    skipped++;
    console.log(`SKIP  ${sample} — references assets not present (run npm run fetch:assets to include it)`);
    continue;
  }
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

const ran = samples.length - skipped;
const skipNote = skipped ? ` (${skipped} skipped — assets not fetched)` : "";
console.log(
  failures === 0
    ? `\nALL ${ran} SAMPLES PASSED${skipNote}`
    : `\n${failures} SAMPLE(S) FAILED${skipNote}`,
);
process.exit(failures === 0 ? 0 : 1);
