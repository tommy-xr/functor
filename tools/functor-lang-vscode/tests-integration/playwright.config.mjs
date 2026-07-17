// Headless VS Code E2E for the Functor Lang extension. Unlike the repo-root
// playwright.config.mjs (browser wasm goldens), this drives the real VS Code
// desktop app via Playwright `_electron` (see baseTest.mjs).
//
// Run:      npm run test:e2e            (from tools/functor-lang-vscode)
//    or:    npm run test:vscode-e2e     (from the repo root)
// Headless: it runs with no human interaction. On Linux CI, wrap with xvfb-run
// (VS Code/Electron needs an X display even when unattended); on macOS it
// launches a background window and drives it — still no manual clicking.
//
// Prerequisites:
//   - functor-lang-lsp on PATH (npm run build:lsp, installs to ~/.cargo/bin)
//   - network access on first run to download the VS Code binary (cached after)
import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: ".",
  testMatch: /.*\.spec\.mjs/,
  // VS Code launches are serial and stateful; one worker keeps port/PATH/tmp
  // isolation simple and deterministic.
  fullyParallel: false,
  workers: 1,
  forbidOnly: !!process.env.CI,
  retries: 0,
  // Downloading + launching VS Code and waiting on the LSP is slow.
  timeout: 120_000,
  globalSetup: "./globalSetup.mjs",
  reporter: [["list"]],
});
