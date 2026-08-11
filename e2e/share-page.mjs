// Share-link e2e: the round trip a reader actually performs — edit, Share, open
// the link somewhere else, and find the same program running.
//
// The codec has its own unit test (site/src/share-link.test.ts). This one proves
// the PAGES:
//
//   1. sandbox, single file: an edit + Share writes `#code=` and copies the URL,
//      and a FRESH page on that URL boots the edited program live (not the
//      pristine example);
//   2. sandbox, multiplayer + off-entry edit: netpong shared with an edited
//      server.fun reopens with all four files, the edit intact, and the SERVER
//      PANE mounted — the role config travels, so the link is still a
//      client+server session;
//   3. sandbox: `#code=` outranks `?example=`, and picking an example again
//      clears the fragment;
//   4. IDE: Share, then open the link in a page whose localStorage already
//      holds a project — the prompt appears, Cancel keeps the stored project
//      (and drops the fragment), Accept opens the shared one WITHOUT persisting
//      it, and the first edit adopts it;
//   5. the assets advisory: a project naming a relative asset the site does not
//      serve shows the banner; an example whose assets the site DOES serve does
//      not.
//
// Run manually (needs the web-runtime wasm bundle):
//
//   wasm-pack build runtime/functor-runtime-web --target=web   # once
//   node e2e/share-page.mjs
import { spawn, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const PORT = Number(process.env.FUNCTOR_SHARE_PORT ?? 8129);
const BASE = `http://127.0.0.1:${PORT}`;
const ROOT = fileURLToPath(new URL("..", import.meta.url));

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

let failures = 0;
const check = (ok, what, detail = "") => {
  console.log(`${ok ? "  ok" : "FAIL"}  ${what}${detail ? ` — ${detail}` : ""}`);
  if (!ok) failures += 1;
};

const build = spawnSync("node", ["site/build.mjs"], { cwd: ROOT, stdio: "inherit" });
if (build.status !== 0) process.exit(build.status ?? 1);

try {
  await fetch(BASE);
  console.error(`port ${PORT} is already in use — kill the process on it first`);
  process.exit(1);
} catch {
  // Nothing listening: good.
}
const server = spawn("node", ["site/serve.mjs", "--port", String(PORT)], {
  cwd: ROOT,
  stdio: "ignore",
});
process.on("exit", () => server.kill());
for (let i = 0; ; i++) {
  try {
    await fetch(BASE);
    break;
  } catch {
    if (i > 50) throw new Error("site server never came up");
    await sleep(200);
  }
}

const browser = await chromium.launch();
// One context throughout: the IDE section needs two pages to see the SAME
// localStorage, and clipboard permission makes the copy path real rather than
// silently falling back to "copy the URL".
const context = await browser.newContext({
  viewport: { width: 1440, height: 900 },
  permissions: ["clipboard-read", "clipboard-write"],
});

const live = (page, timeout = 40000) =>
  page.waitForFunction(
    () => (window.__sandbox ?? window.__ide)?.status().state === "live",
    null,
    { timeout }
  );

/** Click Share and hand back the URL it minted (the page's own address). */
const clickShare = async (page) => {
  await page.click("#share");
  await page.waitForFunction(() => window.location.hash.includes("code="), null, {
    timeout: 15000,
  });
  return page.url();
};

const shareLabel = (page) => page.textContent("#share");

/**
 * A complete little program, marked so a page can be asked WHICH one it holds.
 * Complete on purpose: the IDE section asserts "live", which a program missing
 * `draw` never reaches.
 */
const program = (marker) => `// ${marker}
let init = { t: 0.0 }

let tick = (model, dt: float, tts: float) => { model with t: model.t + dt }

let draw = (model, tts: float) =>
  Frame.create(
    Camera3D.lookAt(Vec3.make(0.0, 2.0, -6.0), Vec3.make(0.0, 0.5, 0.0)),
    Scene.sphere() |> Scene.emissive(Color.rgb(0.15, 1.0, 0.85)) |> Scene.scale(1.4))
`;

const bannerText = (page) =>
  page.evaluate(() => document.querySelector(".share-banner-text")?.textContent ?? "");

// --- 1. Sandbox: an edited single-file example round-trips. -------------------
{
  const page = await context.newPage();
  const consoleLog = [];
  page.on("console", (m) => consoleLog.push(m.text()));
  page.on("pageerror", (e) => consoleLog.push(`pageerror: ${e}`));
  await page.goto(`${BASE}/sandbox.html?example=hero`);
  await live(page);

  const MARK = "// shared-edit marker 4711\n";
  await page.evaluate((mark) => window.__sandbox.setSource(mark + window.__sandbox.getSource()), MARK);
  await page.waitForFunction(() => window.__sandbox.status().state === "live", null, {
    timeout: 30000,
  });

  const url = await clickShare(page);
  check(url.includes("#code="), "Share writes a #code= fragment into the page URL", url.slice(-40));
  check(
    (await shareLabel(page)).includes("copied"),
    "the button confirms itself",
    await shareLabel(page)
  );
  const clipboard = await page.evaluate(() => navigator.clipboard.readText());
  check(clipboard === url, "the full URL is on the clipboard", clipboard.slice(0, 60));
  check((await bannerText(page)) === "", "an asset-free example warns about nothing");

  // A FRESH page on that URL: nothing of the first page's state is available to
  // it, so whatever it shows came out of the fragment.
  const opened = await context.newPage();
  await opened.goto(url);
  await live(opened);
  const state = await opened.evaluate(() => ({
    source: window.__sandbox.getSource(),
    files: window.__sandbox.files(),
    picker: document.querySelector("#example-picker").selectedOptions[0].textContent,
    status: window.__sandbox.status(),
  }));
  check(state.source.startsWith("// shared-edit marker 4711"), "the shared page holds the EDIT");
  check(
    state.files.paths.join(", ") === "hero.fun" && state.files.active === "hero.fun",
    "the project's flat module file is what loads",
    state.files.paths.join(", ")
  );
  check(state.picker === "shared link", "the picker names the loaded link", String(state.picker));
  check(state.status.state === "live", "the shared program is live", JSON.stringify(state.status));
  const errors = consoleLog.filter((line) => /unknown external|panic|pageerror/i.test(line));
  check(errors.length === 0, "no load error on either page", errors.slice(0, 2).join(" | "));
  await opened.close();
  await page.close();
}

// --- 2. Sandbox: netpong, with an edit to the SERVER file. --------------------
{
  const page = await context.newPage();
  await page.goto(`${BASE}/sandbox.html?example=netpong`);
  await live(page);
  await page.click(".file-row:has(.file-label:text-is('server.fun')) .file-name");
  await page.evaluate(() =>
    window.__sandbox.setSource(`${window.__sandbox.getSource()}\n// shared server edit\n`)
  );
  await sleep(500);
  const url = await clickShare(page);
  await page.close();

  const opened = await context.newPage();
  const consoleLog = [];
  opened.on("console", (m) => consoleLog.push(m.text()));
  opened.on("pageerror", (e) => consoleLog.push(`pageerror: ${e}`));
  await opened.goto(url);
  await live(opened);
  const state = await opened.evaluate(() => ({
    files: window.__sandbox.files(),
    status: window.__sandbox.status(),
    serverPanes: document.querySelectorAll(".mp-pane.server").length,
    sidebar: !document.querySelector(".file-pane").hidden,
  }));
  check(
    state.files.paths.join(", ") === "netpong.fun, server.fun, protocol.fun, game.fun",
    "every file travels, entry first",
    state.files.paths.join(", ")
  );
  check(state.sidebar, "the multi-file sidebar is back");
  check(state.serverPanes === 1, "the SERVER pane mounts from the link", String(state.serverPanes));
  // "1+1 running": the clients and the server are counted separately, so the
  // pill itself proves the server pane is live rather than merely mounted.
  await opened
    .waitForFunction(() => window.__sandbox.status().text.includes("+1"), null, { timeout: 40000 })
    .catch(() => {});
  const pill = await opened.evaluate(() => window.__sandbox.status());
  check(
    pill.state === "live" && pill.text.includes("+1"),
    "client and server panes are both live",
    JSON.stringify(pill)
  );
  // The off-entry edit is in the file it was made in, not smeared onto the entry.
  await opened.click(".file-row:has(.file-label:text-is('server.fun')) .file-name");
  const serverSource = await opened.evaluate(() => window.__sandbox.getSource());
  check(
    serverSource.includes("// shared server edit"),
    "the edit to server.fun travelled in server.fun",
    serverSource.slice(-40).replace(/\n/g, " ")
  );
  const errors = consoleLog.filter((line) => /unknown external|panic|pageerror/i.test(line));
  check(errors.length === 0, "no pane reported a load error", errors.slice(0, 2).join(" | "));
  await opened.close();
}

// --- 3. Sandbox: the fragment outranks ?example=, and picking clears it. ------
{
  const page = await context.newPage();
  await page.goto(`${BASE}/sandbox.html?example=counter`);
  await live(page);
  const url = await clickShare(page);
  await page.close();

  const opened = await context.newPage();
  // The query asks for `tetris`; the fragment carries `counter`. The fragment is
  // the only copy of its project, so it wins.
  await opened.goto(url.replace("example=counter", "example=tetris"));
  await live(opened);
  const loaded = await opened.evaluate(() => window.__sandbox.files().paths[0]);
  check(loaded === "counter.fun", "#code= outranks ?example=", loaded);

  await opened.selectOption("#example-picker", "tetris");
  await opened.waitForFunction(() => window.__sandbox.files().paths[0].includes("tetris"), null, {
    timeout: 40000,
  });
  const after = await opened.evaluate(() => window.location.hash);
  check(!after.includes("code="), "picking an example drops the fragment", after || "(empty)");
  await opened.close();
}

// --- 4. IDE: share, prompt, and the localStorage semantics. ------------------
{
  const page = await context.newPage();
  await page.goto(`${BASE}/ide.html`);
  await live(page);

  // The project that gets shared…
  await page.evaluate((source) => window.__ide.setActiveSource(source), program("SHARED"));
  await live(page);
  const url = await clickShare(page);
  check(url.includes("#code="), "the IDE mints a link too", url.slice(-30));
  // …and then the reader's OWN work moves on, which is what a link must not eat.
  await page.evaluate((source) => window.__ide.setActiveSource(source), program("LOCAL"));
  await live(page);
  const stored = await page.evaluate(() => localStorage.getItem("functor-ide-project-v1"));
  check(stored.includes("// LOCAL"), "the reader's edit is what is stored");
  await page.close();

  // Cancel: the stored project stays open and the fragment is dropped.
  const declined = await context.newPage();
  declined.on("dialog", (dialog) => dialog.dismiss());
  await declined.goto(url);
  await live(declined);
  await sleep(500);
  const declinedState = await declined.evaluate(() => ({
    source: window.__ide.files().find((f) => f.path === "game.fun").source,
    hash: window.location.hash,
    stored: localStorage.getItem("functor-ide-project-v1"),
  }));
  check(
    declinedState.source.includes("// LOCAL"),
    "Cancel keeps the stored project open",
    declinedState.source.split("\n")[0]
  );
  check(
    !declinedState.hash.includes("code="),
    "Cancel drops the fragment so a reload doesn't re-ask",
    declinedState.hash || "(empty)"
  );
  check(declinedState.stored.includes("// LOCAL"), "…and localStorage is untouched");
  await declined.close();

  // Accept: the shared project opens, but is NOT written until an edit.
  const accepted = await context.newPage();
  const prompts = [];
  accepted.on("dialog", (dialog) => {
    prompts.push(dialog.message());
    dialog.accept();
  });
  await accepted.goto(url);
  await live(accepted);
  await accepted.waitForFunction(
    () => window.__ide.files().find((f) => f.path === "game.fun").source.includes("// SHARED"),
    null,
    { timeout: 20000 }
  );
  check(prompts.length === 1, "a stored project is never replaced silently", prompts[0] ?? "");
  const beforeEdit = await accepted.evaluate(() =>
    localStorage.getItem("functor-ide-project-v1")
  );
  check(
    beforeEdit.includes("// LOCAL") && !beforeEdit.includes("// SHARED"),
    "looking at a shared project does not persist it",
    beforeEdit.slice(0, 40)
  );
  // …and the first edit adopts it.
  await accepted.evaluate((source) => window.__ide.setActiveSource(source), program("SHARED then mine"));
  await sleep(600);
  const afterEdit = await accepted.evaluate(() => localStorage.getItem("functor-ide-project-v1"));
  check(
    afterEdit.includes("// SHARED then mine"),
    "the first edit adopts the shared project",
    afterEdit.slice(0, 50)
  );
  await accepted.close();
}

// --- 5. The assets advisory. --------------------------------------------------
{
  // An example whose local assets the site DOES serve (synthwave's two
  // textures) must not warn: the link drops nothing.
  const served = await context.newPage();
  await served.goto(`${BASE}/sandbox.html?example=synthwave`);
  await live(served);
  await clickShare(served);
  await sleep(1200);
  check(
    (await bannerText(served)) === "",
    "site-served assets travel — no advisory",
    await bannerText(served)
  );

  // A locator the site cannot serve does warn, on the encode side…
  await served.evaluate(() =>
    window.__sandbox.setSource(
      `let missing = Asset.texture("not-on-this-site.png")\n${window.__sandbox.getSource()}`
    )
  );
  await sleep(600);
  const url = await clickShare(served);
  const warned = await served
    .waitForFunction(
      () =>
        (document.querySelector(".share-banner-text")?.textContent ?? "").includes(
          "not-on-this-site.png"
        ),
      null,
      { timeout: 15000 }
    )
    .then(() => true)
    .catch(() => false);
  check(warned, "an unservable relative locator warns when sharing", await bannerText(served));
  await served.close();

  // …and on the decode side, for whoever opens the link.
  const opened = await context.newPage();
  await opened.goto(url);
  const told = await opened
    .waitForFunction(
      () =>
        (document.querySelector(".share-banner-text")?.textContent ?? "").includes(
          "not-on-this-site.png"
        ),
      null,
      { timeout: 30000 }
    )
    .then(() => true)
    .catch(() => false);
  check(told, "the reader of the link is told too", await bannerText(opened));
  // Non-blocking: the program still runs, and the strip dismisses.
  await live(opened);
  await opened.click(".share-banner-close");
  check((await bannerText(opened)) === "", "the advisory dismisses");
  await opened.close();
}

await context.close();
await browser.close();
server.kill();
console.log(failures === 0 ? "\nshare page e2e passed" : `\n${failures} check(s) failed`);
process.exit(failures === 0 ? 0 : 1);
