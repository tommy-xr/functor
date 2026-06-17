import { readFileSync } from "node:fs";
import { test, expect } from "@playwright/test";

// Wasm golden-image regression, the counterpart to the native runner test
// (runtime/functor-runtime-desktop/tests/golden.rs). Both read the SAME scenario
// list from golden-scenarios.json, so a scenario is validated on both targets.
//
// Each scenario renders a sample at a pinned frame time (?fixed-time, which also
// disables input so the camera can't drift) with an optional ?debug-render mode,
// and compares a screenshot of the WebGL2 canvas to a committed golden.
//
// The dev server (playwright.config.mjs) serves a single sample, chosen by the
// FUNCTOR_SAMPLE env var (default "lighting"). We run the manifest scenarios that
// target wasm AND belong to that served sample; cover another sample by running
// with FUNCTOR_SAMPLE=<sample> (and tagging its scenarios `"wasm"`).
const SAMPLE = process.env.FUNCTOR_SAMPLE || "lighting";

const manifest = JSON.parse(
  readFileSync(new URL("../golden-scenarios.json", import.meta.url)),
);
const scenarios = manifest.scenarios.filter(
  (s) => s.sample === SAMPLE && s.targets.includes("wasm"),
);

for (const scenario of scenarios) {
  test(`${scenario.name} renders deterministically (wasm)`, async ({ page }) => {
    const errors = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    page.on("console", (m) => {
      if (m.type() === "error") errors.push(m.text());
    });

    const params = new URLSearchParams({ "fixed-time": String(scenario.fixedTime) });
    if (scenario.debugRender) params.set("debug-render", scenario.debugRender);
    await page.goto(`/?${params.toString()}`);
    await expect(page.locator("#canvas")).toBeVisible();

    // Let the wasm module initialize and render a stable frame at the pinned time.
    await page.waitForTimeout(2500);

    expect(errors, `page errors:\n${errors.join("\n")}`).toEqual([]);
    await expect(page.locator("#canvas")).toHaveScreenshot(`${scenario.name}-wasm.png`);
  });
}
