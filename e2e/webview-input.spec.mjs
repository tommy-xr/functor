import { test, expect } from "@playwright/test";

// The wasm controlled-input path, end to end through a REAL DOM: click the
// shadow-root <input> of examples/webview, type, and assert the model
// round-trip (the greeting line) — including focus survival across the
// per-keystroke innerHTML swap (the slot + selection restore in
// index-functor-lang.html's setupWebview). The native counterpart is the
// worker-level keyboard/focus tests in
// runtime/functor-runtime-desktop/src/webview_overlay.rs.
//
// Run with the webview sample served:
//   FUNCTOR_SAMPLE=webview npx playwright test e2e/webview-input.spec.mjs
const SAMPLE = process.env.FUNCTOR_SAMPLE || "lighting";

test.describe(() => {
  test.skip(SAMPLE !== "webview", "needs the webview sample: FUNCTOR_SAMPLE=webview");

  test("typing into the webview input round-trips through the model", async ({ page }) => {
    const errors = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    page.on("console", (m) => {
      if (m.type() === "error") errors.push(m.text());
    });

    await page.goto("/");
    await expect(page.locator("#canvas")).toBeVisible();
    // Wasm init + the first webview render (the overlay polls per rAF).
    const input = page.locator("#webview input");
    await expect(input).toBeVisible({ timeout: 30000 });
    await expect(input).toHaveAttribute("placeholder", "your name");

    await input.click();
    // Slow enough that every keystroke's model round-trip swaps the overlay's
    // innerHTML BETWEEN characters — each subsequent character only lands if
    // the swap re-focused the rebuilt input (the focus-survival seam).
    await input.pressSequentially("Ada", { delay: 150 });

    await expect(input).toHaveValue("Ada");
    await expect(
      page.locator("#webview p").filter({ hasText: "Hello, Ada!" }),
    ).toBeVisible();
    expect(errors, `page errors:\n${errors.join("\n")}`).toEqual([]);
  });
});
