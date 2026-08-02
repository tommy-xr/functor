// touch wasm e2e: real browser TouchEvents reach a game's `sampledInput`
// through the page's touch listeners and the shared transition reducer —
// capability first (`Some` with empty lists on a touch-capable device before
// any contact), then a tap's press+release edges with canvas-relative CSS
// coordinates.
//
// Uses a touch-capable Playwright context (`hasTouch: true`, which also makes
// `navigator.maxTouchPoints > 0`) and `page.touchscreen.tap` — real
// dispatched TouchEvents, not synthetic wasm calls, so the whole page bridge
// (ordinal remap, preventDefault, phase mapping) is exercised. The bundle is
// exported with `build wasm` and served from an ephemeral port (the
// e2e/module-role-wasm.mjs shape).
//
//   npm run build:cli:debug   # once, so target/debug/functor embeds the runtime
//   node e2e/touch-wasm.mjs
import { execFileSync } from "node:child_process";
import { createReadStream, statSync } from "node:fs";
import http from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const ROOT = fileURLToPath(new URL("..", import.meta.url));
const DIR = path.join(ROOT, "e2e", "fixtures", "touch");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

console.log(
  execFileSync(path.join(ROOT, "target/debug/functor"), ["-d", DIR, "build", "wasm"], {
    encoding: "utf8",
  })
);

const TYPES = {
  ".html": "text/html",
  ".js": "text/javascript",
  ".wasm": "application/wasm",
  ".fun": "text/plain",
};
const root = path.join(DIR, "dist", "web");
const server = http.createServer((req, res) => {
  let rel = decodeURIComponent(req.url.split("?")[0]);
  if (rel === "/") rel = "/index.html";
  const file = path.join(root, rel);
  try {
    statSync(file);
  } catch {
    res.writeHead(404).end("not found");
    return;
  }
  res.writeHead(200, {
    "Content-Type": TYPES[path.extname(file)] ?? "application/octet-stream",
  });
  createReadStream(file).pipe(res);
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const { port } = server.address();

// Software WebGL2 (swiftshader) so the runtime's GL context comes up on any
// runner — no real GPU needed; this check compares no pixels.
const browser = await chromium.launch({
  args: [
    "--use-gl=angle",
    "--use-angle=swiftshader",
    "--enable-unsafe-swiftshader",
    "--ignore-gpu-blocklist",
  ],
});
try {
  const context = await browser.newContext({
    hasTouch: true,
    viewport: { width: 640, height: 480 },
  });
  const page = await context.newPage();
  const log = [];
  page.on("console", (m) => log.push(m.text()));
  page.on("pageerror", (e) => log.push(`pageerror: ${e}`));

  await page.goto(`http://127.0.0.1:${port}/`);
  const waitFor = async (pattern, what) => {
    const until = Date.now() + 30000;
    while (Date.now() < until) {
      if (log.some((line) => pattern.test(line))) return;
      await sleep(200);
    }
    throw new Error(`timed out waiting for ${what}\n--- console ---\n${log.join("\n")}`);
  };

  await waitFor(/\[functor-lang\] loaded/, "the game to load");
  // Capability is declared BEFORE any contact.
  await waitFor(/e2e-touch.*surface/, "the touch capability to reach the game");

  await page.touchscreen.tap(200, 150);
  await waitFor(/e2e-touch.*tap/, "the tap's edges to reach the game");

  const line = log.find((l) => l.includes("e2e-touch") && l.includes("tap"));
  const expect = (cond, what) => {
    console.log(`  ${cond ? "✓" : "✗"} ${what}`);
    if (!cond) process.exitCode = 1;
  };
  // X is canvas-relative CSS pixels and unaffected by the scrubber's vertical
  // offset; Y just needs to be a positive in-canvas coordinate.
  expect(line.includes("x=200"), `press X in canvas CSS pixels (${line})`);
  expect(/y=\d/.test(line) && !line.includes("y=-"), "press Y positive");
  expect(/rels=[1-9]/.test(line), "the release edge arrived");
  expect(
    !log.some((l) => /\[functor-lang\].*error/.test(l)),
    "no [functor-lang] errors during the run",
  );
} finally {
  await browser.close();
  server.close();
}

console.log(process.exitCode ? "✗ touch wasm e2e FAILED" : "✓ touch wasm e2e passed");
