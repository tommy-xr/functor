// wasm remote-asset e2e: prove that a URL asset (`Scene.model("http://…")`)
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
//   - the draw-error overlay is NOT showing afterward — the 404 degraded to the
//     fallback asset; it did not hang the loop or panic the glTF pipeline.
//
// Why observe the network rather than an in-page "loaded" signal: on wasm a
// failed asset load emits a RuntimeEvent::AssetError that currently falls back
// to `eprintln!` (events.rs) — which goes NOWHERE in a browser. So there is no
// console signal for load success/failure today; the real fetch, seen by
// Playwright, IS the ground truth (see the report for that observability gap).
// This is intentionally an end-to-end NETWORK test, not a glTF-decode test:
// decoding of local assets is already covered by the wasm golden suite; what
// was untested is the remote URL round-trip in a real browser.
//
// Optional live-CDN case: with FUNCTOR_REMOTE_E2E_NETWORK=1, also loads a real
// asset from https://assets.babylonjs.com. OFF by default (needs the internet
// and a CORS-enabled CDN) — the hermetic case is the one that gates.
//
// Run manually (needs the built CLI + Playwright's Chromium — `npx playwright
// install chromium`, the same one wasm-smoke uses):
//
//   npm run build:cli                       # once, so target/debug/functor embeds the runtime
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

// A permissive-CORS "CDN": /model.glb serves a real glb; everything else 404s.
// Every response carries Access-Control-Allow-Origin: * so the game page (a
// different origin) can fetch cross-origin, and a 404 is CORS-visible too.
function startAssetServer() {
  const glb = minimalGlb();
  const server = http.createServer((req, res) => {
    res.setHeader("Access-Control-Allow-Origin", "*");
    if (req.url === "/model.glb") {
      res.writeHead(200, { "Content-Type": "model/gltf-binary" });
      res.end(glb);
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

// A temp Functor Lang project whose draw loads a good URL and a 404 URL, built into a
// self-contained wasm bundle. Both assets fetch once (the asset handle is
// cached), so the browser makes exactly one request per URL.
function buildBundle(goodUrl, badUrl) {
  const dir = mkdtempSync(join(tmpdir(), "functor-remote-e2e-"));
  writeFileSync(
    join(dir, "functor.json"),
    JSON.stringify({ language: "functor-lang", entry: "game.fun" }, null, 2),
  );
  writeFileSync(
    join(dir, "game.fun"),
    `// Generated e2e fixture: load one URL model that resolves and one that 404s.
let init = { frame: 0.0 }

let tick = (model, dt, tts) => { model with frame: model.frame + 1.0 }

let draw = (model, tts) =>
  Frame.create(
    Camera.lookAt(0.0, 2.0, 0.0 - 6.0, 0.0, 0.0, 0.0),
    Scene.group([
      Scene.model("${goodUrl}"),
      Scene.model("${badUrl}") |> Scene.translate(3.0, 0.0, 0.0),
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

async function runCase({ goodUrl, badUrl, label }) {
  const { dir, webDir } = buildBundle(goodUrl, badUrl);
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
      if (r.url() === goodUrl || r.url() === badUrl) {
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
      `PASS  ${label} — good ${good.status} (${goodUrl}), bad ${bad.status} (${badUrl}), overlay hidden`,
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
