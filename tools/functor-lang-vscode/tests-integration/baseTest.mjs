// Playwright fixture that launches the real VS Code as an Electron app with the
// Functor Lang extension under development, and exposes its window as a
// Playwright `Page` ("workbox") the tests drive via the Monaco/workbench DOM.
//
// Pattern mirrors microsoft/playwright-vscode's tests-integration harness:
//   - @vscode/test-electron ONLY downloads the VS Code binary
//     (downloadAndUnzipVSCode); the DRIVER is Playwright's `_electron`.
//   - each test gets an isolated --user-data-dir / --extensions-dir under a
//     fresh tmp dir, so runs don't touch the developer's VS Code.
//
// Prerequisite: `functor-lang-lsp` must be on PATH (the extension launches it as
// its language server). It installs to ~/.cargo/bin via
// `cargo install --path tools/functor-lang-lsp` (or `npm run build:lsp`).
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { test as base, _electron } from "@playwright/test";
import { downloadAndUnzipVSCode } from "@vscode/test-electron/out/download.js";

import { buildTrace } from "./trace.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
export const EXT_DIR = path.resolve(HERE, ".."); // tools/functor-lang-vscode
const REPO = path.resolve(EXT_DIR, "..", ".."); // repo root
export const PROJECT_DIR = path.join(REPO, "examples", "inspector");
export const GAME_FUN = path.join(PROJECT_DIR, "game.fun");

// Which VS Code build to download + drive. `stable` is fine for the extension's
// ^1.85 engine.
const VSCODE_VERSION = process.env.FUNCTOR_VSCODE_VERSION || "stable";

export const test = base.extend({
  // The VS Code window as a Playwright Page, workspace = examples/inspector,
  // with the trace-inject seam armed (FUNCTOR_LANG_TEST_HOOKS + a trace file
  // whose per-file sha256 matches the on-disk game.fun so the LSP hash gate
  // passes).
  workbox: async ({}, use, testInfo) => {
    const vscodePath = await downloadAndUnzipVSCode(VSCODE_VERSION);
    const tmp = mkdtempSync(path.join(os.tmpdir(), "functor-vscode-e2e-"));

    // Canned trace, hashed against the exact on-disk game.fun bytes.
    const source = readFileSync(GAME_FUN, "utf8");
    const tracePath = path.join(tmp, "trace.json");
    writeFileSync(tracePath, JSON.stringify(buildTrace(source)));

    // FUNCTOR_E2E_VIDEO=1 records the whole session as .webm into the test's
    // output dir — raw material for PR descriptions (convert to GIF for
    // GitHub embedding, per the repo's pr-visuals convention). Off by default:
    // recording costs a compositor thread and disk.
    const recordVideo =
      process.env.FUNCTOR_E2E_VIDEO === "1"
        ? { dir: testInfo.outputPath("video"), size: { width: 1440, height: 900 } }
        : undefined;
    const electronApp = await _electron.launch({
      executablePath: vscodePath,
      recordVideo,
      args: [
        "--no-sandbox",
        "--disable-gpu-sandbox",
        "--disable-updates",
        "--skip-welcome",
        "--skip-release-notes",
        "--disable-workspace-trust",
        // Keep only the extension under development active.
        "--disable-extensions",
        `--extensionDevelopmentPath=${EXT_DIR}`,
        `--extensions-dir=${path.join(tmp, "extensions")}`,
        `--user-data-dir=${path.join(tmp, "user-data")}`,
        PROJECT_DIR,
      ],
      env: {
        ...process.env,
        FUNCTOR_LANG_TEST_HOOKS: "1",
        FUNCTOR_INSPECTOR_TEST_TRACE: tracePath,
        // The live-preview command spawns `functor run wasm` — resolve the
        // repo's freshly built CLI ahead of anything on the developer's PATH.
        PATH: `${path.join(REPO, "target", "debug")}${path.delimiter}${process.env.PATH}`,
      },
    });

    const workbox = await electronApp.firstWindow();
    await workbox
      .context()
      .tracing.start({ screenshots: true, snapshots: true, title: testInfo.title });

    await use(workbox);

    const traceOut = testInfo.outputPath("vscode-playwright-trace.zip");
    await workbox.context().tracing.stop({ path: traceOut });
    await electronApp.close();
    rmSync(tmp, { recursive: true, force: true });
  },
});

export { expect } from "@playwright/test";

const IS_MAC = process.platform === "darwin";
const CMD = IS_MAC ? "Meta" : "Control";

// The workbench takes a beat to become interactive after firstWindow(); the very
// first keyboard press is otherwise dropped. Gate all driving on this.
export async function waitForWorkbench(workbox) {
  await workbox.locator(".monaco-workbench").waitFor({ state: "visible", timeout: 60_000 });
}

// Open the quick input (palette / quick open) reliably: the first keypress after
// launch can be lost, so retry until its input box appears.
async function openQuickInput(workbox, chord) {
  const input = workbox.locator(".quick-input-widget .input").first();
  for (let attempt = 0; attempt < 5; attempt++) {
    await workbox.keyboard.press(chord);
    try {
      await input.waitFor({ state: "visible", timeout: 3000 });
      return input;
    } catch {
      // Palette didn't open (dropped keypress); try again.
    }
  }
  await input.waitFor({ state: "visible", timeout: 5000 });
  return input;
}

// Open a workspace file by clicking its Explorer tree item (more reliable than
// Quick Open right after launch).
export async function openFile(workbox, name) {
  await waitForWorkbench(workbox);
  const item = workbox.getByRole("treeitem", { name }).first();
  await item.waitFor({ state: "visible", timeout: 30_000 });
  await item.dblclick();
  // Confirm the editor for this file is up before returning.
  await workbox.getByRole("tab", { name }).first().waitFor({ state: "visible", timeout: 30_000 });
}

// Run a command via the Command Palette (Ctrl/Cmd+Shift+P). `query` is typed to
// filter; `title` (defaults to `query`) is the row text to click.
export async function runCommand(workbox, query, title = query) {
  const input = await openQuickInput(workbox, `${CMD}+Shift+P`);
  await input.fill(`>${query}`);
  const row = workbox
    .locator(".quick-input-list .monaco-list-row")
    .filter({ hasText: title })
    .first();
  await row.waitFor({ state: "visible", timeout: 15_000 });
  await row.click();
}
