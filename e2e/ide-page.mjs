// IDE-page e2e: the browser IDE (site/ide.html) end-to-end, headless. Drives
// the `window.__ide` test seam (no synthesized DOM events) through:
//
//   1. the page boots the multi-file starter to "live" (preview loaded);
//   2. the sidebar lists game.fun + palette.fun (the entry + a sibling module);
//   3. editing the SIBLING (palette.fun) hot-swaps and stays "live" — the
//      multi-file loop the single-buffer sandbox can't do;
//   4. a broken edit reports the error and the old program keeps running;
//   5. a good edit recovers to "live";
//   6. a new file adds a module and the preview stays live;
//   7. Download builds a real .zip (valid EOCD signature, one entry per file).
//
// Run manually (needs the wasm bundle):
//   wasm-pack build runtime/functor-runtime-web --target=web   # once
//   node e2e/ide-page.mjs
import { spawn, spawnSync } from "node:child_process";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const PORT = 8126;
const BASE = `http://127.0.0.1:${PORT}`;
const ROOT = fileURLToPath(new URL("..", import.meta.url));
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const fail = (m) => {
  console.error(`FAIL: ${m}`);
  process.exitCode = 1;
};

const GOOD_PALETTE = `let glow = 0.3
let sky = 0.7
`;
const BROKEN_PALETTE = `let glow =
`;

const built = spawnSync("node", ["site/build.mjs"], { cwd: ROOT, stdio: "inherit" });
if (built.status !== 0) {
  console.error("FAIL: site build failed");
  process.exit(1);
}

const server = spawn("node", ["site/serve.mjs", "--port", String(PORT)], { cwd: ROOT, stdio: "ignore" });
const browser = await chromium.launch({
  args: ["--use-gl=angle", "--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--ignore-gpu-blocklist"],
});

try {
  const page = await browser.newPage({ viewport: { width: 1100, height: 640 } });
  const log = [];
  page.on("console", (m) => log.push(m.text()));
  // Start from a clean slate so a stale localStorage project can't mask the
  // starter (the page persists edits across reloads by design).
  await page.addInitScript(() => {
    try {
      localStorage.removeItem("functor-ide-project-v1");
    } catch {}
  });

  for (let i = 0; i < 60; i++) {
    try {
      await page.goto(`${BASE}/ide.html`);
      break;
    } catch {
      await sleep(500);
    }
  }

  const status = () => page.evaluate(() => window.__ide.status());
  const waitStatus = async (state, what, timeoutMs = 20000) => {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      if ((await status()).state === state) return true;
      await sleep(150);
    }
    fail(`timed out waiting for status "${state}" (${what}); last = ${JSON.stringify(await status())}`);
    return false;
  };

  // 1. Boot to live.
  await waitStatus("live", "initial boot");
  if (log.some((l) => l.includes("[functor-lang] loaded game.fun"))) {
    console.log("boots the starter to live ✓");
  } else {
    fail(`no "[functor-lang] loaded game.fun":\n${log.join("\n")}`);
  }

  // 2. Sidebar lists the two starter files.
  const files = await page.evaluate(() => window.__ide.files().map((f) => f.path));
  if (JSON.stringify(files) === JSON.stringify(["game.fun", "palette.fun"])) {
    console.log("file list = game.fun + palette.fun ✓");
  } else {
    fail(`unexpected file list: ${JSON.stringify(files)}`);
  }

  // 3. Edit the SIBLING module → hot-swap, stays live.
  await page.evaluate(() => window.__ide.openFile("palette.fun"));
  await page.evaluate((src) => window.__ide.setActiveSource(src), GOOD_PALETTE);
  if (await waitStatus("live", "sibling hot-swap")) console.log("sibling-module hot-swap stays live ✓");

  // 4. A broken sibling → error, old program keeps running.
  await page.evaluate((src) => window.__ide.setActiveSource(src), BROKEN_PALETTE);
  if (await waitStatus("error", "broken sibling")) console.log("broken sibling shows error ✓");

  // 5. Recover.
  await page.evaluate((src) => window.__ide.setActiveSource(src), GOOD_PALETTE);
  if (await waitStatus("live", "recovery")) console.log("recovery back to live ✓");

  // 6. New module keeps the preview live.
  await page.evaluate(() => window.__ide.newFile("enemy.fun", "let hp = 3.0\n"));
  const withEnemy = await page.evaluate(() => window.__ide.files().map((f) => f.path));
  if (!withEnemy.includes("enemy.fun")) fail("new file not added");
  if (await waitStatus("live", "after new file")) console.log("new module added, stays live ✓");

  // 7. Download builds a real zip.
  const [download] = await Promise.all([
    page.waitForEvent("download"),
    page.click("#download"),
  ]);
  const path = await download.path();
  const bytes = await readFile(path);
  const hasEOCD = bytes.includes(Buffer.from([0x50, 0x4b, 0x05, 0x06])); // end-of-central-dir
  // EOCD is the last 22 bytes (no comment); "total entries" sits at its
  // offset 10, i.e. length - 12.
  const entryCount = bytes.readUInt16LE(bytes.length - 12);
  const text = bytes.toString("latin1");
  const hasManifest = text.includes("functor.json"); // must ship so `build wasm` works
  const hasEntry = text.includes("game.fun");
  // functor.json + game.fun + palette.fun + enemy.fun
  if (hasEOCD && entryCount === 4 && hasManifest && hasEntry) {
    console.log(`download is a valid zip with ${entryCount} entries incl. functor.json ✓`);
  } else {
    fail(`bad zip: EOCD=${hasEOCD} entries=${entryCount} manifest=${hasManifest} entry=${hasEntry}`);
  }

  console.log(process.exitCode ? "RESULT: FAIL" : "RESULT: PASS");
} finally {
  await browser.close();
  server.kill();
}
