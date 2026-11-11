// The desktop app's UI shell, headless: `functor-app --headless` serves this
// repo with the UI built into it, and headless Chromium drives the page at
// 1440×900 and 700×900.
//
//   npm run test:app-ui     # builds the UI, then functor-app embedding it, then runs this
//
// FUNCTOR_BIN (a built `functor`: `npm run build:cli:debug`) enables the
// Start/Stop session check, which is skipped without it. APP_UI_SHOTS=<dir>
// saves a PNG of each rail view at each width.
import { spawn, execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, statSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { chromium, expect } from "@playwright/test";

const ROOT = fileURLToPath(new URL("../../..", import.meta.url));
const EXE = process.platform === "win32" ? ".exe" : "";
const APP = `${ROOT}target/debug/functor-app${EXE}`;
const FUNCTOR = process.env.FUNCTOR_BIN && existsSync(process.env.FUNCTOR_BIN) ? process.env.FUNCTOR_BIN : null;
const SHOTS = process.env.APP_UI_SHOTS;
const SWITCH_TO = "examples/counter";
const LOOPBACK = new Set(["127.0.0.1", "localhost"]);

for (const [path, what] of [
  [APP, "cargo build -p functor-app"],
  [`${ROOT}app/ui/dist/index.html`, "npm run build:app-ui"],
]) {
  if (!existsSync(path)) {
    console.error(`FAIL: ${path} missing — run ${what}`);
    process.exit(1);
  }
}

const git = (...args) => execFileSync("git", ["-C", ROOT, ...args], { encoding: "utf8" }).trim();
const BRANCH = git("rev-parse", "--abbrev-ref", "HEAD");
const COMMIT = git("rev-parse", "--short", "HEAD");
const printed = execFileSync(APP, ["--headless", "--print", "-d", ROOT], { encoding: "utf8" });
const DEFAULT = printed.match(/^active: (.+)$/m)[1];
const nameOf = (path) => path.split("/").pop();

/** What the Explorer should list: functor.json and the loadable .fun/.funi modules. */
function expectedFiles(path) {
  const dir = `${ROOT}${path}`;
  return readdirSync(dir)
    .filter((n) => (n === "functor.json" || /^[A-Za-z]\w*\.funi?$/.test(n)) && statSync(`${dir}/${n}`).isFile())
    .sort();
}

/** Starts functor-app and resolves with it and its URL once it listens. */
function startServer(port = 0) {
  const args = ["--headless", "--port", String(port), "-d", ROOT, "--no-gpu"];
  if (FUNCTOR) args.push("--functor", FUNCTOR);
  const child = spawn(APP, args, { stdio: ["ignore", "ignore", "pipe"] });
  let err = "";
  return new Promise((resolve, reject) => {
    child.stderr.on("data", (d) => {
      err += d;
      const m = err.match(/^\[functor-app\] listening on (http:\/\/\S+)$/m);
      if (m) resolve({ child, url: m[1], port: Number(new URL(m[1]).port) });
    });
    child.on("exit", (code) => reject(new Error(`functor-app exited (${code}) before listening:\n${err}`)));
  });
}

function stopServer({ child }) {
  if (child.exitCode !== null) return Promise.resolve();
  return new Promise((resolve) => {
    child.on("exit", resolve);
    child.kill("SIGTERM");
  });
}

async function shot(page, name) {
  if (!SHOTS) return;
  mkdirSync(SHOTS, { recursive: true });
  // Narrow layouts scroll, so capture the whole page.
  await page.screenshot({ path: `${SHOTS}/${name}.png`, fullPage: page.viewportSize().width < 1000 });
}

async function run(browser, width, height) {
  const tag = `${width}x${height}`;
  let server = await startServer();
  const page = await browser.newPage({ viewport: { width, height } });
  const errors = [];
  const external = [];
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("request", (r) => {
    const { hostname } = new URL(r.url());
    if (!LOOPBACK.has(hostname)) external.push(r.url());
  });
  const titlebar = page.locator(".titlebar");
  const explorer = page.getByRole("complementary", { name: "Explorer" });
  const status = page.getByTestId("session-status");
  try {
    await page.goto(server.url);

    // The state arrives over the socket: project and branch in the title bar.
    await expect(titlebar).toContainText(nameOf(DEFAULT));
    await expect(titlebar).toContainText(`${BRANCH} · ${COMMIT}`);
    await expect(titlebar).toContainText("No headset");
    await expect(explorer.getByRole("list", { name: "Files" }).getByRole("listitem")).toHaveText(
      expectedFiles(DEFAULT),
    );

    // Switching project updates the title bar and the file list.
    await page.getByLabel("Project").selectOption(SWITCH_TO);
    await expect(titlebar).toContainText(nameOf(SWITCH_TO));
    await expect(explorer.getByRole("list", { name: "Files" }).getByRole("listitem")).toHaveText(
      expectedFiles(SWITCH_TO),
    );

    const start = explorer.getByRole("button", { name: "Start" });
    const stop = explorer.getByRole("button", { name: "Stop" });
    if (FUNCTOR) {
      await start.click();
      await expect(status).toHaveText("running", { timeout: 60_000 });
      await expect(page.getByTestId("session-frame")).toHaveText(/^\d+$/, { timeout: 30_000 });
      await expect(titlebar).toContainText("Local session · running");
    } else {
      console.log(`SKIPPED (${tag}): Start/Stop session — set FUNCTOR_BIN (npm run build:cli:debug)`);
    }
    await shot(page, `explorer-${width}`);

    // Each rail item shows its pane.
    for (const name of ["Tasks", "Devices", "Explorer"]) {
      await page.getByRole("navigation", { name: "Views" }).getByRole("button", { name }).click();
      await expect(page.getByRole("complementary", { name })).toBeVisible();
      await expect(page.getByRole("navigation").getByRole("button", { name })).toHaveAttribute("aria-pressed", "true");
      if (name !== "Explorer") await shot(page, `${name.toLowerCase()}-${width}`);
    }

    if (FUNCTOR) {
      await stop.click();
      await expect(status).toHaveText("stopped", { timeout: 30_000 });
      await expect(start).toBeEnabled();
    }

    // The chat composer is drawn but inert.
    await expect(page.getByLabel("Message the agent")).toBeDisabled();
    await expect(page.getByRole("button", { name: "Send" })).toBeDisabled();

    // Panes stack rather than overflow sideways.
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
    expect(overflow, "horizontal overflow (px)").toBeLessThanOrEqual(0);
    expect(errors, "console errors").toEqual([]);

    // The page reconnects to a restarted server and takes its state.
    await stopServer(server);
    await expect(titlebar).toContainText("Reconnecting…");
    server = await startServer(server.port);
    await expect(titlebar).not.toContainText("Reconnecting…", { timeout: 15_000 });
    await expect(titlebar).toContainText(nameOf(DEFAULT));
    await expect(page.getByLabel("Project")).toHaveValue(DEFAULT);

    expect(external, "requests to non-loopback hosts").toEqual([]);
    console.log(`ok: ${tag}`);
  } finally {
    await page.close();
    await stopServer(server);
  }
}

const browser = await chromium.launch();
try {
  await run(browser, 1440, 900);
  await run(browser, 700, 900);
} catch (e) {
  console.error(`FAIL: ${e.message}`);
  process.exitCode = 1;
} finally {
  await browser.close();
}
