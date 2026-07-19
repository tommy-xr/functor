// E2E: a paused inspector trace, delivered through the real extension → LSP
// client seam, makes live-value inlay hints appear in the VS Code editor.
//
// This is the first headless end-to-end test that exercises the actual VS Code
// extension. It drives the real workbench (Quick Open, Command Palette, Monaco)
// via Playwright `_electron` and asserts on the rendered inlay-hint DOM — no
// mocks, no pixel scraping. Full path under test:
//   extension activates → LanguageClient starts functor-lang-lsp →
//   inject command relays the wire-contract trace → LSP inlay-hint provider
//   (hash-gated) → Monaco renders "= 42" next to the `model` binder.
import { test, expect, openFile, runCommand } from "./baseTest.mjs";
import { EXPECTED_HINT, EXPECTED_RANGE_HINT } from "./trace.mjs";

// The palette title of the guarded inject command (see extension.js).
const INJECT_COMMAND = "Functor: [test] Inject Inspector Trace";
// Distinctive substring to type into the palette (brackets omitted so fuzzy
// matching isn't thrown off).
const INJECT_QUERY = "Inject Inspector Trace";

test("a paused trace makes a live-value inlay hint appear in the editor", async ({ workbox }) => {
  // Open the sample's game.fun (opening a .fun activates the extension via
  // onLanguage:functor-lang, which starts the LSP client).
  await openFile(workbox, "game.fun");
  const editor = workbox.locator(".monaco-editor").first();
  await expect(editor).toBeVisible();
  // The buffer is up; confirm the source is what we hashed against.
  await expect(editor.getByText("let update")).toBeVisible();

  // Deliver the canned trace through the real client.sendNotification path.
  await runCommand(workbox, INJECT_QUERY, INJECT_COMMAND);

  // The live value renders as an inlay hint ("= 42") in the editor. The LSP
  // fires workspace/inlayHint/refresh on the trace, so this appears once the
  // server has ingested it and Monaco re-requested hints — Playwright retries.
  await expect(
    editor.getByText(EXPECTED_HINT, { exact: false }).first()
  ).toBeVisible({ timeout: 30_000 });

  // The numeric-range rendering: the same trace carries a 120-hit numeric
  // site — its hint reads as the swept range, not the last sample.
  await expect(
    editor.getByText(EXPECTED_RANGE_HINT, { exact: false }).first()
  ).toBeVisible({ timeout: 15_000 });

  // The recency gutter: the trace's coverage section becomes four gutter
  // decorations (now/before/after/dark — one per def line in the canned
  // coverage). Monaco renders gutterIconPath decorations as glyph-margin
  // elements with our SVGs as background images.
  const gutterCount = await workbox
    .locator(".glyph-margin-widgets > div, .margin-view-overlays .cgmr")
    .filter({ has: workbox.locator(":scope") })
    .count()
    .catch(() => 0);
  const gutterStyles = await workbox.evaluate(() =>
    [...document.querySelectorAll("[class*='TextEditorDecorationType']")]
      .map((el) => getComputedStyle(el).backgroundImage)
      .filter((s) => s.includes("cov-"))
  );
  expect(
    gutterStyles.filter((s) => s.includes("cov-")).length,
    `gutter decorations rendered: ${JSON.stringify(gutterStyles)} (count probe: ${gutterCount})`
  ).toBeGreaterThanOrEqual(4);

  // Artifact: hints + gutter in one frame.
  await workbox.screenshot({ path: test.info().outputPath("vscode-gutter.png") });

  // The hash gate: mutate the buffer so its text no longer hashes to the
  // trace's recorded source hash, and the live hint must disappear ("never wrong
  // values on wrong lines"). Any edit changes the whole-file hash.
  await editor.click();
  await workbox.keyboard.type(" ");
  await expect(editor.getByText(EXPECTED_HINT, { exact: false })).toHaveCount(0, {
    timeout: 30_000,
  });
});
