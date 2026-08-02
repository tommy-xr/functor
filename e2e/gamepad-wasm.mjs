// gamepad wasm e2e: the browser Gamepad API reaches a game's `sampledInput`
// through the web runtime's per-frame poll, with the standard-mapping
// conversion intact (analog trigger value + up-positive stick Y) and the
// selection rules honored (null slots, disconnected and non-standard pads
// are all skipped).
//
// The pads are FAKED by overriding `navigator.getGamepads` before the runtime
// boots — headless CI has no controller, and a fake is also what pins exact
// axis values AND the duck-typing contract: the runtime reads pads with
// unchecked casts + plain property reads, so a wasm-bindgen toolchain bump
// that broke that would fail here, not silently in a real browser. The
// fixture game (e2e/fixtures/gamepad) logs one "e2e-gamepad" line the first
// time it samples a pad with south held, echoing leftStick and rightTrigger.
//
// The bundle is exported with `build wasm` and served from an ephemeral port
// (the e2e/module-role-wasm.mjs shape) rather than driven through `run wasm`
// (which hardcodes :8080) — hermetic next to a dev server someone else runs.
//
//   npm run build:cli:debug   # once, so target/debug/functor embeds the runtime
//   node e2e/gamepad-wasm.mjs
import { execFileSync } from "node:child_process";
import { createReadStream, statSync } from "node:fs";
import http from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const ROOT = fileURLToPath(new URL("..", import.meta.url));
const DIR = path.join(ROOT, "e2e", "fixtures", "gamepad");
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
  const page = await browser.newPage({ viewport: { width: 640, height: 480 } });
  const log = [];
  page.on("console", (m) => log.push(m.text()));
  page.on("pageerror", (e) => log.push(`pageerror: ${e}`));

  // The pad the runtime must SELECT sits behind a null slot (a disconnected
  // index — getGamepads() explicitly returns those), a disconnected pad, and
  // a non-standard-mapping pad, all of which must be skipped. It holds south,
  // pushes the left stick right+up (browser Y is down-positive, so up is -1),
  // and rests the right trigger's ANALOG value at 0.25 while its digital
  // `pressed` stays false — pinning value-over-shadow.
  await page.addInitScript(() => {
    const button = (pressed, value) => ({ pressed, touched: pressed, value });
    const pad = (overrides) => ({
      id: "e2e-fake-pad",
      index: 0,
      connected: true,
      mapping: "standard",
      timestamp: 0,
      axes: [0, 0, 0, 0],
      buttons: Array.from({ length: 17 }, () => button(false, 0)),
      ...overrides,
    });
    const buttons = Array.from({ length: 17 }, () => button(false, 0));
    buttons[0] = button(true, 1); // south
    buttons[7] = button(false, 0.25); // right trigger's analog value
    const live = pad({ index: 3, axes: [0.5, -1.0, 0.0, 0.0], buttons });
    navigator.getGamepads = () => [
      null, // a disconnected slot
      pad({ index: 1, connected: false, buttons: [button(true, 1)] }),
      pad({ index: 2, mapping: "", buttons: [button(true, 1)] }),
      live,
    ];
  });

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
  await waitFor(/e2e-gamepad/, "the sampled pad to reach the game");

  const line = log.find((l) => l.includes("e2e-gamepad"));
  const expect = (cond, what) => {
    console.log(`  ${cond ? "✓" : "✗"} ${what}`);
    if (!cond) process.exitCode = 1;
  };
  expect(line.includes("x=0.5"), `stick X crossed raw (${line})`);
  expect(line.includes("y=1"), "browser down-positive Y negated to up-positive");
  expect(line.includes("rt=0.25"), "analog trigger value (not the digital shadow)");
  expect(
    !log.some((l) => /\[functor-lang\].*error/.test(l)),
    "no [functor-lang] errors during the run",
  );
} finally {
  await browser.close();
  server.close();
}

console.log(process.exitCode ? "✗ gamepad wasm e2e FAILED" : "✓ gamepad wasm e2e passed");
