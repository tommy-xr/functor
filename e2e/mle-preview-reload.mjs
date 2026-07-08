// D4 e2e: model-preserving hot reload on wasm, driven over the page's
// postMessage seam — the exact path the VSCode live-preview panel uses
// (extension → webview → iframe is just plumbing on top of this).
//
// Serves examples/hello-cubes with the built CLI, then drives headless
// Chromium through the full story:
//
//   1. the original game renders (red-dominant pulsing centerpiece);
//   2. a pushed source with a green emissive centerpiece reloads OK
//      ("model preserved") and the pixels actually change to green;
//   3. a probe push whose `tick` errors IFF `model.spin <= 0.5` runs clean —
//      spin only exceeds 0.5 by accumulating across the reloads, so the
//      model demonstrably survived;
//   4. the inverted probe (errors IFF spin > 0.5) DOES error — proving the
//      probe fires and the accumulated value is real, not a lucky default;
//   5. a broken push (parse error) is rejected with the rendered error and
//      the old program keeps rendering;
//   6. a good push after the broken one still works.
//
// Run manually (not part of `playwright test` — it owns its own server):
//
//   npm run build:cli   # once, so target/debug/functor embeds the runtime
//   node e2e/mle-preview-reload.mjs
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const BASE = "http://127.0.0.1:8080";
const ROOT = fileURLToPath(new URL("..", import.meta.url));

// --- The pushed sources. -----------------------------------------------------

// The model shape hello-cubes's init establishes: { spin, beat }. Every push
// keeps `tick` accumulating spin at 0.5/s so the survival probes have a
// value that only time-across-reloads can produce.
const TICK = `let tick = (model, dt: Float, tts: Float) => { model with spin: model.spin + dt * 0.5 }`;
// A static, unmistakably green frame — draw ignores the model, so the pixel
// assertion can't be confused by animation phase.
const GREEN_DRAW = `let draw = (model, tts: Float) =>
  Frame.create(
    Camera.lookAt(0.0, 0.0, -6.0, 0.0, 0.0, 0.0),
    Scene.sphere() |> Scene.emissive(0.1, 1.0, 0.2) |> Scene.scale(2.0))`;

const V_GREEN = `let init = { spin: 0.0, beat: 0.0 }
${TICK}
${GREEN_DRAW}
`;

// Errors at runtime IFF the condition picks the probe arm (a missing-field
// access is a spanned runtime error; a fresh init has spin = 0.0).
const probe = (cond) => `let probeBoom = (m) => m.thisFieldDoesNotExist
let init = { spin: 0.0, beat: 0.0 }
let tick = (model, dt: Float, tts: Float) =>
  match ${cond} with
  | true => { model with spin: model.spin + dt * 0.5 }
  | false => probeBoom(model)
${GREEN_DRAW}
`;
const V_PROBE_SURVIVED = probe("model.spin > 0.5"); // clean iff model kept
const V_PROBE_INVERTED = probe("model.spin < 0.5"); // errors iff model kept

const V_BROKEN = "let init = {\n"; // parse error

// --- Harness. ----------------------------------------------------------------

let failures = 0;
const check = (name, ok, detail = "") => {
  console.log(`${ok ? "PASS" : "FAIL"}: ${name}${ok || !detail ? "" : ` — ${detail}`}`);
  if (!ok) failures += 1;
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const server = spawn("./target/debug/functor", ["-d", "examples/hello-cubes", "run", "wasm", "--no-open"], {
  cwd: ROOT,
  stdio: "ignore",
});
process.on("exit", () => server.kill());

// Wait for the dev server.
for (let i = 0; ; i++) {
  try {
    await fetch(BASE);
    break;
  } catch {
    if (i > 100) throw new Error("dev server never came up");
    await sleep(200);
  }
}

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 640, height: 480 } });
const consoleLog = [];
page.on("console", (m) => consoleLog.push(`${m.type()}: ${m.text()}`));
page.on("pageerror", (e) => consoleLog.push(`pageerror: ${e}`));

await page.goto(BASE);
for (let i = 0; !consoleLog.some((m) => m.includes("[mle] loaded")); i++) {
  if (i > 100) throw new Error(`game never loaded:\n${consoleLog.join("\n")}`);
  await sleep(200);
}

// Collect set-source results (posted back to the sender — here, the window
// itself) and expose a helper to push source and await its result.
await page.evaluate(() => {
  window.__results = [];
  window.addEventListener("message", (e) => {
    if (e.data && e.data.type === "mle-set-source-result") window.__results.push(e.data);
  });
});
const push = async (source) => {
  const before = await page.evaluate(() => window.__results.length);
  await page.evaluate((s) => window.postMessage({ type: "mle-set-source", source: s }, "*"), source);
  await page.waitForFunction((n) => window.__results.length > n, before, { timeout: 5000 });
  return page.evaluate(() => window.__results[window.__results.length - 1]);
};

// Sample the center pixel of the WebGL canvas. drawImage runs inside a rAF
// callback registered after the runtime's, so it copies the just-rendered
// buffer before compositing clears it (the canvas has no
// preserveDrawingBuffer).
const centerPixel = () =>
  page.evaluate(
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

// 1. Let the game run so spin accumulates past the probe threshold
//    (0.5/s × ~1.6s ≈ 0.8), then pin down the original look: hello-cubes's
//    centerpiece color has red = 1.0 always.
await sleep(1600);
const before = await centerPixel();
check("original centerpiece is red-dominant", before[0] > 150, `center = rgb(${before})`);

// 2. Push the green game: reload OK, model preserved, pixels change.
const green = await push(V_GREEN);
check("green push accepted", green.ok === true, JSON.stringify(green));
check(
  "green push status says model preserved",
  green.ok && green.message.includes("model preserved"),
  green.message
);
await sleep(400);
const after = await centerPixel();
check(
  "render changed to the pushed green",
  after[1] > 150 && after[0] < 100,
  `center = rgb(${after})`
);

// 3. Survival probe: tick errors unless spin > 0.5 — a value only the
//    PRESERVED model has (a fresh init restarts at 0.0).
const survived = await push(V_PROBE_SURVIVED);
check("survival probe accepted", survived.ok === true, JSON.stringify(survived));
const errCount = () => consoleLog.filter((m) => m.includes("[mle] tick error")).length;
const errsBefore = errCount();
await sleep(700);
check(
  "model SURVIVED the reloads (probe tick runs clean)",
  errCount() === errsBefore,
  consoleLog.slice(-3).join("\n")
);

// 4. Inverted probe: must error — proving the probe fires and spin really
//    is the accumulated value.
const inverted = await push(V_PROBE_INVERTED);
check("inverted probe accepted", inverted.ok === true, JSON.stringify(inverted));
await sleep(700);
check("inverted probe errors (accumulated spin is real)", errCount() > errsBefore);

// 5. Broken push: rejected with the rendered parse error; the old program
//    keeps rendering (still the static green frame).
const broken = await push(V_BROKEN);
check("broken push rejected", broken.ok === false, JSON.stringify(broken));
check(
  "broken push reports the parse error",
  !broken.ok && broken.message.includes("expected an expression"),
  broken.message
);
await sleep(400);
const stillGreen = await centerPixel();
check(
  "old program keeps rendering after the broken push",
  stillGreen[1] > 150 && stillGreen[0] < 100,
  `center = rgb(${stillGreen})`
);

// 6. A good push after the broken one still lands.
const recovered = await push(V_PROBE_SURVIVED);
check("push after broken push works", recovered.ok === true, JSON.stringify(recovered));

await browser.close();
server.kill();
console.log(failures === 0 ? "\nALL CHECKS PASSED" : `\n${failures} CHECK(S) FAILED`);
process.exit(failures === 0 ? 0 : 1);
