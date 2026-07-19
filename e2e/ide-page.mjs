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
//   7. Download builds a real .zip (valid EOCD signature, one entry per file);
//   8. project-aware language intelligence: the starter's cross-module
//      references earn lenses, `Palette.` completes to the sibling's members,
//      and a type error underlines then clears.
//
// Run manually (needs both wasm bundles):
//   wasm-pack build runtime/functor-runtime-web --target=web   # once
//   npm run build:lang-wasm                                    # once
//   node e2e/ide-page.mjs
import { spawn, spawnSync } from "node:child_process";
import { access, readFile } from "node:fs/promises";
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

// The language-intel pkg is REQUIRED by this suite (same rule as
// site-sandbox.mjs): the checks in section 8 must not silently skip.
try {
  await access(`${ROOT}site/dist/pkg/functor_lang_wasm.js`);
} catch {
  console.error(
    "FAIL: site/dist/pkg/functor_lang_wasm.js missing — build it first: npm run build:lang-wasm"
  );
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

  // 8. Project-aware language intelligence. `__lang.ready` resolves once the
  // analysis wasm is up; the pkg is guaranteed present (startup check), so
  // not-ready is a failure, never a skip.
  const langReady = await page.evaluate(() => window.__lang && window.__lang.ready);
  if (!langReady) {
    fail("language analysis not ready (__lang.ready is false)");
  } else {
    const poll = async (fn, pred, timeout = 8000) => {
      const deadline = Date.now() + timeout;
      for (;;) {
        const v = await fn();
        if (pred(v)) return v;
        if (Date.now() > deadline) return v;
        await sleep(150);
      }
    };

    // (a) The entry's cross-module program earns ITS signature lenses — the
    // analysis ran project-wide (a single-file pass has no sibling Palette).
    // Assert content, not mere presence: a stale lens mapped over from another
    // buffer would count but not read `draw`.
    await page.evaluate(() => window.__ide.openFile("game.fun"));
    const gameSource = await page.evaluate(
      () => window.__ide.files().find((f) => f.path === "game.fun").source
    );
    const lensTexts = await poll(
      () => page.locator(".cm-lens").allTextContents(),
      (ts) => ts.some((t) => t.startsWith("draw"))
    );
    if (lensTexts.some((t) => t.startsWith("draw"))) {
      console.log("entry file shows draw's signature lens ✓");
    } else {
      fail(`no draw lens on game.fun: ${JSON.stringify(lensTexts)}`);
    }

    // (b) Dot-completion on the SIBLING module offers its members — the
    // capability the project-aware wasm API adds. Open-and-wait with retries
    // (the sandbox's pattern: a popup open can be swallowed by a lagging
    // transaction, so a single-shot trigger flakes).
    const openCompletion = async (source, cursor, pred) => {
      for (let attempt = 0; attempt < 4; attempt++) {
        await page.evaluate(
          ({ s, c }) => window.__ide.triggerComplete(s, c),
          { s: source, c: cursor }
        );
        const t0 = Date.now();
        while (Date.now() - t0 < 2500) {
          const labels = await page.evaluate(() =>
            [...document.querySelectorAll(".cm-tooltip-autocomplete .cm-completionLabel")].map(
              (el) => el.textContent
            )
          );
          if (pred(labels)) return labels;
          await sleep(150);
        }
      }
      return [];
    };

    // NO cache priming: the lint heartbeat's analyze pass primes the
    // completion cache from the clean buffer on its own — the probe (a
    // broken buffer, trailing dot) must answer from it, same as real typing.
    const probe = `${gameSource}let probe = Palette.`;
    const labels = await openCompletion(probe, probe.length, (ls) => ls.includes("glow"));
    if (labels.includes("glow") && labels.includes("sky")) {
      console.log("Palette. completes to the sibling's members (glow, sky) ✓");
    } else {
      fail(`sibling members not offered: ${JSON.stringify(labels)}`);
    }

    // (c) A type error in the active file underlines, and fixing it clears —
    // the lint loop works against the project-wide pass. Restore the clean
    // buffer first (the completion probe left a broken one, which would
    // underline on its own and mask this check).
    await page.evaluate((src) => window.__ide.setActiveSource(src), gameSource);
    await poll(() => page.locator(".cm-lintRange-error").count(), (n) => n === 0);
    await page.evaluate(
      (src) => window.__ide.setActiveSource(src),
      `${gameSource}let oops = (x: Float) => x + "type error"\n`
    );
    const underlined = await poll(() => page.locator(".cm-lintRange-error").count(), (n) => n > 0);
    if (underlined > 0) console.log("type error draws a lint underline ✓");
    else fail("no lint underline for a type error");

    // The status bar's Problems tab mirrors the same pass, naming the file.
    const problemsTab = page.locator('.statusbar-tab[data-tab="problems"]');
    const flaggedTab = await poll(
      async () => ((await problemsTab.textContent()) || "").trim(),
      (t) => t.includes("1 problem")
    );
    if (flaggedTab.includes("1 problem")) console.log("problems tab counts the type error ✓");
    else fail(`problems tab out of sync: ${flaggedTab}`);
    await problemsTab.click();
    const rowText = await poll(
      async () => {
        const row = page.locator(".problem-row");
        return (await row.count()) ? (await row.first().textContent()) || "" : "";
      },
      (t) => t.includes("game.fun")
    );
    if (rowText.includes("game.fun")) console.log("problems row names the active file ✓");
    else fail(`problem row missing the file: ${rowText}`);
    await problemsTab.click(); // close the panel again

    await page.evaluate((src) => window.__ide.setActiveSource(src), gameSource);
    const cleared = await poll(() => page.locator(".cm-lintRange-error").count(), (n) => n === 0);
    if (cleared === 0) console.log("fixing the type error clears the underline ✓");
    else fail(`underline did not clear (count=${cleared})`);

    if (await waitStatus("live", "after intel checks")) {
      console.log("language intelligence keeps the preview live ✓");
    }

    // (d) Deleting a referenced sibling re-analyzes the OPEN file without an
    // edit (the forced lint pass on a topology change — without it the linter
    // never reruns): a sibling constant of the wrong type errors inside an
    // UNCALLED function in game.fun (uncalled so the preview stays live —
    // type diagnostics are advisory); deleting the sibling makes the module
    // Unknown (tolerated) and the underline must clear, no edit involved.
    await page.evaluate(() => window.__ide.newFile("colors.fun", 'let bad = "nope"\n'));
    await page.evaluate(() => window.__ide.openFile("game.fun"));
    await page.evaluate(
      (src) => window.__ide.setActiveSource(src),
      `${gameSource}let tinted = (s) => s |> Scene.emissive(Color.rgb(0.1, 0.2, Colors.bad))\n`
    );
    const flagged = await poll(() => page.locator(".cm-lintRange-error").count(), (n) => n > 0);
    if (flagged > 0) console.log("wrong-typed sibling constant underlines in the entry ✓");
    else fail("no underline for a wrong-typed sibling reference");

    page.on("dialog", (d) => d.accept());
    await page.click('.file-delete[title="Delete colors.fun"]');
    const afterDelete = await poll(() => page.locator(".cm-lintRange-error").count(), (n) => n === 0);
    if (afterDelete === 0) console.log("deleting the sibling re-analyzes and clears the underline ✓");
    else fail(`underline survived the sibling delete (count=${afterDelete})`);

    await page.evaluate((src) => window.__ide.setActiveSource(src), gameSource);
    if (await waitStatus("live", "after delete-sibling checks")) {
      console.log("delete-sibling checks keep the preview live ✓");
    }

    // (e) Inline expect tests: typing expects earns live gutter states — a
    // pass, a fail (with the decomposed actual-vs-expected detail, which
    // also lands in Problems), and an unrunnable engine call.
    await page.evaluate(
      (src) => window.__ide.setActiveSource(src),
      `${gameSource}let double = (x) => x * 2.0\n` +
        `expect double(4.0) == 8.0\n` +
        `expect double(4.0) == 9.0\n` +
        `expect Scene.cube() == Scene.cube()\n`
    );
    const expectRows = await poll(
      () => page.evaluate(() => window.__lang.expects()),
      (rows) => rows.length === 3 && rows.every((r) => r.state !== "running")
    );
    const states = expectRows.map((r) => r.state).join(",");
    if (states === "pass,fail,unrunnable") {
      console.log("expect gutter settles to pass/fail/unrunnable ✓");
    } else {
      fail(`expect gutter states: ${states || "(none)"}`);
    }
    if ((expectRows[1] || {}).detail.includes("left: 8, right: 9")) {
      console.log("failing expect carries the decomposed detail ✓");
    } else {
      fail(`fail detail: ${JSON.stringify(expectRows[1])}`);
    }
    const dots = await page.locator(".cm-expect").count();
    if (dots === 3) console.log("expect gutter renders three dots ✓");
    else fail(`expected 3 gutter dots, got ${dots}`);

    await page.evaluate((src) => window.__ide.setActiveSource(src), gameSource);
    if (await waitStatus("live", "after expect checks")) {
      console.log("expect checks keep the preview live ✓");
    }
  }

  console.log(process.exitCode ? "RESULT: FAIL" : "RESULT: PASS");
} finally {
  await browser.close();
  server.kill();
}
