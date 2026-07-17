// GROWTH PATH (scaffold, skipped): drive the live-preview WEBVIEW end-to-end.
//
// The inspector-inlay test injects the trace through the extension's LSP-client
// seam. The fuller integration is the wasm push path: open the live preview
// (`Functor Lang: Open Live Preview`), let the game run in the webview iframe,
// PAUSE it, and have the runtime emit a `functor-inspector-trace` postMessage
// that the webview relays to the LSP (extension.js onDidReceiveMessage →
// inspector.relayTrace) — producing the same inlay hints, but from a REAL frame.
//
// This pairs with the separate `inspector-wasm-emit` work: the wasm runtime does
// not emit inspector traces yet, so there is nothing to relay and this cannot go
// green today. It is scaffolded here so it drops in once that lands.
//
// What it still needs before un-skipping:
//   1. `inspector-wasm-emit`: the web runtime must postMessage
//      `{ type: "functor-inspector-trace", trace: <wire doc> }` on pause.
//   2. A way to reach across the webview → iframe boundary from Playwright. VS
//      Code webviews are nested iframes (workbench → webview → game iframe);
//      use `workbox.frameLocator(...)` to step in, or assert the RESULT (inlay
//      hints in the editor) rather than the iframe DOM — the editor assertion is
//      the robust check, same as inspector-inlay.spec.mjs.
//   3. The preview needs the built `functor` CLI on PATH / functor-lang.functorPath
//      and `npm run build:cli` (embedded web runtime) up to date.
import { test, expect, openFile, runCommand } from "./baseTest.mjs";
import { EXPECTED_HINT } from "./trace.mjs";

test.skip("live preview: pausing a real frame relays a trace and shows live hints", async ({
  workbox,
}) => {
  await openFile(workbox, "game.fun");
  await runCommand(workbox, "Functor Lang: Open Live Preview");

  // TODO(inspector-wasm-emit): drive the preview to a paused frame here, then
  // assert the relayed trace's live hints appear in the editor — the robust,
  // DOM-stable check (avoid scraping the game iframe canvas):
  const editor = workbox.locator(".monaco-editor").first();
  await expect(
    editor.getByText(EXPECTED_HINT, { exact: false }).first()
  ).toBeVisible({ timeout: 30_000 });
});
