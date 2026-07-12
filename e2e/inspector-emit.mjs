// Inspector-emit E2E (visual-debugger PR2b): the wasm runtime must EMIT the
// paused-scene inspector trace over postMessage, so the VS Code live-preview
// auto-connects the inspector with no explicit gesture.
//
// Modeled on e2e/wasm-smoke.mjs (same server-spawn + headless Chromium). It:
//
//   1. serves `examples/inspector` with the built CLI (`functor run wasm`) and
//      loads it in headless Chromium (asserts "[functor-lang] loaded");
//   2. installs a `window` message listener capturing `functor-inspector-trace`
//      messages (the page posts to `window.parent`, which resolves to the page
//      itself when loaded top-level — so a top-level listener captures them);
//   3. runs a few frames, then PAUSES via the real DOM scrubber pause button
//      (`#scrub-pause`), falling back to the `functor_lang_scrub_toggle_pause`
//      export if the button isn't hit;
//   4. asserts a `functor-inspector-trace` arrived with `paused: true` and at
//      least one invocation whose bindings carry real (string) values.
//
// Run manually (owns its own server on :8080):
//
//   npm run build:cli            # so target/debug/functor embeds the runtime
//   node e2e/inspector-emit.mjs
import { spawn } from "node:child_process";
import net from "node:net";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const ROOT = fileURLToPath(new URL("..", import.meta.url));
const BASE = "http://127.0.0.1:8080";
const PORT = 8080;
const SAMPLE = "inspector";

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// Serialize against a lingering dev server on :PORT (see wasm-smoke.mjs).
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

async function run() {
  if (!(await waitPortFree(15000))) {
    return { ok: false, reason: `:${PORT} still in use by a previous server` };
  }
  const server = spawn(
    "./target/debug/functor",
    ["-d", `examples/${SAMPLE}`, "run", "wasm", "--no-open"],
    { cwd: ROOT, stdio: "ignore" },
  );
  const browser = await chromium.launch({
    args: [
      "--use-gl=angle",
      "--use-angle=swiftshader",
      "--enable-unsafe-swiftshader",
      "--ignore-gpu-blocklist",
    ],
  });
  try {
    const page = await browser.newPage({ viewport: { width: 640, height: 480 } });
    const log = [];
    page.on("console", (m) => log.push(`${m.type()}: ${m.text()}`));
    page.on("pageerror", (e) => log.push(`pageerror: ${e}`));

    // Wait for the dev server to come up, then the game to load.
    for (let i = 0; i < 120; i++) {
      try {
        await page.goto(BASE);
        break;
      } catch {
        await sleep(500);
      }
    }
    for (let i = 0; !log.some((m) => m.includes("[functor-lang] loaded")); i++) {
      if (i > 60) return { ok: false, reason: "never loaded", log };
      await sleep(250);
    }

    // Capture every `functor-inspector-trace` the runtime emits (the page posts
    // to window.parent; top-level, that is this same window).
    await page.evaluate(() => {
      window.__traces = [];
      window.addEventListener("message", (e) => {
        if (e.data && e.data.type === "functor-inspector-trace")
          window.__traces.push(e.data.trace);
      });
    });

    // Run a few frames of real play (crosses at least one tick).
    await sleep(1800);

    // PAUSE via the real DOM scrubber button; fall back to the export if the
    // click somehow didn't land.
    const clicked = await page.evaluate(() => {
      const btn = document.getElementById("scrub-pause");
      if (btn) {
        btn.click();
        return true;
      }
      return false;
    });
    if (!clicked) {
      await page.evaluate(() => window.functor_lang_scrub_toggle_pause?.());
    }

    // Let the poll loop observe the pause, build the paused trace, and post it.
    await sleep(1200);

    const traces = await page.evaluate(() => window.__traces);
    const paused = (traces || []).filter((t) => t && t.paused === true);
    if (paused.length === 0) {
      return {
        ok: false,
        reason: `no paused functor-inspector-trace received (got ${traces ? traces.length : 0} trace message(s), clickedButton=${clicked})`,
        log: log.slice(-12),
      };
    }

    // The paused doc must replay the last real frame into >=1 invocation whose
    // bindings carry real values (the recorder captured live binding-site data).
    const doc = paused[paused.length - 1];
    const invs = Array.isArray(doc.invocations) ? doc.invocations : [];
    const withRealBindings = invs.find(
      (i) =>
        Array.isArray(i.bindings) &&
        i.bindings.length > 0 &&
        i.bindings.every((b) => typeof b.value === "string" && b.value.length > 0),
    );
    if (!withRealBindings) {
      return {
        ok: false,
        reason: `paused trace has no invocation with real-valued bindings: ${JSON.stringify(doc).slice(0, 400)}`,
      };
    }

    // Sources hash present (the LSP's span/version gate).
    const hashOk =
      Array.isArray(doc.sources) &&
      doc.sources.length > 0 &&
      typeof doc.sources[0].hash === "string" &&
      doc.sources[0].hash.length === 64;

    return {
      ok: hashOk,
      reason: hashOk ? "" : "sources hash missing/malformed",
      summary: {
        pausedMessages: paused.length,
        invocations: invs.length,
        entries: invs.map((i) => i.entry),
        sampleBinding: withRealBindings.bindings[0],
      },
    };
  } finally {
    await browser.close();
    server.kill("SIGKILL");
    await waitPortFree(10000);
  }
}

let r;
try {
  r = await run();
} catch (e) {
  r = { ok: false, reason: `harness error: ${e}` };
}
if (r.ok) {
  console.log("PASS  inspector-emit");
  console.log(`      ${JSON.stringify(r.summary)}`);
} else {
  console.log(`FAIL  inspector-emit — ${r.reason}`);
  if (r.log) for (const line of r.log) console.log(`        ${line}`);
}
process.exit(r.ok ? 0 : 1);
