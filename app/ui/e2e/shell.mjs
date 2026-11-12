// The desktop app's UI shell, headless: `functor-app --headless` serves this
// repo with the UI built into it, and headless Chromium drives the page at
// 1440×900 and 700×900.
//
//   npm run test:app-ui     # builds the UI, then functor-app embedding it, then runs this
//
// FUNCTOR_BIN (a built `functor`: `npm run build:cli:debug`) enables the
// Start/Stop session and no-GPU game view checks, which are skipped without
// it. FUNCTOR_APP_GPU=1 adds the live game view against a hidden (GPU)
// session on examples/primitives — local only, and FUNCTOR_BIN must be built
// from this checkout. APP_UI_SHOTS=<dir> saves a PNG of each rail view at
// each width.
import { spawn, execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, statSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { chromium, expect } from "@playwright/test";

const ROOT = fileURLToPath(new URL("../../..", import.meta.url));
const EXE = process.platform === "win32" ? ".exe" : "";
const APP = `${ROOT}target/debug/functor-app${EXE}`;
const FUNCTOR = process.env.FUNCTOR_BIN && existsSync(process.env.FUNCTOR_BIN) ? process.env.FUNCTOR_BIN : null;
const GPU = process.env.FUNCTOR_APP_GPU === "1";
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
function startServer(port = 0, { dir = ROOT, gpu = false } = {}) {
  const args = ["--headless", "--port", String(port), "-d", dir];
  if (!gpu) args.push("--no-gpu");
  if (FUNCTOR) args.push("--functor", FUNCTOR);
  const child = spawn(APP, args, { stdio: ["ignore", "ignore", "pipe"] });
  let err = "";
  return new Promise((resolve, reject) => {
    child.stderr.on("data", (d) => {
      err += d;
      // Up to the newline: the line can arrive split across chunks.
      const m = err.match(/^\[functor-app\] listening on (http:\/\/\S+)\r?\n/m);
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
  const errors = watchErrors(page);
  const external = [];
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

/** The app's last state, as its WebSocket delivered it. */
function watchState(page) {
  const app = { state: null };
  page.on("websocket", (ws) =>
    ws.on("framereceived", ({ payload }) => {
      const msg = JSON.parse(payload);
      if (msg.type === "state") app.state = msg.state;
    }),
  );
  return app;
}

function watchErrors(page) {
  const errors = [];
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  page.on("pageerror", (e) => errors.push(String(e)));
  return errors;
}

/** The session's `/state.input`, read by the test (the page never talks to it). */
async function sessionInput(debugUrl) {
  return (await (await fetch(`${debugUrl}/state`)).json()).input;
}

/** Game view without a GPU (CI): states, input capture and release, the mode picker. */
async function gameViewNoGpu(browser) {
  const server = await startServer(0, { dir: `${ROOT}examples/counter` });
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  const app = watchState(page);
  const errors = watchErrors(page);
  const view = page.getByTestId("game-view");
  const banner = view.getByRole("status");
  const picker = page.getByLabel("Where the game runs");
  try {
    await page.goto(server.url);
    await expect(view).toHaveText(/^No game running/);
    await expect(view.getByRole("button", { name: "Start" })).toBeEnabled();
    await expect(picker).toHaveValue("hidden");
    // Arrow keys on the picker stay with it rather than switching tabs.
    await picker.focus();
    await page.keyboard.press("ArrowRight");
    await expect(page.getByRole("tab", { name: "Game" })).toHaveAttribute("aria-selected", "true");
    await page.keyboard.press("Escape");
    await picker.selectOption("hidden");
    await expect.poll(() => app.state?.session_mode).toBe("hidden");
    if (!FUNCTOR) {
      // Without a session the picker only records the mode for the next start.
      await picker.selectOption("window");
      await expect.poll(() => app.state?.session_mode).toBe("window");
      await picker.selectOption("hidden");
      await expect.poll(() => app.state?.session_mode).toBe("hidden");
      console.log("SKIPPED: game view with a session — set FUNCTOR_BIN (npm run build:cli:debug)");
      return;
    }

    await view.getByRole("button", { name: "Start" }).click();
    await expect(view).toContainText("Frames need a GPU session", { timeout: 60_000 });
    const debugUrl = app.state.session.status.debug_url;

    // A click captures input; a held key reaches the session and Esc lets it go.
    await expect(banner).toHaveText("Click to play");
    await view.click();
    await expect(banner).toHaveText("Esc to release");
    await page.keyboard.down("KeyW");
    await expect.poll(async () => (await sessionInput(debugUrl)).held_keys).toEqual(["W"]);
    await page.keyboard.press("Escape");
    await expect(banner).toHaveText("Click to play");
    await expect.poll(async () => (await sessionInput(debugUrl)).held_keys).toEqual([]);
    await page.keyboard.up("KeyW");
    // Released, keys stay in the page.
    await page.keyboard.down("KeyA");
    await page.waitForTimeout(300);
    expect((await sessionInput(debugUrl)).held_keys).toEqual([]);
    await page.keyboard.up("KeyA");

    // Chorded buttons each reach the session.
    const buttons = async () => {
      const { left, right } = (await sessionInput(debugUrl)).mouse.buttons;
      return [left, right];
    };
    await view.click();
    await page.mouse.down({ button: "left" });
    await page.mouse.down({ button: "right" });
    await expect.poll(buttons).toEqual([true, true]);
    await page.mouse.up({ button: "left" });
    await expect.poll(buttons).toEqual([false, true]);
    await page.mouse.up({ button: "right" });
    await expect.poll(buttons).toEqual([false, false]);
    // Another app taking focus releases a held key.
    await page.keyboard.down("KeyS");
    await expect.poll(async () => (await sessionInput(debugUrl)).held_keys).toEqual(["S"]);
    await page.evaluate(() => window.dispatchEvent(new Event("blur")));
    await expect.poll(async () => (await sessionInput(debugUrl)).held_keys).toEqual([]);
    await expect(banner).toHaveText("Click to play");
    await page.keyboard.up("KeyS");

    // Leaving the tab releases what is held too.
    await view.click();
    await page.keyboard.down("KeyD");
    await expect.poll(async () => (await sessionInput(debugUrl)).held_keys).toEqual(["D"]);
    await page.getByRole("tab", { name: "Code" }).click();
    await expect.poll(async () => (await sessionInput(debugUrl)).held_keys).toEqual([]);
    await page.keyboard.up("KeyD");
    await page.getByRole("tab", { name: "Game" }).click();
    await expect(banner).toHaveText("Click to play");

    // The picker restarts the session in the other mode (headless here, so no window opens).
    for (const mode of ["window", "hidden"]) {
      const id = app.state.session.id;
      await picker.selectOption(mode);
      await expect.poll(() => app.state.session.id).toBe(id + 1);
      await expect.poll(() => app.state.session.status.kind, { timeout: 60_000 }).toBe("running");
      expect(app.state.session_mode).toBe(mode);
    }
    await expect(view).toContainText("Frames need a GPU session");
    expect(errors, "console errors").toEqual([]);
    console.log("ok: game view (no GPU)");
  } finally {
    await page.close();
    await stopServer(server);
  }
}

/** Connections to `port` held by anything but functor and functor-app: the page's streams. */
function openStreams(port) {
  let out = "";
  try {
    out = execFileSync("lsof", ["-nP", `-iTCP:${port}`, "-sTCP:ESTABLISHED"], { encoding: "utf8" });
  } catch {
    // lsof exits 1 when nothing matches.
  }
  return out
    .split("\n")
    .slice(1)
    .filter((l) => l && !l.startsWith("functor") && l.includes(`->127.0.0.1:${port} `)).length;
}

/** The standard deviation of a PNG's luminance, decoded in the page: near 0 is blank. */
function spread(page, png) {
  return page.evaluate(async (b64) => {
    const bytes = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
    const bitmap = await createImageBitmap(new Blob([bytes], { type: "image/png" }));
    const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
    const ctx = canvas.getContext("2d");
    ctx.drawImage(bitmap, 0, 0);
    const { data } = ctx.getImageData(0, 0, bitmap.width, bitmap.height);
    let sum = 0;
    let sq = 0;
    for (let i = 0; i < data.length; i += 4) {
      const l = 0.2126 * data[i] + 0.7152 * data[i + 1] + 0.0722 * data[i + 2];
      sum += l;
      sq += l * l;
    }
    const n = data.length / 4;
    return Math.sqrt(sq / n - (sum / n) ** 2);
  }, png.toString("base64"));
}

/** Game view on a hidden GPU session: live frames, input, stream lifetime, resize. */
async function gameViewGpu(browser) {
  if (!GPU || !FUNCTOR) {
    console.log("SKIPPED: live game view — set FUNCTOR_APP_GPU=1 and FUNCTOR_BIN to a functor built from this checkout");
    return;
  }
  const server = await startServer(0, { dir: `${ROOT}examples/primitives`, gpu: true });
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  const app = watchState(page);
  const errors = watchErrors(page);
  const view = page.getByTestId("game-view");
  const img = view.locator("img");
  const requested = async () => {
    const src = await img.getAttribute("src");
    return src && Object.fromEntries(new URL(src).searchParams);
  };
  try {
    await page.goto(server.url);
    await view.getByRole("button", { name: "Start" }).click();
    await expect(img).toBeVisible({ timeout: 60_000 });
    const debugUrl = app.state.session.status.debug_url;
    const port = Number(new URL(debugUrl).port);

    // Non-blank frames that change.
    const first = await img.screenshot();
    await page.waitForTimeout(500);
    const second = await img.screenshot();
    expect(first.equals(second), "the frame changed").toBe(false);
    expect(await spread(page, second), "luminance spread").toBeGreaterThan(10);

    // Requested at the view's size in device pixels; the frame fits inside it.
    const size = await view.evaluate((el) => ({ w: el.clientWidth, h: el.clientHeight, dpr: devicePixelRatio }));
    const asked = await requested();
    expect(Math.abs(asked.w - size.w * size.dpr)).toBeLessThanOrEqual(1);
    expect(Math.abs(asked.h - size.h * size.dpr)).toBeLessThanOrEqual(1);
    const natural = await img.evaluate((el) => ({ w: el.naturalWidth, h: el.naturalHeight }));
    expect(natural.w <= asked.w && natural.h <= asked.h, JSON.stringify({ natural, asked })).toBe(true);

    // The pointer maps through the letterbox: the frame's centre and corners are the surface's.
    const [sw, sh] = app.state.session.status.surface;
    const frame = await img.boundingBox();
    await view.click();
    for (const [fx, fy] of [[0.5, 0.5], [0, 0], [1, 1]]) {
      await page.mouse.move(frame.x + fx * frame.width, frame.y + fy * frame.height);
      await expect
        .poll(async () => {
          const { mouse } = await sessionInput(debugUrl);
          return [mouse.x, mouse.y];
        })
        .toEqual([Math.min(Math.round(fx * sw), sw - 1), Math.min(Math.round(fy * sh), sh - 1)]);
    }
    // A key reaches the session.
    await page.keyboard.down("KeyW");
    await expect.poll(async () => (await sessionInput(debugUrl)).held_keys).toEqual(["W"]);
    await page.keyboard.press("Escape");
    await expect.poll(async () => (await sessionInput(debugUrl)).held_keys).toEqual([]);
    await page.keyboard.up("KeyW");
    if (SHOTS) {
      mkdirSync(SHOTS, { recursive: true });
      await page.screenshot({ path: `${SHOTS}/game-1440.png` });
    }

    // One stream while shown; none on another tab or a hidden page; back on return.
    await expect.poll(() => openStreams(port)).toBe(1);
    await page.getByRole("tab", { name: "Code" }).click();
    await expect.poll(() => openStreams(port)).toBe(0);
    await page.getByRole("tab", { name: "Game" }).click();
    await expect.poll(() => openStreams(port)).toBe(1);
    const setHidden = (hidden) =>
      page.evaluate((hidden) => {
        Object.defineProperty(document, "visibilityState", { value: hidden ? "hidden" : "visible", configurable: true });
        document.dispatchEvent(new Event("visibilitychange"));
      }, hidden);
    await setHidden(true);
    await expect.poll(() => openStreams(port)).toBe(0);
    await setHidden(false);
    await expect.poll(() => openStreams(port)).toBe(1);

    // A resize re-requests at the new size and replaces the stream.
    await page.setViewportSize({ width: 1200, height: 760 });
    await expect.poll(async () => (await requested())?.w).not.toBe(asked.w);
    const resized = await view.evaluate((el) => ({ w: el.clientWidth, h: el.clientHeight, dpr: devicePixelRatio }));
    const after = await requested();
    expect(Math.abs(after.w - resized.w * resized.dpr)).toBeLessThanOrEqual(1);
    await expect.poll(() => openStreams(port)).toBe(1);
    await expect(img).toBeVisible();
    if (SHOTS) {
      await page.setViewportSize({ width: 700, height: 900 });
      await expect.poll(async () => Number((await requested())?.w)).toBeLessThan(700);
      await page.waitForTimeout(1000);
      await page.screenshot({ path: `${SHOTS}/game-700.png`, fullPage: true });
    }

    // Stopping the session ends the stream.
    await page.getByRole("complementary", { name: "Explorer" }).getByRole("button", { name: "Stop" }).click();
    await expect(view).toHaveText(/^No game running/);
    await expect.poll(() => openStreams(port)).toBe(0);
    expect(errors, "console errors").toEqual([]);
    console.log("ok: live game view (GPU)");
  } finally {
    await page.close();
    await stopServer(server);
  }
}

const browser = await chromium.launch();
try {
  await run(browser, 1440, 900);
  await run(browser, 700, 900);
  await gameViewNoGpu(browser);
  await gameViewGpu(browser);
} catch (e) {
  console.error(`FAIL: ${e.message}`);
  process.exitCode = 1;
} finally {
  await browser.close();
}
