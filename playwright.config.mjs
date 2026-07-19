import { defineConfig, devices } from "@playwright/test";

// Headless golden-image test for the wasm runtime, the counterpart to the native
// `golden.rs`. Both iterate the shared scenario list in golden-scenarios.json.
// The wasm bundle is renderer-specific (browser + GL backend), so snapshots are
// committed and compared with a tolerance, and this runs locally/manually (it
// needs the wasm build + a browser), not in CI.
//
// The dev server serves one sample, chosen by FUNCTOR_SAMPLE (default
// "lighting"); the spec runs that sample's wasm-tagged scenarios.
//
// Prerequisite: `npm run build:cli:debug` (rebuilds the embedded web runtime + CLI).
// Run: `npm run test:wasm-golden`   Update: `npm run test:wasm-golden:update`.
const SAMPLE = process.env.FUNCTOR_SAMPLE || "lighting";

export default defineConfig({
  testDir: "./e2e",
  fullyParallel: false,
  expect: {
    // The lit scene has soft gradients; allow a small fraction of pixels to
    // differ (driver/AA wobble). Analogous to the native golden's
    // MAX_DIFF_FRACTION in golden.rs (a touch looser for swiftshader AA).
    toHaveScreenshot: { maxDiffPixelRatio: 0.02 },
  },
  use: {
    baseURL: "http://127.0.0.1:8080",
    viewport: { width: 800, height: 600 },
    deviceScaleFactor: 1,
  },
  projects: [
    {
      name: "chromium",
      use: {
        ...devices["Desktop Chrome"],
        // Software WebGL2 in headless Chromium, for deterministic rendering.
        launchOptions: {
          args: [
            "--use-gl=angle",
            "--use-angle=swiftshader",
            "--enable-unsafe-swiftshader",
            "--ignore-gpu-blocklist",
          ],
        },
      },
    },
  ],
  webServer: {
    // Builds the selected sample's game wasm and serves it at :8080. Needs the
    // CLI built with the current web runtime bundle (`npm run build:cli:debug`).
    // `--no-open` keeps the headless run from popping a stray system browser tab
    // (Playwright drives its own browser).
    command: `./target/debug/functor -d examples/${SAMPLE} run wasm --no-open`,
    url: "http://127.0.0.1:8080",
    timeout: 300_000,
    reuseExistingServer: true,
  },
});
