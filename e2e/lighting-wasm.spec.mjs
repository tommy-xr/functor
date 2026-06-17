import { test, expect } from "@playwright/test";

// Renders the lighting sample in the wasm runtime at a pinned frame time
// (?fixed-time, which also disables input so the camera can't drift) and
// compares a screenshot of the canvas to a committed golden — the wasm
// counterpart of the native golden test.
test("lighting sample renders deterministically (wasm)", async ({ page }) => {
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/?fixed-time=2");
  await expect(page.locator("#canvas")).toBeVisible();

  // Let the wasm module initialize and render a stable frame at the pinned time.
  await page.waitForTimeout(2500);

  expect(errors, `page errors:\n${errors.join("\n")}`).toEqual([]);
  await expect(page.locator("#canvas")).toHaveScreenshot("lighting-wasm.png");
});
