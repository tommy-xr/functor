// wasm remote-asset e2e: prove that a URL asset (`Scene.model(Asset.model("http://…"))`)
// actually loads over the network in a real browser wasm runtime — the
// browser-side twin of runtime/functor-runtime-desktop/tests/remote_assets.rs.
//
// The wasm asset path (functor_runtime_common::io::load_bytes_async, wasm arm)
// hands any URL straight to `fetch()` and turns a non-OK HTTP status into an
// Err, so a 404 routes to the asset system's fallback instead of feeding the
// 404 page's HTML to the glTF parser. That code needed ZERO changes for remote
// assets; this test is the missing proof that it works end-to-end in a real
// browser: URL passthrough, cross-origin CORS, the fetch firing, and a 404
// degrading gracefully rather than hanging or crashing.
//
// What it does, hermetically (no external network, no shared ports):
//   1. Generates the same minimal valid .glb the native test uses.
//   2. Serves it from a permissive-CORS localhost server (a stand-in CDN) on an
//      EPHEMERAL port: /model.glb → 200 glb, everything else → 404. Both carry
//      `Access-Control-Allow-Origin: *`, so the game page (a DIFFERENT origin)
//      can fetch cross-origin — exactly the real CDN scenario.
//   3. Writes a temp Functor Lang project whose `draw` loads BOTH a good URL and a
//      404 URL from that server, and `functor build wasm`s it into a
//      self-contained static bundle.
//   4. Serves that bundle from its own ephemeral-port static server and loads
//      it in headless Chromium (swiftshader WebGL2, like e2e/wasm-smoke.mjs),
//      then OBSERVES THE ACTUAL fetches via Playwright's network events.
//
// Building + serving the static bundle ourselves (rather than `functor run
// wasm`) keeps the test off the CLI dev server's hardcoded :8080 — so it is
// hermetic and parallel-safe, and never collides with a stray :8080 holder.
//
// Assertions:
//   - the browser issued a GET to the good URL and got 200 (URL passthrough +
//     CORS + the fetch really fired inside the wasm runtime);
//   - the browser issued a GET to the 404 URL and got 404;
//   - (hermetic case) a "soft 404" — HTTP 200 whose body is an HTML error
//     page — FAILS the load: the magic-byte guard refuses to feed HTML to the
//     glTF parser (wasm parity with the native guard);
//   - each failed asset shows up as a console.error — the web runtime's
//     RuntimeEvent sink (a failed asset used to fall back to `eprintln!`,
//     which goes nowhere in a browser: totally invisible);
//   - the draw-error overlay is NOT showing afterward — failures degraded to
//     the fallback asset; nothing hung the loop or panicked the glTF pipeline.
//
// Network observation (Playwright response events) remains the ground truth
// for "the fetch really happened"; the console signal is the ground truth for
// "and a human/agent can SEE what failed". This is intentionally an
// end-to-end NETWORK test, not a glTF-decode test: decoding of local assets
// is already covered by the wasm golden suite.
//
// Optional live-CDN case: with FUNCTOR_REMOTE_E2E_NETWORK=1, also loads a real
// asset from https://assets.babylonjs.com. OFF by default (needs the internet
// and a CORS-enabled CDN) — the hermetic case is the one that gates.
//
// Run manually (needs the built CLI + Playwright's Chromium — `npx playwright
// install chromium`, the same one wasm-smoke uses):
//
//   npm run build:cli:debug                 # once, so target/debug/functor embeds the runtime
//   node e2e/remote-assets.mjs              # the hermetic case
//   FUNCTOR_REMOTE_E2E_NETWORK=1 node e2e/remote-assets.mjs   # + the live-CDN case
import { spawnSync } from "node:child_process";
import { createReadStream, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import http from "node:http";
import { tmpdir } from "node:os";
import { extname, join, normalize } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const ROOT = fileURLToPath(new URL("..", import.meta.url));
const FUNCTOR = join(ROOT, "target", "debug", "functor");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// The smallest valid .glb: a 12-byte header + a JSON chunk declaring an empty
// glTF 2.0 asset. The glTF pipeline PARSES it (garbage bytes would not) — the
// exact bytes minimal_glb() builds in the native test.
function minimalGlb() {
  let json = Buffer.from('{"asset":{"version":"2.0"}}', "utf8");
  while (json.length % 4 !== 0) json = Buffer.concat([json, Buffer.from(" ")]);
  const header = Buffer.alloc(12);
  header.write("glTF", 0, "ascii");
  header.writeUInt32LE(2, 4);
  header.writeUInt32LE(12 + 8 + json.length, 8);
  const chunkHeader = Buffer.alloc(8);
  chunkHeader.writeUInt32LE(json.length, 0);
  chunkHeader.write("JSON", 4, "ascii");
  return Buffer.concat([header, chunkHeader, json]);
}

// A permissive-CORS "CDN": /model.glb serves a real glb, /soft404.glb serves
// the classic CDN failure — HTTP 200 with an HTML error page — and everything
// else 404s. Every response carries Access-Control-Allow-Origin: * so the game
// page (a different origin) can fetch cross-origin, and failures are
// CORS-visible too.
function startAssetServer() {
  const glb = minimalGlb();
  const server = http.createServer((req, res) => {
    res.setHeader("Access-Control-Allow-Origin", "*");
    if (req.url === "/model.glb") {
      res.writeHead(200, { "Content-Type": "model/gltf-binary" });
      res.end(glb);
    } else if (req.url === "/soft404.glb") {
      res.writeHead(200, { "Content-Type": "text/html" });
      res.end("<html><body>totally not a model</body></html>");
    } else {
      res.writeHead(404, { "Content-Type": "text/plain" });
      res.end("not found");
    }
  });
  return new Promise((resolve) => {
    server.listen(0, "127.0.0.1", () =>
      resolve({ server, port: server.address().port }),
    );
  });
}

const MIME = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".wasm": "application/wasm",
  ".json": "application/json",
  ".fun": "text/plain; charset=utf-8",
};

// A plain static server for the built dist/web bundle (its own origin, so the
// asset fetches above are genuinely cross-origin).
function startBundleServer(dir) {
  const server = http.createServer((req, res) => {
    const rel = normalize(decodeURIComponent(req.url.split("?")[0])).replace(
      /^(\.\.[/\\])+/,
      "",
    );
    const file = join(dir, rel === "/" || rel === "" ? "index.html" : rel);
    res.setHeader("Content-Type", MIME[extname(file)] || "application/octet-stream");
    res.setHeader("Cache-Control", "no-store");
    const stream = createReadStream(file);
    stream.on("error", () => {
      res.writeHead(404);
      res.end("not found");
    });
    stream.pipe(res);
  });
  return new Promise((resolve) => {
    server.listen(0, "127.0.0.1", () =>
      resolve({ server, port: server.address().port }),
    );
  });
}

// A temp Functor Lang project whose draw loads a good URL, a 404 URL, and (when the
// case provides one) a soft-404 URL, built into a self-contained wasm bundle.
// Each asset fetches once (the asset handle is cached), so the browser makes
// exactly one request per URL.
function buildBundle(goodUrl, badUrl, softUrl) {
  const dir = mkdtempSync(join(tmpdir(), "functor-remote-e2e-"));
  writeFileSync(
    join(dir, "functor.json"),
    JSON.stringify({ language: "functor-lang", entry: "game.fun" }, null, 2),
  );
  const soft = softUrl
    ? `\n      Scene.model(Asset.model("${softUrl}")) |> Scene.translate(Vec3.make(0.0 - 3.0, 0.0, 0.0)),`
    : "";
  writeFileSync(
    join(dir, "game.fun"),
    `// Generated e2e fixture: URL models that resolve, 404, and soft-404.
let init = { frame: 0.0 }

let tick = (model, dt, tts) => { model with frame: model.frame + 1.0 }

let draw = (model, tts) =>
  Frame.create(
    Camera.lookAt(Vec3.make(0.0, 2.0, 0.0 - 6.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.group([
      Scene.model(Asset.model("${goodUrl}")),
      Scene.model(Asset.model("${badUrl}")) |> Scene.translate(Vec3.make(3.0, 0.0, 0.0)),${soft}
    ]))
`,
  );
  const built = spawnSync(FUNCTOR, ["-d", dir, "build", "wasm"], {
    cwd: ROOT,
    encoding: "utf8",
  });
  if (built.status !== 0) {
    throw new Error(`build wasm failed:\n${built.stdout}\n${built.stderr}`);
  }
  return { dir, webDir: join(dir, "dist", "web") };
}

async function runCase({ goodUrl, badUrl, softUrl, label }) {
  const { dir, webDir } = buildBundle(goodUrl, badUrl, softUrl);
  const { server: bundleServer, port: bundlePort } = await startBundleServer(webDir);
  const base = `http://127.0.0.1:${bundlePort}`;
  const browser = await chromium.launch({
    // Software WebGL2 so the runtime's GL context comes up headless without a
    // GPU — the same flags wasm-smoke / the wasm golden config use.
    args: [
      "--use-gl=angle",
      "--use-angle=swiftshader",
      "--enable-unsafe-swiftshader",
      "--ignore-gpu-blocklist",
    ],
  });
  const responses = [];
  try {
    const page = await browser.newPage({ viewport: { width: 640, height: 480 } });
    const log = [];
    page.on("console", (m) => log.push(`${m.type()}: ${m.text()}`));
    page.on("pageerror", (e) => log.push(`pageerror: ${e}`));
    // The ground truth: every response the browser actually received for a URL
    // the game asked for.
    page.on("response", (r) => {
      if (r.url() === goodUrl || r.url() === badUrl || r.url() === softUrl) {
        responses.push({ url: r.url(), status: r.status() });
      }
    });

    await page.goto(base);
    for (let i = 0; !log.some((m) => m.includes("[functor-lang] loaded")); i++) {
      if (i > 80) {
        throw new Error(
          `[${label}] game never loaded. Log tail: ${log.slice(-8).join(" | ")}`,
        );
      }
      await sleep(250);
    }

    // Run frames so draw hydrates the models and the fetches fire; then give
    // the network a moment to settle.
    await sleep(3000);

    const good = responses.find((r) => r.url === goodUrl);
    const bad = responses.find((r) => r.url === badUrl);

    if (!good) {
      throw new Error(
        `[${label}] browser never fetched the good URL ${goodUrl} — URL passthrough or CORS failed`,
      );
    }
    if (good.status !== 200) {
      throw new Error(`[${label}] good URL returned ${good.status}, expected 200`);
    }
    if (!bad) {
      throw new Error(`[${label}] browser never fetched the 404 URL ${badUrl}`);
    }
    if (bad.status !== 404) {
      throw new Error(`[${label}] 404 URL returned ${bad.status}, expected 404`);
    }

    // Failed assets must be VISIBLE: the web runtime's RuntimeEvent sink turns
    // each AssetError into a console.error naming the asset. (Before the sink,
    // failures fell back to eprintln! — nothing in a browser.)
    const assetError = (url) =>
      log.some(
        (m) => m.startsWith("error:") && m.includes(url) && m.includes("failed to load"),
      );
    if (!assetError(badUrl)) {
      throw new Error(
        `[${label}] no console.error for the 404 asset ${badUrl}. Log tail: ${log
          .slice(-8)
          .join(" | ")}`,
      );
    }

    if (softUrl) {
      const soft = responses.find((r) => r.url === softUrl);
      if (!soft || soft.status !== 200) {
        throw new Error(
          `[${label}] soft-404 URL should have returned 200 (got ${soft?.status}) — the case needs a 200-with-HTML body`,
        );
      }
      // The magic-byte guard must fail the load (HTML never reaches the glTF
      // parser) and say so in the console.
      if (!assetError(softUrl)) {
        throw new Error(
          `[${label}] no console.error for the soft-404 asset ${softUrl} — the wasm magic-byte guard did not reject the HTML body. Log tail: ${log
            .slice(-8)
            .join(" | ")}`,
        );
      }
    }

    // Nothing may have thrown uncaught in the page — a wasm panic surfaces as
    // a pageerror, and a dead frame loop would otherwise pass the checks below
    // silently.
    const pageErrors = log.filter((m) => m.startsWith("pageerror:"));
    if (pageErrors.length > 0) {
      throw new Error(`[${label}] uncaught page error(s): ${pageErrors.join(" | ")}`);
    }

    // The 404 must have degraded to the fallback asset — NOT crashed the frame
    // loop or the glTF pipeline. A persistent draw error leaves the red overlay
    // up; check it's hidden.
    const overlay = await page.evaluate(() => {
      const el = document.getElementById("functor-lang-error");
      if (!el) return { visible: false, text: "" };
      return {
        visible: getComputedStyle(el).display !== "none",
        text: el.textContent || "",
      };
    });
    if (overlay.visible) {
      throw new Error(
        `[${label}] draw-error overlay visible after loading URL assets: ${overlay.text
          .replace(/\s+/g, " ")
          .trim()}`,
      );
    }

    console.log(
      `PASS  ${label} — good ${good.status} (${goodUrl}), bad ${bad.status} (${badUrl})${
        softUrl ? ", soft-404 rejected by magic guard" : ""
      }, console errors present, overlay hidden`,
    );
  } finally {
    await browser.close();
    bundleServer.close();
    rmSync(dir, { recursive: true, force: true });
  }
}

async function main() {
  let failures = 0;

  // Hermetic case: our own permissive-CORS localhost "CDN".
  const { server: assetServer, port: assetPort } = await startAssetServer();
  const origin = `http://127.0.0.1:${assetPort}`;
  try {
    await runCase({
      label: "hermetic (localhost CORS server)",
      goodUrl: `${origin}/model.glb`,
      badUrl: `${origin}/missing.glb`,
      softUrl: `${origin}/soft404.glb`,
    });
  } catch (e) {
    failures++;
    console.log(`FAIL  ${e.message}`);
  } finally {
    assetServer.close();
  }

  // Optional live-CDN case — off unless explicitly enabled (needs the internet
  // and a CORS-enabled host).
  if (process.env.FUNCTOR_REMOTE_E2E_NETWORK === "1") {
    try {
      await runCase({
        label: "network (assets.babylonjs.com)",
        goodUrl: "https://assets.babylonjs.com/meshes/box.glb",
        badUrl: "https://assets.babylonjs.com/meshes/does-not-exist.glb",
      });
    } catch (e) {
      failures++;
      console.log(`FAIL  ${e.message}`);
    }
  } else {
    console.log(
      "SKIP  network (assets.babylonjs.com) — set FUNCTOR_REMOTE_E2E_NETWORK=1 to run",
    );
  }

  console.log(failures === 0 ? "\nALL CASES PASSED" : `\n${failures} CASE(S) FAILED`);
  process.exit(failures === 0 ? 0 : 1);
}

main().catch((e) => {
  console.error(`harness error: ${e}`);
  process.exit(1);
});
