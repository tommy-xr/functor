// The game view's transport bar (D13): pause, step, seek and resume a
// session by mouse and keyboard, in a pane and compact in Mini, checked
// against the session's own /state. Without a GPU (CI) on examples/counter:
// the clock exists without frames. FUNCTOR_APP_GPU=1 adds a hidden session
// on examples/primitives: the picture holds still while paused, changes on a
// step, and a seek back shows the earlier picture again.
//
//   npm run test:app-ui     # needs FUNCTOR_BIN (npm run build:cli:debug); skipped without it
//
// APP_UI_SHOTS=<dir> saves stills at 1440 and 700 wide and of the mini view,
// and (with a GPU) the frames of a play → pause → step → drag back → resume loop.
import { mkdirSync } from "node:fs";
import { chromium, expect } from "@playwright/test";
import { FUNCTOR, ROOT, requireBuilt, shot, startServer, stopServer, watchErrors, watchExternal, watchState } from "./app.mjs";

const GPU = process.env.FUNCTOR_APP_GPU === "1";

requireBuilt();
if (!FUNCTOR) {
  console.log("SKIPPED: transport — set FUNCTOR_BIN (npm run build:cli:debug)");
  process.exit(0);
}

/** The session's own clock, without the model. */
async function clock(debugUrl) {
  const state = await (await fetch(`${debugUrl}/state?model=0`)).json();
  return { frame: state.frame, ...state.clock };
}

/** Starts the active project's game in a pane and resolves with its bar and debug server. */
async function begin(page, app, server) {
  await page.goto(server.url);
  await page.getByTestId("game-view").getByRole("button", { name: "Start game" }).click();
  const bar = page.getByRole("group", { name: "Transport" });
  await expect(bar).toBeVisible({ timeout: 60_000 });
  return { bar, debugUrl: app.state.session.status.debug_url };
}

/** The playhead's frame as the bar shows it. */
const shown = async (bar) => Number(await bar.getByRole("slider", { name: "Selected frame" }).getAttribute("aria-valuenow"));

/** Drags the playhead to `unit` (0–1) of the rail. */
async function dragTo(page, bar, unit) {
  const head = await bar.getByRole("slider", { name: "Selected frame" }).boundingBox();
  const rail = await bar.locator(".transport-rail").boundingBox();
  await page.mouse.move(head.x + head.width / 2, head.y + head.height / 2);
  await page.mouse.down();
  const x = rail.x + unit * rail.width;
  await page.mouse.move((head.x + x) / 2, head.y + head.height / 2, { steps: 4 });
  await page.mouse.move(x, head.y + head.height / 2, { steps: 4 });
  await page.mouse.up();
}

/** Pause, step, seek by drag, click and key, and resume; in a pane, then Mini. */
async function noGpu(browser) {
  const server = await startServer(0, { dir: `${ROOT}examples/counter` });
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  const app = watchState(page);
  const errors = watchErrors(page);
  const external = watchExternal(page);
  const view = page.getByTestId("game-view");
  try {
    const { bar, debugUrl } = await begin(page, app, server);
    const playhead = bar.getByRole("slider", { name: "Selected frame" });
    const now = () => clock(debugUrl);

    // Playing: the playhead follows the session's frame.
    await expect(bar.getByRole("button", { name: "Pause" })).toBeVisible();
    const first = await shown(bar);
    await expect.poll(() => shown(bar)).toBeGreaterThan(first + 10);
    await expect(page.getByTestId("session-frame")).toHaveText(/^\d+$/);

    // Pause stops the session's frame, and says so on the game view.
    await bar.getByRole("button", { name: "Pause" }).click();
    await expect.poll(async () => (await now()).paused).toBe(true);
    const paused = (await now()).frame;
    await page.waitForTimeout(400);
    expect((await now()).frame, "paused").toBe(paused);
    await expect.poll(() => shown(bar)).toBe(paused);
    await expect(bar.getByRole("button", { name: "Play" })).toBeVisible();
    await expect(view.getByRole("status")).toHaveText("Paused");
    // Paused, a click on the view does not capture input.
    await view.click();
    await expect(view.getByRole("status")).toHaveText("Paused");

    // Step: one frame, past the window the pause froze (as on the web).
    await expect(bar.getByTestId("transport-frame")).toHaveText(`${paused} / ${paused}`);
    await bar.getByRole("button", { name: "Step one frame forward" }).click();
    await expect.poll(async () => (await now()).frame).toBe(paused + 1);
    await expect.poll(() => shown(bar)).toBe(paused + 1);
    await expect(bar.getByTestId("transport-frame")).toHaveText(`${paused + 1} outside / ${paused}`);
    await shot(page, "transport-1440");

    // Drag back: the session shows that earlier frame, still paused, and keeps the future.
    const head = paused + 1;
    await dragTo(page, bar, 0.25);
    // The drag's last seek lands, and the bar and the session agree on it.
    const settled = async () => {
      const frame = (await now()).frame;
      return frame === (await shown(bar)) ? frame : -1;
    };
    await expect.poll(settled).toBeGreaterThan(0);
    let state = await now();
    expect(state.frame).toBeLessThan(head * 0.5);
    expect(state.paused).toBe(true);
    expect(state.recorded.newest, "the future is kept").toBe(head);
    // A click on the rail seeks there too.
    const rail = await bar.locator(".transport-rail").boundingBox();
    await page.mouse.click(rail.x + rail.width * 0.75, rail.y + rail.height / 2);
    await expect.poll(async () => (await now()).frame).toBeGreaterThan(head * 0.5);

    // Keys on the playhead: arrows a frame, Home and End the window's ends
    // (it froze at the pause), and forward from its end steps.
    const at = (await now()).frame;
    await playhead.focus();
    await page.keyboard.press("ArrowLeft");
    await expect.poll(async () => (await now()).frame).toBe(at - 1);
    await page.keyboard.press("ArrowRight");
    await expect.poll(async () => (await now()).frame).toBe(at);
    await page.keyboard.press("Home");
    await expect.poll(async () => (await now()).frame).toBe(state.recorded.oldest);
    await page.keyboard.press("End");
    await expect.poll(async () => (await now()).frame).toBe(paused);
    await page.keyboard.press("ArrowRight");
    await expect.poll(async () => (await now()).frame).toBe(paused + 1);

    // Resume from a sought frame: play branches there, and the bar follows.
    await page.keyboard.press("Home");
    await expect.poll(async () => (await now()).frame).toBe(state.recorded.oldest);
    const generation = (await now()).generation;
    await bar.getByRole("button", { name: "Play" }).click();
    await expect.poll(async () => (await now()).paused).toBe(false);
    state = await now();
    expect(state.generation, "a branch").not.toBe(generation);
    expect(state.frame).toBeLessThan(head);
    await expect.poll(async () => (await now()).frame).toBeGreaterThan(state.frame + 5);
    await expect(view.getByRole("status")).toHaveText("Click to play");

    await page.setViewportSize({ width: 700, height: 900 });
    await expect(bar).toBeVisible();
    await shot(page, "transport-700", { fullPage: true });
    await page.setViewportSize({ width: 1440, height: 900 });

    // Mini: the same bar, compact (no frame count), still driving the session.
    await page.locator(".pane-bar").getByRole("group", { name: "Where the game shows" }).getByRole("button", { name: "Mini" }).click();
    const mini = page.getByTestId("mini");
    const compact = mini.getByRole("group", { name: "Transport" });
    await expect(compact).toBeVisible();
    await expect(compact).toHaveClass(/transport-compact/);
    await expect(compact.getByTestId("transport-frame")).toHaveCount(0);
    await compact.getByRole("button", { name: "Pause" }).click();
    await expect.poll(async () => (await now()).paused).toBe(true);
    await shot(page, "transport-mini");
    await compact.getByRole("button", { name: "Play" }).click();
    await expect.poll(async () => (await now()).paused).toBe(false);

    // Stopped, the bar goes with the session.
    await mini.getByRole("button", { name: "Stop game" }).click();
    await expect(page.getByRole("group", { name: "Transport" })).toHaveCount(0);

    expect(errors, "console errors").toEqual([]);
    expect(external, "non-loopback requests").toEqual([]);
    console.log("ok: transport (no GPU)");
  } finally {
    await page.close();
    await stopServer(server);
  }
}

/** The mean absolute difference of two PNGs' pixels, decoded in the page (0–255). */
function difference(page, a, b) {
  return page.evaluate(
    async ([a, b]) => {
      const pixels = async (b64) => {
        const bytes = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
        const bitmap = await createImageBitmap(new Blob([bytes], { type: "image/png" }));
        const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
        canvas.getContext("2d").drawImage(bitmap, 0, 0);
        return canvas.getContext("2d").getImageData(0, 0, bitmap.width, bitmap.height).data;
      };
      const [pa, pb] = [await pixels(a), await pixels(b)];
      if (pa.length !== pb.length) return 255;
      let sum = 0;
      for (let i = 0; i < pa.length; i++) sum += Math.abs(pa[i] - pb[i]);
      return sum / pa.length;
    },
    [a.toString("base64"), b.toString("base64")],
  );
}

/** A hidden GPU session: the picture follows pause, step and a seek back. */
async function gpu(browser) {
  if (!GPU) {
    console.log("SKIPPED: transport with frames — set FUNCTOR_APP_GPU=1 and FUNCTOR_BIN to a functor built from this checkout");
    return;
  }
  const server = await startServer(0, { dir: `${ROOT}examples/primitives`, gpu: true });
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  const app = watchState(page);
  const errors = watchErrors(page);
  const view = page.getByTestId("game-view");
  const img = view.locator("img");
  const stack = page.locator(".game-stack");
  let loop = 0;
  const shots = process.env.APP_UI_SHOTS;
  if (shots) mkdirSync(shots, { recursive: true });
  const still = async () => shots && (await stack.screenshot({ path: `${shots}/transport-loop-${String(loop++).padStart(2, "0")}.png` }));
  try {
    const { bar, debugUrl } = await begin(page, app, server);
    await expect(img).toBeVisible({ timeout: 60_000 });
    const now = () => clock(debugUrl);
    // Playing: the picture moves.
    for (let i = 0; i < 4; i++) {
      await still();
      await page.waitForTimeout(150);
    }

    // Paused: it holds still.
    await bar.getByRole("button", { name: "Pause" }).click();
    await expect.poll(async () => (await now()).paused).toBe(true);
    await page.waitForTimeout(300);
    const frame = (await now()).frame;
    const pausedShot = await img.screenshot();
    await page.waitForTimeout(400);
    expect(await difference(page, pausedShot, await img.screenshot()), "still while paused").toBeLessThan(0.5);
    await still();

    // A step changes it.
    const playhead = bar.getByRole("slider", { name: "Selected frame" });
    await playhead.focus();
    for (let i = 0; i < 12; i++) {
      await page.keyboard.press("ArrowRight");
      await expect.poll(async () => (await now()).frame).toBe(frame + i + 1);
      if (i % 4 === 3) await still();
    }
    await page.waitForTimeout(300);
    expect(await difference(page, pausedShot, await img.screenshot()), "a step moves it").toBeGreaterThan(0.5);

    // Dragged back, then to the window's end (where it froze at the pause):
    // that earlier frame's picture again, with the steps after it kept.
    await dragTo(page, bar, 0.4);
    await still();
    await page.keyboard.press("End");
    await expect.poll(async () => (await now()).frame).toBe(frame);
    expect((await now()).recorded.newest).toBe(frame + 12);
    await page.waitForTimeout(400);
    expect(await difference(page, pausedShot, await img.screenshot()), "the earlier picture").toBeLessThan(0.5);
    await still();

    // Resumed, it moves on.
    await bar.getByRole("button", { name: "Play" }).click();
    await expect.poll(async () => (await now()).frame).toBeGreaterThan(frame + 10);
    for (let i = 0; i < 4; i++) {
      await page.waitForTimeout(150);
      await still();
    }
    expect(errors, "console errors").toEqual([]);
    console.log("ok: transport (GPU)");
  } finally {
    await page.close();
    await stopServer(server);
  }
}

const browser = await chromium.launch();
try {
  await noGpu(browser);
  await gpu(browser);
} finally {
  await browser.close();
}
