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
import path from "node:path";
import {
  ROOT,
  expect,
  launchSoftwareGL,
  serveExportedBundle,
  waitFor,
} from "./wasm-harness.mjs";

const DIR = path.join(ROOT, "e2e", "fixtures", "gamepad");
const { server, port } = await serveExportedBundle(DIR);
const browser = await launchSoftwareGL();
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
  await waitFor(log, /\[functor-lang\] loaded/, "the game to load");
  await waitFor(log, /e2e-gamepad/, "the sampled pad to reach the game");

  const line = log.find((l) => l.includes("e2e-gamepad"));
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
