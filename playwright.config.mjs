import { defineConfig, devices } from "@playwright/test";

// Headless golden-image test for the wasm runtime, the counterpart to the native
// `golden.rs`. The wasm bundle is renderer-specific (browser + GL backend), so
// snapshots are committed and compared with a tolerance, and this runs
// locally/manually (it needs the wasm build + a browser), not in CI.
//
// Prerequisite: `npm run build:cli` (rebuilds the embedded web runtime + CLI).
// Run: `npm run test:wasm-golden`   Update: `npm run test:wasm-golden:update`.
export default defineConfig({
  testDir: "./e2e",
  fullyParallel: false,
  expect: {
    // The lit scene has soft gradients; allow a small fraction of pixels to
    // differ (driver/AA wobble), like the native golden's tolerance.
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
    // Builds the lighting game wasm and serves it at :8080. Needs the CLI built
    // with the current web runtime bundle (`npm run build:cli`). `--no-open`
    // keeps the headless run from popping a stray system browser tab (Playwright
    // drives its own browser).
    command: "./target/debug/functor -d examples/lighting run wasm --no-open",
    url: "http://127.0.0.1:8080",
    timeout: 300_000,
    reuseExistingServer: true,
  },
});
