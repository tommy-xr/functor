// wasm smoke test: every Functor Lang sample must LOAD → UPDATE (a pushed reload) → run
// several frames in the browser wasm runtime WITHOUT a persistent error.
//
// This is the guard for the whole class of "works native, broken in wasm" bugs
// — a missing sibling module (hello-cubes's `Pieces.*`), a stale interpreter
// dropping record fields, a contract regression — that a native-only test can't
// see. Each sample:
//
//   1. serves with the built CLI (`functor run wasm`) and loads in headless
//      Chromium (asserts the game reached "[functor-lang] loaded");
//   2. runs a few frames, then PUSHES its own entry source back through the
//      `functor-lang-set-source` seam (the VSCode live-preview / hot-reload path) and
//      asserts the reload succeeds — for a multi-file game this re-links the
//      siblings kept from the initial fetch;
//   3. runs a few more frames and asserts the red DRAW-error overlay is NOT
//      showing (a persistent broken `draw` — the blank-screen symptom — leaves
//      it up; a benign first-frame transient auto-clears and passes).
//
// A visible overlay or a failed load/push fails that sample. Per-frame `[functor-lang]`
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
// pure client smoke — and `loading` is out because it is the one sample that
// streams from an EXTERNAL CDN (assets.babylonjs.com): a hermetic CI smoke
// must not depend on third-party network. Its wasm remote-loading path is
// covered hermetically by e2e/remote-assets.mjs, and its Sub.assets
// lifecycle by the producer tests + native debug-server verification.
const EXCLUDE = new Set([
  "loading",
  "mp",
  "wsdemo",
  "wsserverdemo",
]);
// A sample that references binary assets (glTF/texture/audio, fetched via
// `npm run fetch:assets` and gitignored) runs fine WITHOUT them: a missing/404
// asset degrades to the empty fallback (matching native — see
// functor_runtime_common::io::load_bytes_async), so the sample still loads,
// reloads, and ticks without a Functor Lang error. So every sample runs on CI even
// asset-free; the missing-asset path is itself worth exercising here.

// The sample list: CLI args if given, else every examples/* with a functor.json
// declaring `"language": "functor-lang"`, minus the network demos.
const allSamples = readdirSync(`${ROOT}/examples`, { withFileTypes: true })
  .filter((d) => d.isDirectory())
  .map((d) => d.name)
  .filter((name) => {
    const cfg = `${ROOT}/examples/${name}/functor.json`;
    return (
      existsSync(cfg) &&
      JSON.parse(readFileSync(cfg, "utf8")).language === "functor-lang" &&
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
  const entry = cfg.entry || "game.fun";
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
      functorLangErrors: [],
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
  const functorLangErrors = [];
  try {
    const page = await browser.newPage({ viewport: { width: 640, height: 480 } });
    const log = [];
    page.on("console", (m) => {
      const line = `${m.type()}: ${m.text()}`;
      log.push(line);
      if (/\[functor-lang\].*error/.test(m.text())) functorLangErrors.push(m.text());
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
    for (let i = 0; !log.some((m) => m.includes("[functor-lang] loaded")); i++) {
      if (i > 60) {
        return { sample, ok: false, reason: "never loaded", functorLangErrors, log };
      }
      await sleep(250);
    }

    // Guard against a stale server: the entry the page fetched must match this
    // sample's entry on disk. A mismatch means we connected to a leftover dev
    // server for a different project — fail loud instead of reporting phantom
    // errors from the wrong game.
    const served = await page.evaluate(async () => {
      const path = window.__functorLangGamePath;
      const res = await fetch(path, { cache: "no-store" });
      return res.text();
    });
    if (served.trim() !== entrySource(sample).trim()) {
      return {
        sample,
        ok: false,
        reason: "served entry source does not match this sample (stale/wrong dev server?)",
        functorLangErrors,
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
        if (e.data && e.data.type === "functor-lang-set-source-result")
          window.__smokeResults.push(e.data);
      });
    });
    const src = entrySource(sample);
    // The runtime QUEUES the pushed source and applies it at the top of a frame
    // (never mid-frame), then posts `functor-lang-set-source-result`. On a heavy sample
    // under software GL a frame can take several seconds, so wait generously.
    const before = await page.evaluate(() => window.__smokeResults.length);
    const pushStart = Date.now();
    await page.evaluate(
      (s) => window.postMessage({ type: "functor-lang-set-source", source: s }, "*"),
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
      const applied = log.filter((m) => /\[functor-lang\] (reloaded|reload error)/.test(m));
      const tail = log.slice(-8).map((m) => m.replace(/\s+/g, " ").slice(0, 100));
      return {
        sample,
        ok: false,
        reason:
          `reload push failed after ${waited}s: ${result ? result.message : "no functor-lang-set-source-result received"}` +
          ` | runtime applied-reload log: ${applied.length ? applied.join(" ; ") : "NONE"}` +
          ` | last console: ${tail.join(" | ")}`,
        functorLangErrors,
        log,
      };
    }

    // Run a few more frames, then read the draw-error overlay state.
    await sleep(1500);
    const overlay = await page.evaluate(() => {
      const el = document.getElementById("functor-lang-error");
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
        functorLangErrors,
        log,
      };
    }

    return { sample, ok: true, functorLangErrors };
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
for (const sample of samples) {
  let r;
  try {
    r = await smoke(sample);
  } catch (e) {
    r = { sample, ok: false, reason: `harness error: ${e}`, functorLangErrors: [] };
  }
  const note = r.functorLangErrors?.length
    ? ` (${r.functorLangErrors.length} transient [functor-lang] error line(s))`
    : "";
  if (r.ok) {
    console.log(`PASS  ${sample}${note}`);
  } else {
    failures++;
    console.log(`FAIL  ${sample} — ${r.reason}${note}`);
    for (const e of (r.functorLangErrors || []).slice(0, 3)) console.log(`        ${e}`);
  }
}

console.log(
  failures === 0
    ? `\nALL ${samples.length} SAMPLES PASSED`
    : `\n${failures} SAMPLE(S) FAILED`,
);
process.exit(failures === 0 ? 0 : 1);
