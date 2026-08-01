// Focused browser coverage for the site's opt-in Vim keybindings:
//
//   - the standard default does not fetch the Vim chunk;
//   - opting in through the visible control lazy-loads and persists it;
//   - failed loads remain visible and preserve the right preference;
//   - common normal/insert operations work and normal-mode Tab is inert;
//   - the IDE keeps the active Vim mode while switching files;
//   - the landing hero inherits the preference and supports blockwise edits;
//   - opting back out removes Vim and persists Standard.

import { spawn, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const PORT = 8134;
const BASE = `http://127.0.0.1:${PORT}`;
const ROOT = fileURLToPath(new URL("..", import.meta.url));
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

let failures = 0;
const check = (name, ok, detail = "") => {
  console.log(`${ok ? "PASS" : "FAIL"}: ${name}${ok || !detail ? "" : ` — ${detail}`}`);
  if (!ok) failures += 1;
};

const built = spawnSync("node", ["site/build.mjs"], { cwd: ROOT, stdio: "inherit" });
if (built.status !== 0) process.exit(built.status ?? 1);

const server = spawn("node", ["site/serve.mjs", "--port", String(PORT)], {
  cwd: ROOT,
  stdio: "ignore",
});
process.on("exit", () => server.kill());
for (let attempt = 0; ; attempt++) {
  try {
    await fetch(BASE);
    break;
  } catch {
    if (attempt > 50) throw new Error("site server never came up");
    await sleep(200);
  }
}

const browser = await chromium.launch();
try {
  // Failure behavior is part of the option's contract too: an explicit failed
  // opt-in reports the problem without stealing keyboard focus, while an
  // automatic restore failure must not erase the user's saved Vim preference.
  const failureContext = await browser.newContext({ viewport: { width: 1280, height: 800 } });
  const failurePage = await failureContext.newPage();
  await failurePage.route("**/assets/sandbox-dist.js", (route) => route.abort("failed"));
  await failurePage.goto(`${BASE}/sandbox.html`);
  await failurePage.waitForFunction(() => window.__sandbox);
  await failurePage.click("#editor-keybindings");
  await failurePage.waitForFunction(() => window.__sandbox.keybindings().error);
  check(
    "failed opt-in reports unavailability and keeps button focus",
    (await failurePage.locator("#editor-keybindings").innerText()) === "keys: unavailable" &&
      (await failurePage.evaluate(() => document.activeElement?.id)) === "editor-keybindings" &&
      (await failurePage.getAttribute("#editor-keybindings", "aria-live")) === "polite" &&
      (await failurePage.getAttribute("#editor-keybindings", "aria-disabled")) === "true"
  );
  check(
    "explicit failed opt-in persists Standard",
    (await failurePage.evaluate(() => localStorage.getItem("functor-editor-keybindings-v1"))) ===
      "standard"
  );
  await failurePage.evaluate(() =>
    localStorage.setItem("functor-editor-keybindings-v1", "vim")
  );
  await failurePage.reload();
  await failurePage.waitForFunction(
    () => window.__sandbox && window.__sandbox.keybindings().error
  );
  check(
    "restore-time chunk failure preserves the Vim preference",
    (await failurePage.evaluate(() => localStorage.getItem("functor-editor-keybindings-v1"))) ===
      "vim"
  );
  await failurePage.evaluate(() => document.querySelector("#editor-keybindings").click());
  check(
    "unavailable control does not discard the preserved preference",
    (await failurePage.evaluate(() => localStorage.getItem("functor-editor-keybindings-v1"))) ===
      "vim"
  );
  await failureContext.close();

  const context = await browser.newContext({ viewport: { width: 1280, height: 800 } });
  const page = await context.newPage();
  const requested = [];
  page.on("request", (request) => requested.push(new URL(request.url()).pathname));

  // Standard is the default and keeps the adapter off the network.
  await page.goto(`${BASE}/sandbox.html`);
  await page.waitForFunction(() => window.__sandbox);
  await sleep(100);
  check(
    "sandbox defaults to Standard keybindings",
    (await page.evaluate(() => window.__sandbox.keybindings().mode)) === "standard" &&
      (await page.locator("#editor-keybindings").innerText()) === "keys: standard"
  );
  check(
    "sandbox keeps keybindings in the status bar",
    (await page.locator("#statusbar > .statusbar-strip > #editor-keybindings").count()) === 1 &&
      (await page.locator(".sandbox-controls #editor-keybindings").count()) === 0
  );
  check(
    "sandbox does not fetch Vim before opt-in",
    !requested.includes("/assets/sandbox-dist.js"),
    requested.filter((path) => path.startsWith("/assets/")).join(", ")
  );

  // The visible option owns the transition and advertises its pressed state.
  await page.click("#editor-keybindings");
  await page.waitForFunction(
    () => window.__sandbox.keybindings().mode === "vim" && !window.__sandbox.keybindings().loading
  );
  check("sandbox fetches its lazy Vim chunk after opt-in", requested.includes("/assets/sandbox-dist.js"));
  check(
    "sandbox Vim control reports enabled",
    (await page.getAttribute("#editor-keybindings", "aria-pressed")) === "true" &&
      (await page.locator("#editor-keybindings").innerText()) === "keys: vim"
  );
  check(
    "sandbox shows the Vim mode panel",
    (await page.locator("#editor .cm-vim-panel").innerText()).includes("NORMAL")
  );

  const source = "alpha\nbeta\ngamma\n";
  await page.evaluate((text) => window.__sandbox.setSource(text), source);
  await page.locator("#editor .cm-content").focus();
  await page.keyboard.press("Escape");
  await page.keyboard.press("g");
  await page.keyboard.press("g");
  const beforeTab = await page.evaluate(() => window.__sandbox.source());
  await page.keyboard.press("Tab");
  const afterTab = await page.evaluate(() => window.__sandbox.source());
  check("Tab is inert in Vim normal mode", afterTab === beforeTab, afterTab);

  await page.keyboard.press("j");
  await page.keyboard.press("d");
  await page.keyboard.press("d");
  const afterDelete = await page.evaluate(() => window.__sandbox.source());
  check("Vim normal-mode operator edits the buffer", afterDelete === "alpha\ngamma\n", afterDelete);
  await page.keyboard.press("u");
  check(
    "Vim undo restores the edit",
    (await page.evaluate(() => window.__sandbox.source())) === source
  );
  await page.keyboard.press("g");
  await page.keyboard.press("g");
  await page.keyboard.press("0");
  await page.keyboard.press("i");
  await page.keyboard.type("Z");
  await page.keyboard.press("Escape");
  check(
    "Vim insert mode returns to Normal with Escape",
    (await page.evaluate(() => window.__sandbox.source())).startsWith("Zalpha") &&
      (await page.locator("#editor .cm-vim-panel").innerText()).includes("NORMAL")
  );

  // Vim handles Escape before CodeMirror's keymaps; confirm autocomplete still
  // closes while that same key returns the adapter to Normal.
  const langReady = await page.evaluate(() => window.__lang.ready);
  check("language intelligence is ready for the Vim completion check", langReady);
  if (langReady) {
    const completionSource = "let init = 0.0\nlet shape = Scene.";
    await page.keyboard.press("i");
    await page.evaluate(
      (text) => window.__sandbox.triggerComplete(text, text.length),
      completionSource
    );
    await page.waitForFunction(
      () => document.querySelectorAll(".cm-tooltip-autocomplete .cm-completionLabel").length > 0
    );
    await page.keyboard.press("Escape");
    await page.waitForFunction(() => !document.querySelector(".cm-tooltip-autocomplete"));
    check(
      "Escape closes completion and returns Vim to Normal",
      (await page.locator("#editor .cm-vim-panel").innerText()).includes("NORMAL")
    );
  }

  // The persisted preference is shared by the multi-file IDE. Its one editor
  // view survives file switches, so the current modal state should too.
  await page.goto(`${BASE}/ide.html`);
  await page.waitForFunction(
    () => window.__ide && window.__ide.keybindings().mode === "vim" && !window.__ide.keybindings().loading
  );
  check("IDE inherits the Vim preference", true);
  check("IDE fetches its own lazy Vim chunk", requested.includes("/assets/ide-dist.js"));
  check(
    "IDE keeps keybindings in the status bar",
    (await page.locator("#statusbar > .statusbar-strip > #editor-keybindings").count()) === 1 &&
      (await page.locator(".sandbox-controls #editor-keybindings").count()) === 0
  );
  await page.locator("#editor .cm-content").focus();
  await page.keyboard.press("i");
  await page.waitForFunction(() => document.querySelector("#editor .cm-vim-panel")?.textContent?.includes("INSERT"));
  await page.evaluate(() => window.__ide.openFile("palette.fun"));
  check(
    "IDE file switch preserves Vim insert mode",
    (await page.locator("#editor .cm-vim-panel").innerText()).includes("INSERT")
  );
  await page.keyboard.press("Escape");

  // The landing mini-editor has no basicSetup, so visual selection exercises
  // its explicit drawSelection integration as well as preference inheritance.
  await page.goto(BASE);
  await page.waitForFunction(
    () => window.__hero && window.__hero.keybindings().mode === "vim" && !window.__hero.keybindings().loading
  );
  check("hero inherits the Vim preference", true);
  check("hero fetches its lazy Vim chunk", requested.includes("/assets/hero-dist.js"));
  const heroControl = await page.evaluate(() => {
    const control = document.querySelector(".hero-editor-keybindings");
    return {
      pressed: control?.getAttribute("aria-pressed") ?? null,
      text: control?.textContent ?? null,
      direct: control?.parentElement?.id === "hero-editor",
    };
  });
  check(
    "hero Vim control reports enabled",
    heroControl.pressed === "true" && heroControl.text === "keys: vim" && heroControl.direct,
    JSON.stringify(heroControl)
  );
  await page.locator(".hero-editor .cm-content").waitFor({ state: "visible" });
  await page.locator(".hero-editor .cm-content").focus();
  await page.keyboard.press("Escape");
  await page.keyboard.press("g");
  await page.keyboard.press("g");
  await page.keyboard.press("v");
  await page.keyboard.press("j");
  await page.locator(".hero-editor .cm-selectionBackground").first().waitFor({
    state: "attached",
    timeout: 2_000,
  }).catch(() => {});
  const heroSelection = await page.evaluate(() => ({
    activeClass: document.activeElement?.className ?? "",
    nativeSelection: window.getSelection()?.toString() ?? "",
    selectionLayers: document.querySelectorAll(".hero-editor .cm-selectionLayer").length,
    backgrounds: document.querySelectorAll(".hero-editor .cm-selectionBackground").length,
  }));
  check(
    "hero draws Vim visual selection",
    heroSelection.backgrounds > 0,
    JSON.stringify(heroSelection)
  );
  await page.keyboard.press("Escape");

  // Blockwise insert needs CodeMirror's allowMultipleSelections facet. A
  // selection-layer count alone cannot distinguish this from one wrapped
  // range, so assert that Vim applies the edit at both cursors.
  await page.evaluate(() => window.__hero.setRegion("alpha\nbeta\ngamma"));
  await page.locator(".hero-editor .cm-content").focus();
  await page.keyboard.press("Escape");
  await page.keyboard.press("g");
  await page.keyboard.press("g");
  await page.keyboard.press("Control+v");
  await page.keyboard.press("j");
  await page.keyboard.press("I");
  await page.keyboard.type("X");
  await page.keyboard.press("Escape");
  const blockEditedRegion = await page.evaluate(() => window.__hero.region());
  check(
    "hero blockwise visual mode edits at multiple cursors",
    blockEditedRegion.split("\n").filter((line) => line.startsWith("X")).length === 2,
    blockEditedRegion
  );

  // Opting out through the hero's visible control removes the adapter and
  // updates the shared preference for the next page.
  await page.click(".hero-editor-keybindings");
  await page.waitForFunction(() => window.__hero.keybindings().mode === "standard");
  check(
    "hero option disables Vim keybindings",
    (await page.getAttribute(".hero-editor-keybindings", "aria-pressed")) === "false" &&
      (await page.locator(".hero-editor-keybindings").innerText()) === "keys: standard"
  );
  check(
    "opting out persists Standard",
    (await page.evaluate(() => localStorage.getItem("functor-editor-keybindings-v1"))) === "standard"
  );

  await context.close();
} finally {
  await browser.close();
  server.kill();
}

if (failures) {
  console.error(`RESULT: FAIL (${failures})`);
  process.exit(1);
}
console.log("RESULT: PASS");
