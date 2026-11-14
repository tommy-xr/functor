// The app's check of the `functor` it runs (D40, interim): a CLI from before
// the check is named, with its version and the fix, in a banner under the
// title bar and where the game would be; once one is built, Try again finds
// it and the game starts. Runs at 1440×900 and 700×900; Unix only (the CLIs
// here are shell scripts).
//
//   npm run test:app-ui
//
// APP_UI_SHOTS=<dir> saves the banner (cli-old-<width>) and the cleared page
// (cli-fixed-<width>).
import { chmodSync, existsSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { chromium, expect } from "@playwright/test";
import { requireBuilt, shot, startServer, stopServer, tempDir, tempProject, watchErrors, watchExternal } from "./app.mjs";

if (process.platform === "win32") {
  console.log("SKIPPED: CLI check — its fake CLIs are shell scripts");
  process.exit(0);
}
requireBuilt();

/** Replaces the fake `functor` at `path` with `script`, which logs its checks to `<path>.log`. */
function write(path, script) {
  writeFileSync(path, `#!/bin/sh\n[ "$1" = app-compat ] && echo >> '${path}.log'\n${script}\n`);
  chmodSync(path, 0o755);
}

/** How many times the app has checked the fake `functor` at `path`. */
function checks(path) {
  return existsSync(`${path}.log`) ? readFileSync(`${path}.log`, "utf8").length : 0;
}

// A release from before the check: knows no `app-compat`, says its version.
const OLD = `case "$1" in
  --version) echo 'functor-cli 0.1.2' ;;
  *) echo "error: unrecognized subcommand '$1'" >&2; exit 2 ;;
esac`;
// A current one: passes the check; its game never answers (it stays Starting).
const FIXED = `[ "$1" = app-compat ] && { echo '{"app_protocol":1,"version":"0.0.0-dev","debug_protocol":43}'; exit 0; }
exec sleep 60`;

async function check(browser, width, height) {
  const functor = join(tempDir("functor-app-e2e-cli-"), "functor");
  write(functor, OLD);
  const server = await startServer(0, { dir: tempProject("examples/counter"), functor });
  const page = await browser.newPage({ viewport: { width, height } });
  const errors = watchErrors(page);
  const external = watchExternal(page);
  const banner = page.getByRole("alert").filter({ hasText: "functor CLI" });
  try {
    await page.goto(server.url);
    await expect(banner).toBeVisible();
    await expect(banner).toContainText(`The functor CLI at ${functor} (0.1.2) is older than this app`);
    // Named with --functor, so that is the fix (not a build where this app looks).
    await expect(banner).toContainText("Pass --functor <path> to a current one.");
    // The game it refused to start says the same, once, where its error would be.
    const message = (await banner.locator("p").textContent()).trim();
    await expect(page.getByTestId("game-view")).toHaveText(message);
    await shot(page, `cli-old-${width}`);

    // Still old: the check runs again, with the same message.
    expect(checks(functor)).toBe(1);
    await banner.getByRole("button", { name: "Try again" }).click();
    await expect.poll(() => checks(functor)).toBe(2);
    await expect(banner.getByRole("button", { name: "Try again" })).toBeEnabled();
    await expect(banner.locator("p")).toHaveText(message);

    // Built meanwhile: found, and the game starts.
    write(functor, FIXED);
    await banner.getByRole("button", { name: "Try again" }).click();
    await expect(banner).toHaveCount(0);
    await expect(page.getByTestId("game-view")).toHaveText("Starting…");
    await shot(page, `cli-fixed-${width}`);

    expect(errors, "console errors").toEqual([]);
    expect(external, "non-loopback requests").toEqual([]);
    console.log(`ok: CLI check at ${width}px`);
  } finally {
    await page.close();
    await stopServer(server);
  }
}

const browser = await chromium.launch();
try {
  await check(browser, 1440, 900);
  await check(browser, 700, 900);
} finally {
  await browser.close();
}
