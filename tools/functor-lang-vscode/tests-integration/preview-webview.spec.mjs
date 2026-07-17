// The full wasm push path, end to end: open the live preview
// (`Functor Lang: Open Live Preview` spawns `functor run wasm` and hosts the
// game in a webview), let the game RUN, pause it via the real scrubber
// button, and assert the runtime-emitted `functor-inspector-trace`
// (postMessage → webview relay → LSP notification) produces live-value inlay
// hints in the editor — values from a REAL paused frame, not a canned trace.
//
// The frame chain from the workbench: VS Code hosts the panel in an
// <iframe class="webview"> wrapping an <iframe id="active-frame"> (the
// extension's HTML), which hosts <iframe id="frame"> (the CLI dev server's
// game page, where the scrubber lives). The pause CLICK happens at the
// innermost level; the ASSERTION happens in the Monaco editor — the
// DOM-stable check, same as inspector-inlay.spec.mjs.
//
// Prerequisites beyond the base harness: the `functor` CLI built at
// target/debug (npm run build:cli — the web runtime bundle is embedded, so
// it must be current); the base fixture prepends it to PATH.
import { test, expect, openFile, runCommand } from "./baseTest.mjs";

test("live preview: pausing a real frame relays a trace and shows live hints", async ({
  workbox,
}) => {
  await openFile(workbox, "game.fun");
  await runCommand(workbox, "Functor Lang: Open Live Preview");

  // Step into the game page: workbench webview → active-frame → game iframe.
  const gamePage = workbox
    .frameLocator("iframe.webview")
    .last()
    .frameLocator("#active-frame")
    .frameLocator("#frame");

  // The dev server answered and the game booted once the scrubber's pause
  // button exists (the runtime page mounts it after init).
  const pause = gamePage.locator("#scrub-pause");
  await pause.waitFor({ state: "visible", timeout: 60_000 });
  // Let a few real frames tick so the paused frame has a journal.
  await workbox.waitForTimeout(1500);
  await pause.click();

  // The runtime emits the paused trace → the page posts it to the webview →
  // extension.js relays it over client.sendNotification → the LSP pushes an
  // inlay refresh → Monaco renders the live hints. examples/inspector's
  // `model` is a composite record, so its hint is the depth-limited preview
  // `= { ticks: N, lastTime: … }` — assert on the distinctive field name
  // (type hints render ": Type", never "= { ticks"). STRING matcher, not a
  // regex: Monaco renders inlay-hint spaces as NBSP, which getByText's
  // string form normalizes and a literal-space regex does not.
  const editor = workbox.locator(".monaco-editor").first();
  await expect(editor.getByText("= { ticks:", { exact: false }).first()).toBeVisible({
    timeout: 30_000,
  });

  // A screenshot artifact of the real thing: paused game + live hints.
  await workbox.screenshot({
    path: test.info().outputPath("vscode-live-preview-paused.png"),
  });
});
