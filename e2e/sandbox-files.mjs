// Sandbox multi-file e2e: the FILE SIDEBAR and editing a file that is not the
// entry.
//
// The sandbox pushes the loaded example's whole file set over
// `functor-lang-set-project` (e2e/sandbox-mp.mjs proves the push reaches every
// role). This one proves the AUTHORING half of that seam:
//
//   1. a multi-file example (netpong: client/server/protocol/game) shows the
//      sidebar, listing every .fun with the entry open;
//   2. clicking a row switches the buffer — and only the buffer: nothing is
//      pushed, so the running panes keep their models;
//   3. an edit to server.fun — a file the editor could not previously open —
//      hot-reloads the SERVER pane at its own entry, model preserved;
//   4. analysis is project-wide: client.fun's calls into its sibling modules
//      (Protocol.*, Game.*) resolve, and the Problems rows name the file the
//      editor is actually showing;
//   5. a single-file example shows no sidebar at all — unchanged from before
//      the switcher existed.
//
// Run manually (needs the web-runtime wasm bundle):
//
//   wasm-pack build runtime/functor-runtime-web --target=web   # once
//   node e2e/sandbox-files.mjs
import { spawn, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const PORT = Number(process.env.FUNCTOR_FILES_PORT ?? 8127);
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

/** The sidebar as rendered: the visible rows and which one is marked open. */
const sidebar = (page) =>
  page.evaluate(() => {
    const pane = document.querySelector(".file-pane");
    return {
      shown: Boolean(pane) && !pane.hidden,
      rows: [...document.querySelectorAll(".file-row .file-label")].map((n) => n.textContent),
      active: document.querySelector(".file-row.active .file-label")?.textContent ?? null,
      // The rows are a switcher here, not a file manager.
      manage: document.querySelectorAll(".file-pane #new-file, .file-pane .file-delete").length,
    };
  });

const browser = await chromium.launch();

// --- 1–4. netpong: four files, switch, edit the server, per-file problems. ----
{
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  const consoleLog = [];
  page.on("console", (m) => consoleLog.push(m.text()));
  page.on("pageerror", (e) => consoleLog.push(`pageerror: ${e}`));
  await page.goto(`${BASE}/sandbox.html?example=netpong`);
  await page.waitForFunction(() => window.__sandbox?.status().state === "live", {
    timeout: 40000,
  });

  // The rows are the paths the BUILT SITE serves — the entry is copied to
  // `examples/<id>.fun` (examples.ts `exampleEntryPath`), which is also the
  // module the running panes derive, so netpong's client entry reads
  // `netpong.fun` here.
  const listed = await sidebar(page);
  check(
    listed.shown && listed.rows.join(", ") === "netpong.fun, server.fun, protocol.fun, game.fun",
    "a multi-file example lists every .fun in the sidebar",
    listed.rows.join(", ")
  );
  check(
    listed.active === "netpong.fun",
    "the entry is the file that starts open",
    String(listed.active)
  );
  check(listed.manage === 0, "the sandbox sidebar offers no new/delete", `${listed.manage} controls`);

  // Language analysis is project-wide, so the entry's calls into its siblings
  // resolve. Before setLangContext reached the sandbox these were "unknown"
  // errors on a perfectly good sample.
  await page.evaluate(() => window.__lang.ready);
  await page.waitForFunction(
    () => document.querySelector(".statusbar-tab.problems, .statusbar-tab") !== null,
    null,
    { timeout: 10000 }
  );
  await sleep(1500);
  const clientProblems = await page.evaluate(() =>
    [...document.querySelectorAll(".problem-row")].map((n) => n.innerText.replace(/\s+/g, " "))
  );
  // Nothing at all: the sibling modules resolve (single-file analysis reported
  // `unknown module Protocol` on this very sample), and netpong's expects —
  // which the browser's hostless runner cannot execute, since game.fun's
  // palette calls `Color.rgb` — read as unrunnable rather than failing.
  check(
    clientProblems.length === 0,
    "a multi-file example analyzes clean — siblings resolve, no false problems",
    clientProblems.slice(0, 3).join(" | ")
  );

  // --- Switch to server.fun by CLICKING its row. -----------------------------
  const framesBefore = await page.evaluate(() =>
    [...document.querySelectorAll(".mp-pane iframe")].map((f) => f.getAttribute("src"))
  );
  await page.click(".file-row:has(.file-label:text-is('server.fun')) .file-name");
  const opened = await page.evaluate(() => ({
    files: window.__sandbox.files(),
    source: window.__sandbox.getSource(),
  }));
  check(
    opened.files.active === "examples/netpong/server.fun",
    "clicking a row opens that file",
    opened.files.active
  );
  check(
    opened.source.includes("let init") && !opened.source.includes("Client"),
    "the buffer really holds server.fun",
    opened.source.slice(0, 60).replace(/\n/g, " ")
  );
  check(
    opened.files.paths[0] === "examples/netpong.fun",
    "switching files does NOT reorder the project (files[0] is still the entry)",
    opened.files.paths.join(", ")
  );
  const framesAfter = await page.evaluate(() =>
    [...document.querySelectorAll(".mp-pane iframe")].map((f) => f.getAttribute("src"))
  );
  check(
    framesAfter.join("|") === framesBefore.join("|"),
    "a file switch reloads no pane",
    framesAfter.join(" | ")
  );

  // --- Edit server.fun → the SERVER pane hot-reloads. ------------------------
  await page.evaluate(() =>
    window.__sandbox.setSource(`${window.__sandbox.getSource()}\n// sandbox server edit\n`)
  );
  const reloaded = await page
    .waitForFunction(
      () => {
        const lines = [...document.querySelectorAll(".output-line")].map((n) => n.textContent);
        return lines.some((line) => line.includes("[server]") && line.includes("model preserved"));
      },
      null,
      { timeout: 30000 }
    )
    .then(() => true)
    .catch(() => false);
  const lines = await page.locator(".output-line").allTextContents();
  check(
    reloaded,
    "editing server.fun hot-reloads the server pane, model preserved",
    lines.filter((line) => line.includes("[server]")).slice(-2).join(" / ")
  );
  const pushed = await page.evaluate(() =>
    window.__sandbox
      .files()
      .paths.map((path, index) => ({ path, index }))
      .find(({ path }) => path.endsWith("server.fun"))
  );
  check(pushed !== undefined, "server.fun stays in the pushed file set", JSON.stringify(pushed));
  check(
    (await page.evaluate(() => window.__sandbox.status().state)) === "live",
    "the session stays live after an off-entry edit"
  );

  // Problems are per-file: a type error typed into server.fun is reported
  // against server.fun, not against the entry the panel used to hardcode.
  const good = await page.evaluate(() => window.__sandbox.getSource());
  await page.evaluate(
    (src) => window.__sandbox.setSource(`${src}\nlet wrongType: float = "not a float"\n`),
    good
  );
  const located = await page
    .waitForFunction(
      () =>
        [...document.querySelectorAll(".problem-loc")].some((n) =>
          n.textContent.includes("netpong/server.fun")
        ),
      null,
      { timeout: 15000 }
    )
    .then(() => true)
    .catch(() => false);
  const locs = await page.evaluate(() =>
    [...document.querySelectorAll(".problem-loc")].map((n) => n.textContent)
  );
  check(located, "a problem in an off-entry file is reported against THAT file", locs.join(" | "));
  await page.evaluate((src) => window.__sandbox.setSource(src), good);
  await page.waitForFunction(
    () => document.querySelectorAll(".problem-row").length === 0,
    null,
    { timeout: 15000 }
  );

  const errors = consoleLog.filter((line) => /unknown external|panic|pageerror/i.test(line));
  check(errors.length === 0, "no pane reported a load error", errors.slice(0, 2).join(" | "));
  await page.close();
}

// --- 5. A single-file example has no sidebar. ---------------------------------
{
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  await page.goto(`${BASE}/sandbox.html?example=hero`);
  await page.waitForFunction(() => window.__sandbox?.status().state === "live", {
    timeout: 40000,
  });
  const solo = await sidebar(page);
  check(
    !solo.shown && solo.rows.length === 1,
    "a single-file example shows no file sidebar",
    JSON.stringify(solo)
  );

  // …and switching TO a multi-file example brings it back live, without a
  // reload: the sidebar is a function of the loaded project, not of boot.
  await page.selectOption("#example-picker", "netpong");
  await page.waitForFunction(() => window.__sandbox?.status().state === "live", {
    timeout: 40000,
  });
  const grown = await sidebar(page);
  check(
    grown.shown && grown.rows.length === 4 && grown.active === "netpong.fun",
    "switching to a multi-file example reveals the sidebar at its entry",
    JSON.stringify(grown)
  );
  await page.close();
}

await browser.close();
server.kill();
console.log(failures === 0 ? "\nsandbox files e2e passed" : `\n${failures} check(s) failed`);
process.exit(failures === 0 ? 0 : 1);
