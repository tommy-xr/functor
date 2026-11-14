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
import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { chromium, expect } from "@playwright/test";
import { APP, APP_ARGS, chooseProject, FUNCTOR, RELEASE_HINT, ROOT, requireBuilt, sessionInput, startServer, stopServer, tempProject, watchErrors, watchExternal, watchState } from "./app.mjs";

const GPU = process.env.FUNCTOR_APP_GPU === "1";
const SHOTS = process.env.APP_UI_SHOTS;
const SWITCH_TO = "examples/counter";

requireBuilt();

const git = (...args) => execFileSync("git", ["-C", ROOT, ...args], { encoding: "utf8" }).trim();
const BRANCH = git("rev-parse", "--abbrev-ref", "HEAD");
const COMMIT = git("rev-parse", "--short", "HEAD");
const printed = execFileSync(APP, [...APP_ARGS, "--headless", "--print", "-d", ROOT], { encoding: "utf8" });
const DEFAULT = printed.match(/^active: (.+)$/m)[1];
const nameOf = (path) => path.split("/").pop();

/** What the Explorer should list: functor.json and the loadable .fun/.funi modules. */
function expectedFiles(path) {
  const dir = `${ROOT}${path}`;
  return readdirSync(dir)
    .filter((n) => (n === "functor.json" || /^[A-Za-z]\w*\.funi?$/.test(n)) && statSync(`${dir}/${n}`).isFile())
    .sort();
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
  const external = watchExternal(page);
  const titlebar = page.locator(".titlebar");
  const explorer = page.getByRole("complementary", { name: "Explorer" });
  const status = titlebar.locator("[class*='pill-session-']");
  try {
    // Without the launch's token (here, the printed URL's fragment) the page
    // cannot connect: its attempts are refused and no state arrives.
    const stranger = await browser.newPage();
    const attempts = [];
    stranger.on("websocket", (ws) => {
      attempts.push(ws);
      ws.on("framereceived", () => attempts.push("frame"));
    });
    await stranger.goto(server.url.split("#")[0]);
    await expect.poll(() => attempts.length, { message: "socket attempts" }).toBeGreaterThanOrEqual(2);
    await expect(stranger.locator(".titlebar")).toContainText("No session token — open the URL functor-app printed");
    expect(attempts, "frames without the token").not.toContain("frame");
    await stranger.close();

    await page.goto(server.url);

    // The state arrives over the socket: project and branch in the title bar.
    await expect(titlebar).toContainText(nameOf(DEFAULT));
    await expect(titlebar).toContainText(`${BRANCH} · ${COMMIT}`);
    await expect(titlebar).toContainText("Headset · Not paired");
    // In a browser the bar keeps its own padding: no room for traffic lights.
    await expect(page.locator("html")).not.toHaveClass(/native-titlebar/);
    await expect(titlebar).toHaveCSS("padding-left", "16px");
    // The token is dropped from the address bar once read; the title names the project.
    expect(page.url(), "address bar").not.toContain("token");
    await expect(page).toHaveTitle(`Functor — ${nameOf(DEFAULT)}`);
    await expect(explorer.getByRole("list", { name: "Files" }).getByRole("listitem")).toHaveText(
      expectedFiles(DEFAULT),
    );

    // Switching project updates the title bar and the file list.
    await chooseProject(page, SWITCH_TO);
    await expect(titlebar).toContainText(nameOf(SWITCH_TO));
    await expect(explorer.getByRole("list", { name: "Files" }).getByRole("listitem")).toHaveText(
      expectedFiles(SWITCH_TO),
    );

    // The project's game starts on its own, paused on its first frame (D37).
    const start = page.getByRole("button", { name: "Start game" });
    const stop = page.getByRole("button", { name: "Stop game" });
    if (FUNCTOR) {
      await expect(status).toHaveText("Local session · running", { timeout: 60_000 });
      // Paused on its first frame until Play.
      await expect(page.getByTestId("transport-frame")).toHaveText(/^0( \/ 0)?$/, { timeout: 30_000 });
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
      await expect(status).toHaveText("Local session · stopped", { timeout: 30_000 });
      await expect(page.getByTestId("game-view")).toHaveText("The game is stopped");
      await expect(start).toBeEnabled();
    }

    // The chat composer takes text; Send waits for some.
    await expect(page.getByLabel("Message the agent")).toBeEditable();
    await expect(page.getByRole("button", { name: "Send" })).toBeDisabled();

    // Panes stack rather than overflow sideways.
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
    expect(overflow, "horizontal overflow (px)").toBeLessThanOrEqual(0);
    expect(errors, "console errors").toEqual([]);

    // The page reconnects to a restarted server and takes its state.
    await stopServer(server);
    await expect(titlebar).toContainText("Reconnecting…");
    server = await startServer(server.port, { home: server.home, token: server.token });
    await expect(titlebar).not.toContainText("Reconnecting…", { timeout: 15_000 });
    await expect(titlebar).toContainText(nameOf(DEFAULT));
    await expect(explorer.getByRole("button", { name: /^Project: / })).toHaveAccessibleName(`Project: ${DEFAULT}`);

    expect(external, "requests to non-loopback hosts").toEqual([]);
    console.log(`ok: ${tag}`);
  } finally {
    await page.close();
    await stopServer(server);
  }
}

/** Game view without a GPU (CI): states, input capture and release, the placement control. */
async function gameViewNoGpu(browser) {
  const server = await startServer(0, { dir: `${ROOT}examples/counter` });
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  const app = watchState(page);
  const errors = watchErrors(page);
  const view = page.getByTestId("game-view");
  const banner = view.getByRole("status");
  const placement = page.getByRole("group", { name: "Where the game shows" });
  const placed = () => app.state?.layouts[app.state.active]?.game.placement;
  const note = page.getByTestId("game-note");
  try {
    await page.goto(server.url);
    await expect(placement.getByRole("button", { name: "Pane" })).toHaveAttribute("aria-pressed", "true");
    // No headset placement until M2, not even disabled.
    await expect(placement.getByRole("button")).toHaveText(["Pane", "Mini", "Own window"]);
    if (!FUNCTOR) {
      // Without a session the control only records the placement for the next start.
      await placement.getByRole("button", { name: "Own window" }).click();
      await expect.poll(placed).toBe("window");
      await note.getByRole("button", { name: "Bring back to a pane" }).click();
      await expect.poll(placed).toBe("pane");
      console.log("SKIPPED: game view with a session — set FUNCTOR_BIN (npm run build:cli:debug)");
      return;
    }

    // Opened, the game is there, paused; input waits for Play.
    await expect(view).toContainText("Frames need a GPU session", { timeout: 60_000 });
    const debugUrl = app.state.session.status.debug_url;
    await expect(page.getByRole("group", { name: "Transport" }).getByRole("button", { name: "Play" })).toBeVisible();
    await expect(banner).toHaveText("Paused");
    await page.getByRole("group", { name: "Transport" }).getByRole("button", { name: "Play" }).click();
    await expect(page.getByRole("group", { name: "Transport" }).getByRole("button", { name: "Pause" })).toBeVisible();

    // A click captures input; a held key reaches the session and ⌘Esc lets it go.
    await expect(banner).toHaveText("Click to play");
    await view.click();
    await expect(banner).toHaveText(RELEASE_HINT);
    await page.keyboard.down("KeyW");
    await expect.poll(async () => (await sessionInput(debugUrl)).held_keys).toEqual(["W"]);
    await page.keyboard.press("Meta+Escape");
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

    // Leaving the tab (for a file in its pane) releases what is held too.
    await view.click();
    await page.keyboard.down("KeyD");
    await expect.poll(async () => (await sessionInput(debugUrl)).held_keys).toEqual(["D"]);
    await page.getByRole("button", { name: "game.fun", exact: true }).click();
    await expect.poll(async () => (await sessionInput(debugUrl)).held_keys).toEqual([]);
    await page.keyboard.up("KeyD");
    await page.getByRole("tab", { name: "Game", exact: true }).click();
    await expect(banner).toHaveText("Click to play");

    // Own window restarts the session in the other mode (headless here, so no
    // window opens); the canvas says so, and bringing it back restarts it,
    // paused on its first frame again.
    for (const back of [false, true]) {
      const id = app.state.session.id;
      if (back) await note.getByRole("button", { name: "Bring back to a pane" }).click();
      else await placement.getByRole("button", { name: "Own window" }).click();
      await expect.poll(() => app.state.session.id).toBe(id + 1);
      await expect.poll(() => app.state.session.status.kind, { timeout: 60_000 }).toBe("running");
      expect(placed()).toBe(back ? "pane" : "window");
      await expect(note).toHaveCount(back ? 0 : 1);
      if (!back) {
        await expect(note).toContainText("Own window: sessions here run without a GPU");
        await expect(page.getByRole("tab", { name: "Game", exact: true })).toHaveCount(0);
        await expect(view).toHaveCount(0);
      }
    }
    await expect(view).toContainText("Frames need a GPU session");
    await expect(page.getByRole("group", { name: "Transport" }).getByRole("button", { name: "Play" })).toBeVisible();
    expect(errors, "console errors").toEqual([]);
    console.log("ok: game view (no GPU)");
  } finally {
    await page.close();
    await stopServer(server);
  }
}

/** A game that fails to load says why where its frame would be; fixed, it opens paused (D37). */
async function brokenGame(browser) {
  if (!FUNCTOR) {
    console.log("SKIPPED: broken game — set FUNCTOR_BIN (npm run build:cli:debug)");
    return;
  }
  const dir = tempProject("examples/counter");
  const source = readFileSync(`${dir}/game.fun`, "utf8");
  writeFileSync(`${dir}/game.fun`, `${source}\nlet oops = (\n`);
  const server = await startServer(0, { dir });
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  const errors = watchErrors(page);
  const view = page.getByTestId("game-view");
  try {
    await page.goto(server.url);
    await expect(view.locator(".reason")).toContainText("expected an expression", { timeout: 60_000 });
    await expect(page.getByRole("button", { name: "Start game" })).toBeEnabled();
    writeFileSync(`${dir}/game.fun`, source);
    await expect(page.getByRole("group", { name: "Transport" }).getByRole("button", { name: "Play" })).toBeVisible({ timeout: 60_000 });
    expect(errors, "console errors").toEqual([]);
    console.log("ok: broken game");
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
    await expect(img).toBeVisible({ timeout: 60_000 });
    const debugUrl = app.state.session.status.debug_url;
    const port = Number(new URL(debugUrl).port);

    // Opened paused: its first frame, non-blank, holding still (D37).
    await page.waitForTimeout(1000);
    const still = await img.screenshot();
    expect(await spread(page, still), "luminance spread").toBeGreaterThan(10);
    await page.waitForTimeout(500);
    expect(still.equals(await img.screenshot()), "the paused frame held").toBe(true);
    if (SHOTS) {
      mkdirSync(SHOTS, { recursive: true });
      await page.screenshot({ path: `${SHOTS}/game-opened-1440.png` });
    }

    // Played: non-blank frames that change.
    await page.getByRole("group", { name: "Transport" }).getByRole("button", { name: "Play" }).click();
    await expect(page.getByRole("group", { name: "Transport" }).getByRole("button", { name: "Pause" })).toBeVisible();
    await page.waitForTimeout(300);
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
    await page.keyboard.press("Meta+Escape");
    await expect.poll(async () => (await sessionInput(debugUrl)).held_keys).toEqual([]);
    await page.keyboard.up("KeyW");
    if (SHOTS) {
      mkdirSync(SHOTS, { recursive: true });
      await page.screenshot({ path: `${SHOTS}/game-1440.png` });
    }

    // One stream while shown; none on another tab or a hidden page; back on return.
    await expect.poll(() => openStreams(port)).toBe(1);
    await page.getByRole("button", { name: "game.fun", exact: true }).click();
    await expect.poll(() => openStreams(port)).toBe(0);
    await page.getByRole("tab", { name: "Game", exact: true }).click();
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

    // Moving the Game tab to another pane moves the one view: the same <img>,
    // never a second stream, and frames that keep changing.
    await page.setViewportSize({ width: 1440, height: 900 });
    await img.evaluate((el) => (el.dataset.mark = "the one"));
    await page.getByRole("button", { name: "Split right" }).first().click();
    await expect(page.locator(".pane-box")).toHaveCount(2);
    const peaks = [];
    // lsof blocks this process, so sampled sparsely.
    const sample = setInterval(() => peaks.push(openStreams(port)), 150);
    try {
      const from = await page.getByRole("tab", { name: "Game", exact: true }).boundingBox();
      const to = await page.locator(".pane-box").nth(1).locator(".pane-body").boundingBox();
      await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2);
      await page.mouse.down();
      await page.mouse.move(from.x + from.width / 2 + 20, from.y + 10, { steps: 5 });
      await page.mouse.move(to.x + to.width / 2, to.y + to.height / 2, { steps: 10 });
      await page.mouse.up();
      // game.fun, opened above, stays behind in the first pane.
      await expect(page.locator(".pane-box").nth(1).getByRole("tab", { name: "Game", exact: true })).toBeVisible();
      await expect(img).toHaveAttribute("data-mark", "the one");
      await expect.poll(() => openStreams(port)).toBe(1);
      await page.waitForTimeout(600);
    } finally {
      clearInterval(sample);
    }
    expect(Math.max(...peaks), "streams open at once while moving").toBeLessThanOrEqual(1);
    const moved = await img.screenshot();
    await page.waitForTimeout(500);
    expect(moved.equals(await img.screenshot()), "the frame changed after the move").toBe(false);
    expect(await page.getByTestId("game-view").count()).toBe(1);
    if (SHOTS) await page.screenshot({ path: `${SHOTS}/game-moved-1440.png` });

    // Pane → Mini → Pane moves the same view too: the same <img>, never a
    // second stream, the same session, frames that keep changing, and a key
    // still reaching the game in Mini.
    const session = app.state.session.id;
    const minis = [];
    const sampleMini = setInterval(() => minis.push(openStreams(port)), 150);
    const mini = page.getByTestId("mini");
    try {
      await page.getByRole("group", { name: "Where the game shows" }).getByRole("button", { name: "Mini" }).click();
      await expect(mini.locator("img")).toHaveAttribute("data-mark", "the one");
      await expect(page.getByRole("tab", { name: "Game", exact: true })).toHaveCount(0);
      await expect.poll(() => openStreams(port)).toBe(1);
      await page.waitForTimeout(600);
      const inMini = await img.screenshot();
      await page.waitForTimeout(500);
      expect(inMini.equals(await img.screenshot()), "the frame changed in Mini").toBe(false);
      await view.click();
      await expect(view.getByRole("status")).toHaveText(RELEASE_HINT);
      await page.keyboard.down("KeyW");
      await expect.poll(async () => (await sessionInput(debugUrl)).held_keys).toEqual(["W"]);
      await page.keyboard.press("Meta+Escape");
      await expect.poll(async () => (await sessionInput(debugUrl)).held_keys).toEqual([]);
      await page.keyboard.up("KeyW");
      if (SHOTS) await page.screenshot({ path: `${SHOTS}/game-mini-1440.png` });
      await mini.getByRole("button", { name: "Dock" }).click();
      await expect(page.getByRole("tab", { name: "Game", exact: true })).toBeVisible();
      await expect(img).toHaveAttribute("data-mark", "the one");
      await expect.poll(() => openStreams(port)).toBe(1);
      await page.waitForTimeout(600);
    } finally {
      clearInterval(sampleMini);
    }
    expect(Math.max(...minis), "streams open at once across pane → mini → pane").toBeLessThanOrEqual(1);
    expect(app.state.session.id, "the same session").toBe(session);
    const docked = await img.screenshot();
    await page.waitForTimeout(500);
    expect(docked.equals(await img.screenshot()), "the frame changed after docking").toBe(false);

    // Stopping the session ends the stream.
    await page.getByRole("button", { name: "Stop game" }).click();
    await expect(view).toHaveText("The game is stopped");
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
  await brokenGame(browser);
  await gameViewGpu(browser);
} catch (e) {
  console.error(`FAIL: ${e.message}`);
  process.exitCode = 1;
} finally {
  await browser.close();
}
