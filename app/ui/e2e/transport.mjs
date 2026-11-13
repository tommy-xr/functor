// The game view's transport bar (D13): pause, step, seek and resume a
// session by mouse and keyboard, in a pane and compact in Mini, checked
// against the session's own /state. Without a GPU (CI) on examples/counter:
// the clock exists without frames. Then input and hot-reload marks on a copy
// of examples/breakout (it has an `input` hook). FUNCTOR_APP_GPU=1 adds a hidden session
// on examples/primitives: the picture holds still while paused, changes on a
// step, and a seek back shows the earlier picture again; and one on a weak-jump
// platformer: Extrapolate draws its path, and an edit landing while paused
// peeks at the new one, whose arc over the gap is new.
//
//   npm run test:app-ui     # needs FUNCTOR_BIN (npm run build:cli:debug); skipped without it
//
// APP_UI_SHOTS=<dir> saves stills at 1440 and 700 wide and of the mini view,
// and (with a GPU) the frames of a play → pause → step → drag back → resume loop.
import { appendFileSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { chromium, expect } from "@playwright/test";
import {
  clock,
  FUNCTOR,
  ROOT,
  requireBuilt,
  shot,
  startServer,
  stopServer,
  tempProject,
  watchErrors,
  watchExternal,
  watchState,
} from "./app.mjs";

const GPU = process.env.FUNCTOR_APP_GPU === "1";

requireBuilt();
if (!FUNCTOR) {
  console.log("SKIPPED: transport — set FUNCTOR_BIN (npm run build:cli:debug)");
  process.exit(0);
}

/**
 * Opens the app: with no click, the active project's game is there, paused
 * on frame 0 (D37), and Play runs it. Resolves with its bar and debug server.
 */
async function begin(page, app, server) {
  await page.goto(server.url);
  const bar = page.getByRole("group", { name: "Transport" });
  await expect(bar).toBeVisible({ timeout: 60_000 });
  const debugUrl = app.state.session.status.debug_url;
  await expect(bar.getByRole("button", { name: "Play" })).toBeVisible();
  expect(await clock(debugUrl)).toMatchObject({ frame: 0, paused: true });
  await page.waitForTimeout(400);
  expect((await clock(debugUrl)).frame, "held on frame 0").toBe(0);
  await bar.getByRole("button", { name: "Play" }).click();
  await expect.poll(async () => (await clock(debugUrl)).frame).toBeGreaterThan(0);
  // The page has seen it play: a click on the view now captures input.
  await expect(bar.getByRole("button", { name: "Pause" })).toBeVisible();
  return { bar, debugUrl };
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

    // Without frames there is no path to draw.
    await expect(bar.getByRole("button", { name: /^Extrapolate/ })).toBeDisabled();
    // Playing: the playhead follows the session's frame.
    await expect(bar.getByRole("button", { name: "Pause" })).toBeVisible();
    const first = await shown(bar);
    await expect.poll(() => shown(bar)).toBeGreaterThan(first + 10);
    await expect(bar.getByTestId("transport-frame")).toHaveText(/^\d+ \/ \d+$/);

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
    // (it froze at the pause); forward from its end walks the step recorded
    // past it, then steps on from the newest.
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
    await expect.poll(async () => (await now()).frame).toBe(head);
    expect((await now()).recorded.newest, "walked, not branched").toBe(head);
    await page.keyboard.press("ArrowRight");
    await expect.poll(async () => (await now()).frame).toBe(head + 1);

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
    await expect(compact.getByRole("button", { name: "Play" })).toBeVisible();
    await shot(page, "transport-mini");
    await compact.getByRole("button", { name: "Play" }).click();
    await expect.poll(async () => (await now()).paused).toBe(false);

    // Stopped, the bar goes with the session.
    await mini.getByRole("button", { name: "Stop game" }).click();
    await expect(page.getByRole("group", { name: "Transport" })).toHaveCount(0);
    // Start brings it back paused on frame 0.
    await mini.getByRole("button", { name: "Start game" }).click();
    await expect(compact.getByRole("button", { name: "Play" })).toBeVisible({ timeout: 60_000 });
    expect(await clock(app.state.session.status.debug_url)).toMatchObject({ frame: 0, paused: true });

    expect(errors, "console errors").toEqual([]);
    expect(external, "non-loopback requests").toEqual([]);
    console.log("ok: transport (no GPU)");
  } finally {
    await page.close();
    await stopServer(server);
  }
}

/** Marks: a key pressed in the view, and a hot reload of an edited file, each explained by its tooltip. */
async function marks(browser) {
  const dir = tempProject("examples/breakout");
  const server = await startServer(0, { dir });
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  const app = watchState(page);
  const errors = watchErrors(page);
  const external = watchExternal(page);
  try {
    const { bar, debugUrl } = await begin(page, app, server);
    const view = page.getByTestId("game-view");
    await view.click();
    await page.keyboard.down("KeyA");
    await page.waitForTimeout(100);
    await page.keyboard.up("KeyA");
    await page.keyboard.press("Escape");
    const key = bar.getByRole("button", { name: /^frame \d+, A down/ });
    await expect(key).toHaveCount(1, { timeout: 10_000 });
    await key.hover();
    await expect(bar.getByRole("status")).toHaveText(/^frame \d+ · A down/);

    // The agent edits a file: the session reloads, a mark says so, and the
    // recorded history carries on.
    const before = await clock(debugUrl);
    appendFileSync(`${dir}/game.fun`, "\nlet transportProbe = 1.0\n");
    const reload = bar.getByRole("button", { name: /^frame \d+, hot reload/ });
    await expect(reload).toHaveCount(1, { timeout: 20_000 });
    const after = await clock(debugUrl);
    expect(after.program_revision).toBe(before.program_revision + 1);
    expect(after.recorded.oldest).toBe(before.recorded.oldest);
    await expect(key).toHaveCount(1);
    await reload.hover();
    await expect(bar.getByRole("status")).toHaveText(/^frame \d+ · hot reload/);
    await shot(page, "transport-marks");
    // A click on a mark seeks to it.
    const at = Number((await reload.getAttribute("aria-label")).match(/^frame (\d+)/)[1]);
    await reload.click();
    await expect.poll(async () => (await clock(debugUrl)).frame).toBe(at);
    expect(errors, "console errors").toEqual([]);
    expect(external, "non-loopback requests").toEqual([]);
    console.log("ok: transport marks");
  } finally {
    await page.close();
    await stopServer(server);
  }
}

/**
 * The mean absolute difference of two PNGs' pixels, decoded in the page
 * (0–255); within `region` (`[x0, y0, x1, y1]`, fractions of the image) when given.
 */
function difference(page, a, b, region = [0, 0, 1, 1]) {
  return page.evaluate(
    async ([a, b, [x0, y0, x1, y1]]) => {
      const pixels = async (b64) => {
        const bytes = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
        const bitmap = await createImageBitmap(new Blob([bytes], { type: "image/png" }));
        const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
        canvas.getContext("2d").drawImage(bitmap, 0, 0);
        const [x, y] = [Math.round(x0 * bitmap.width), Math.round(y0 * bitmap.height)];
        const [w, h] = [Math.round(x1 * bitmap.width) - x, Math.round(y1 * bitmap.height) - y];
        return canvas.getContext("2d").getImageData(x, y, w, h).data;
      };
      const [pa, pb] = [await pixels(a), await pixels(b)];
      if (pa.length !== pb.length) return 255;
      let sum = 0;
      for (let i = 0; i < pa.length; i++) sum += Math.abs(pa[i] - pb[i]);
      return sum / pa.length;
    },
    [a.toString("base64"), b.toString("base64"), region],
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
    await expect.poll(async () => (await now()).frame === (await shown(bar))).toBe(true);
    await page.waitForTimeout(400);
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

/** Resolves with the session's state (with its model) once `test(state)` holds. */
async function until(debugUrl, test) {
  let state;
  // Polled tightly: the jump must land before the platform's edge.
  await expect
    .poll(async () => test((state = await (await fetch(`${debugUrl}/state`)).json())), { timeout: 20_000, intervals: [5] })
    .toBe(true);
  return state;
}

/**
 * Extrapolate (D38) on examples/platformer with the jump weakened to 8.0:
 * paused before a recorded run and jump, the switch draws the path that falls
 * into the gap; an edit to 16.0 landing while paused peeks at the arc that
 * clears it, then the view goes back to the switch (off). The arc's region,
 * above the gap, changes only under the new code. Then the switch in Mini.
 */
async function extrapolate(browser) {
  if (!GPU) {
    console.log("SKIPPED: extrapolate — set FUNCTOR_APP_GPU=1 and FUNCTOR_BIN to a functor built from this checkout");
    return;
  }
  const dir = tempProject("examples/platformer");
  const file = `${dir}/game.fun`;
  const source = readFileSync(file, "utf8");
  const tuned = "let jumpVelocity = 13.0";
  if (!source.includes(tuned)) throw new Error(`game.fun has no ${tuned}`);
  writeFileSync(file, source.replace(tuned, "let jumpVelocity = 8.0"));
  const server = await startServer(0, { dir, gpu: true });
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  const app = watchState(page);
  const errors = watchErrors(page);
  const view = page.getByTestId("game-view");
  const img = view.locator("img");
  const shots = process.env.APP_UI_SHOTS;
  try {
    const { bar, debugUrl } = await begin(page, app, server);
    await expect(img).toBeVisible({ timeout: 60_000 });
    const button = bar.getByRole("button", { name: /^Extrapolate/ });
    // Run right and jump short of the left edge (x = -3): it falls in.
    await page.waitForTimeout(300);
    await view.click();
    await page.keyboard.down("ArrowRight");
    await until(debugUrl, (s) => s.model.x >= -4.6);
    await page.keyboard.press("ArrowUp");
    const takeoff = await until(debugUrl, (s) => !s.model.grounded);
    expect(takeoff.model.vy, "a jump, not a walk off the edge").toBeGreaterThan(0);
    await until(debugUrl, (s) => s.model.grounded);
    // Right stays held past where a full jump lands: the replayed release
    // would otherwise stop the new arc over the gap.
    await page.waitForTimeout(1300);
    await page.keyboard.up("ArrowRight");
    await page.keyboard.press("Escape");

    // Paused on the oldest frame, before the run: the recorded inputs are ahead.
    await bar.getByRole("button", { name: "Pause" }).click();
    await bar.getByRole("slider", { name: "Selected frame" }).focus();
    await page.keyboard.press("Home");
    await until(debugUrl, (s) => s.clock.paused && s.frame === s.clock.recorded.oldest && s.model.x === -6);
    const settle = () => page.waitForTimeout(1000);
    await settle();
    const plain = await img.screenshot();
    // Above the gap (x in [-3, 3], y in [1.6, 4.4] of the 24 × 13.5 view):
    // only an arc that clears it passes there.
    const arc = [0.4, 0.17, 0.6, 0.38];

    // On: the weak path is drawn (it falls into the gap), not the arc.
    await button.click();
    await expect(button).toHaveAttribute("aria-pressed", "true");
    await expect(bar.locator(".transport-past")).toHaveCount(1);
    await settle();
    const weak = await img.screenshot();
    const drawn = await difference(page, plain, weak);
    const weakArc = await difference(page, plain, weak, arc);
    await shot(page, "extrapolate-weak");
    // Off: back to the plain frame.
    await button.click();
    await expect(button).toHaveAttribute("aria-pressed", "false");
    await settle();
    const off = await difference(page, plain, await img.screenshot());

    // The agent's edit lands while paused: a peek shows the new path by
    // itself, the switch stays off, then the view returns to it.
    const before = await clock(debugUrl);
    writeFileSync(file, source.replace(tuned, "let jumpVelocity = 16.0"));
    await expect(button).toHaveClass(/peeking/, { timeout: 20_000 });
    expect((await clock(debugUrl)).program_revision).toBe(before.program_revision + 1);
    await expect(button).toHaveAttribute("aria-pressed", "false");
    await page.waitForTimeout(700);
    const strong = await img.screenshot();
    const strongArc = await difference(page, plain, strong, arc);
    const changed = await difference(page, weak, strong, arc);
    await shot(page, "extrapolate-peek");
    // With APP_UI_SHOTS, the rest of the peek as frames (for a GIF).
    for (let i = 0; shots && i < 12; i++) {
      await page.locator(".game-stack").screenshot({ path: `${shots}/extrapolate-loop-${String(i).padStart(2, "0")}.png` });
      await page.waitForTimeout(150);
    }
    await expect(button).not.toHaveClass(/peeking/, { timeout: 5_000 });
    await settle();
    const after = await difference(page, plain, await img.screenshot());
    console.log(
      `extrapolate: drawn ${drawn.toFixed(2)}, weak arc ${weakArc.toFixed(2)}, off ${off.toFixed(2)}, ` +
        `peek arc ${strongArc.toFixed(2)}, changed ${changed.toFixed(2)}, after ${after.toFixed(2)}`,
    );
    expect(drawn, "the weak path is drawn").toBeGreaterThan(0.3);
    expect(off, "off: the plain frame").toBeLessThan(0.3);
    expect(strongArc, "the peek draws the arc over the gap").toBeGreaterThan(4 * Math.max(weakArc, 0.5));
    expect(changed, "the path changed").toBeGreaterThan(4 * Math.max(weakArc, 0.5));
    expect(after, "the peek ends: the plain frame").toBeLessThan(0.3);
    expect((await clock(debugUrl)).paused, "still paused").toBe(true);

    // Mini: the same switch.
    await page.locator(".pane-bar").getByRole("group", { name: "Where the game shows" }).getByRole("button", { name: "Mini" }).click();
    const compact = page.getByTestId("mini").getByRole("group", { name: "Transport" });
    const miniImg = page.getByTestId("mini").locator("img");
    await expect(miniImg).toBeVisible();
    await settle();
    const miniPlain = await miniImg.screenshot();
    await compact.getByRole("button", { name: /^Extrapolate/ }).click();
    await expect(compact.getByRole("button", { name: /^Extrapolate/ })).toHaveAttribute("aria-pressed", "true");
    await settle();
    expect(await difference(page, miniPlain, await miniImg.screenshot(), arc), "the arc in Mini").toBeGreaterThan(2);
    await shot(page, "extrapolate-mini");
    expect(errors, "console errors").toEqual([]);
    console.log("ok: extrapolate (GPU)");
  } finally {
    await page.close();
    await stopServer(server);
  }
}

const browser = await chromium.launch();
try {
  await noGpu(browser);
  await marks(browser);
  await gpu(browser);
  await extrapolate(browser);
} finally {
  await browser.close();
}
