// touch wasm e2e: real browser TouchEvents reach a game's `sampledInput`
// through the page's touch listeners and the shared transition reducer —
// capability first (`Some` with empty lists on a touch-capable device before
// any contact), a tap's press+release edges with exact canvas-relative CSS
// coordinates, distinct ordinals for a two-finger gesture, and the
// compatibility mouse events staying ALIVE (no preventDefault — `Ui.*`
// widgets must remain tappable on touch devices).
//
// Uses a touch-capable Playwright context (`hasTouch: true`, which also makes
// `navigator.maxTouchPoints > 0`); `page.touchscreen.tap` plus hand-dispatched
// two-finger TouchEvents — real DOM events, so the whole page bridge
// (ordinal remap, phase mapping, coordinates) is exercised.
//
//   npm run build:cli:debug   # once, so target/debug/functor embeds the runtime
//   node e2e/touch-wasm.mjs
import path from "node:path";
import {
  ROOT,
  expect,
  launchSoftwareGL,
  serveExportedBundle,
  waitFor,
} from "./wasm-harness.mjs";

const DIR = path.join(ROOT, "e2e", "fixtures", "touch");
const { server, port } = await serveExportedBundle(DIR);
const browser = await launchSoftwareGL();
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
  await waitFor(log, /\[functor-lang\] loaded/, "the game to load");
  // Capability is declared BEFORE any contact.
  await waitFor(log, /e2e-touch.*surface/, "the touch capability to reach the game");

  // Record whether the browser's compatibility mouse events survive the
  // touch listeners (they must — the Ui.* widget bridge runs on them).
  await page.evaluate(() => {
    window.__compatMouse = 0;
    document
      .getElementById("canvas")
      .addEventListener("mousedown", () => (window.__compatMouse += 1));
  });

  await page.touchscreen.tap(200, 150);
  await waitFor(log, /e2e-touch.*tap/, "the tap's edges to reach the game");

  const line = log.find((l) => l.includes("e2e-touch") && l.includes("tap"));
  // The canvas sits below the scrubber bar (margin-top), so Y is the axis a
  // rect-vs-client mistake shows up on — assert it exactly.
  const canvasTop = await page.evaluate(
    () => document.getElementById("canvas").getBoundingClientRect().top,
  );
  const expectedY = 150 - canvasTop;
  expect(/\btap x=200 y=/.test(line), `press X in canvas CSS pixels (${line})`);
  const y = Number((line.match(/y=([\d.]+)/) ?? [])[1]);
  expect(
    Math.abs(y - expectedY) <= 1,
    `press Y is canvas-relative (${y} ≈ ${expectedY})`,
  );
  expect(/rels=[1-9]/.test(line), "the release edge arrived");

  const compat = await page.evaluate(() => window.__compatMouse);
  expect(compat > 0, "compatibility mouse events still fire (Ui.* stays tappable)");

  // Two simultaneous contacts get DISTINCT ordinals.
  await page.evaluate(() => {
    const canvas = document.getElementById("canvas");
    const rect = canvas.getBoundingClientRect();
    const touch = (id, x, y) =>
      new Touch({
        identifier: id,
        target: canvas,
        clientX: rect.left + x,
        clientY: rect.top + y,
      });
    const pair = [touch(101, 100, 100), touch(202, 300, 100)];
    canvas.dispatchEvent(
      new TouchEvent("touchstart", {
        touches: pair,
        targetTouches: pair,
        changedTouches: pair,
        bubbles: true,
      }),
    );
  });
  await waitFor(log, /e2e-touch.*multi n=2/, "two simultaneous contacts to sample");

  expect(
    !log.some((l) => /\[functor-lang\].*error/.test(l)),
    "no [functor-lang] errors during the run",
  );
} finally {
  await browser.close();
  server.close();
}

console.log(process.exitCode ? "✗ touch wasm e2e FAILED" : "✓ touch wasm e2e passed");
