// The game view's placements (D21): Pane, Mini and Own window, held by the
// core per project. Mini takes the Game tab out of its pane and floats the
// same game view over the canvas; it is dragged, resized and docked back;
// panes change underneath it; a reopen and a second page show the same.
// Runs at 1440×900 and 700×900 against a temp copy of examples/counter.
//
//   npm run test:app-ui
//
// FUNCTOR_BIN (a built `functor`) adds a (headless) session, to check that
// Pane ↔ Mini keeps it and Own window restarts it. APP_UI_SHOTS=<dir> saves
// stills at both widths and the frames of a pane → mini → drag → resize →
// dock loop at 1440×900.
import { chromium, expect } from "@playwright/test";
import { FUNCTOR, reopen, requireBuilt, shot, startServer, stopServer, tabs, tempProject, watchErrors, watchExternal, watchState } from "./app.mjs";

const PROJECT = "examples/counter";

requireBuilt();

let frame = 0;
async function step(page, width) {
  if (width === 1440) await shot(page, `placements-loop-${String(frame++).padStart(2, "0")}`);
}

/** Whether box `a` lies inside box `b`. */
function inside(a, b) {
  return a.x >= b.x - 0.5 && a.y >= b.y - 0.5 && a.x + a.width <= b.x + b.width + 0.5 && a.y + a.height <= b.y + b.height + 0.5;
}

function overlaps(a, b) {
  return a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height;
}

/** Drags from the middle of `box` by (dx, dy) with the mouse. */
async function drag(page, box, dx, dy) {
  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x + dx / 2, y + dy / 2, { steps: 4 });
  await page.mouse.move(x + dx, y + dy, { steps: 4 });
  await page.mouse.up();
}

async function placements(browser, width, height) {
  const tag = `${width}x${height}`;
  const dir = tempProject(PROJECT);
  const server = await startServer(0, { dir });
  const page = await browser.newPage({ viewport: { width, height } });
  const app = watchState(page);
  const errors = watchErrors(page);
  const external = watchExternal(page);
  const game = () => app.state?.layouts[PROJECT]?.game;
  const explorer = page.getByRole("complementary", { name: "Explorer" });
  const canvas = page.getByRole("main", { name: "Canvas" });
  const control = (root) => root.getByRole("group", { name: "Where the game shows" });
  const mini = page.getByTestId("mini");
  const view = page.getByTestId("game-view");
  const strip = mini.locator(".mini-strip");
  let second;
  try {
    await page.goto(server.url);
    await expect.poll(() => tabs(page)).toEqual([["Game"]]);
    await expect(control(page).getByRole("button", { name: "Pane" })).toHaveAttribute("aria-pressed", "true");
    // A session, when there is a functor to run, shows Pane ↔ Mini keeps it.
    if (FUNCTOR) {
      await view.getByRole("button", { name: "Start game" }).click();
      await expect.poll(() => app.state?.session?.status.kind, { timeout: 60_000 }).toBe("running");
    }
    const session = app.state.session?.id ?? null;
    await explorer.getByRole("button", { name: "game.fun", exact: true }).click();
    await page.getByRole("tab", { name: "Game", exact: true }).click();
    await expect.poll(() => tabs(page)).toEqual([["Game", "game.fun"]]);
    await view.evaluate((el) => (el.dataset.mark = "the one"));
    await step(page, width);

    // Mini: the Game tab leaves its pane, and the same view floats bottom right.
    await control(page.locator(".pane-bar")).getByRole("button", { name: "Mini" }).click();
    await expect.poll(() => tabs(page)).toEqual([["game.fun"]]);
    await expect(mini).toBeVisible();
    await expect(mini.getByTestId("game-view")).toHaveAttribute("data-mark", "the one");
    await expect(view).toHaveCount(1);
    expect(game()).toMatchObject({ placement: "mini", mini: { corner: "bottom_right", width: 360 }, dock: { pane: 1, index: 0 } });
    await expect(control(mini).getByRole("button", { name: "Mini" })).toHaveAttribute("aria-pressed", "true");
    await expect(mini.getByRole("button", { name: "Dock" })).toBeVisible();
    if (FUNCTOR) await expect(mini.getByRole("button", { name: "Stop game" })).toBeVisible();
    let box = await mini.boundingBox();
    let room = await canvas.boundingBox();
    expect(inside(box, room), "mini inside the canvas").toBe(true);
    expect(box.x + box.width).toBeGreaterThan(room.x + room.width - 30);
    expect(box.y + box.height).toBeGreaterThan(room.y + room.height - 30);
    await step(page, width);
    await shot(page, `placements-mini-${width}`);

    // Files and splits underneath: the mini stays, the same view, on top.
    await explorer.getByRole("button", { name: "Open functor.json to the side" }).click();
    await expect.poll(() => tabs(page)).toEqual([["game.fun"], ["functor.json"]]);
    await page.locator(".pane-box").nth(1).getByRole("button", { name: "Split down" }).click();
    await expect.poll(() => tabs(page)).toEqual([["game.fun"], ["functor.json"], []]);
    await expect(mini.getByTestId("game-view")).toHaveAttribute("data-mark", "the one");
    box = await mini.boundingBox();
    const topmost = await page.evaluate(({ x, y }) => document.elementFromPoint(x, y)?.closest("[data-testid=mini]") !== null, {
      x: box.x + box.width / 2,
      y: box.y + box.height / 2,
    });
    expect(topmost, "mini on top of the panes").toBe(true);
    // Opening the Game is offered only while the game is in a pane.
    await page.locator(".pane-box").nth(2).getByRole("tabpanel").waitFor();
    await expect(page.locator(".pane-box").nth(2).getByRole("button", { name: "Game", exact: true })).toHaveCount(0);
    await step(page, width);

    // Dragged by its strip and let go near the top left, it snaps there.
    await canvas.evaluate((el) => el.scrollIntoView());
    room = await canvas.boundingBox();
    const grip = await strip.boundingBox();
    await drag(page, { ...grip, width: 40 }, room.x + 60 - (grip.x + 20), room.y + 60 - (grip.y + grip.height / 2));
    await expect.poll(() => game()?.mini.corner).toBe("top_left");
    box = await mini.boundingBox();
    expect(box.x - room.x).toBeLessThan(30);
    expect(box.y - room.y).toBeLessThan(30);
    await step(page, width);

    // Resized from its free corner (bottom right, now), at the game's aspect ratio.
    const handle = await mini.locator(".mini-resize").boundingBox();
    await drag(page, handle, 80, 0);
    await expect.poll(() => game()?.mini.width).toBe(440);
    box = await mini.boundingBox();
    expect(Math.abs(box.width - 440)).toBeLessThanOrEqual(1);
    const body = await mini.locator(".mini-body").boundingBox();
    // The session's surface, or 16:9 before it reports one.
    const surface = app.state.session?.status.surface;
    const aspect = surface ? surface[0] / surface[1] : 16 / 9;
    expect(Math.abs(body.width / body.height - aspect), "aspect ratio").toBeLessThan(0.05);
    await step(page, width);

    // Keyboard: the strip's handle moves it between corners and resizes it.
    await mini.getByRole("button", { name: "Move the game view" }).focus();
    await page.keyboard.press("ArrowDown");
    await expect.poll(() => game()?.mini.corner).toBe("bottom_left");
    await page.keyboard.press("-");
    await expect.poll(() => game()?.mini.width).toBe(400);
    // The strip's controls are all reachable by Tab.
    for (const name of ["Pane", "Mini", "Own window"]) {
      await page.keyboard.press("Tab");
      await expect(control(mini).getByRole("button", { name })).toBeFocused();
    }
    if (FUNCTOR) {
      await page.keyboard.press("Tab");
      await expect(mini.getByRole("button", { name: "Stop game" })).toBeFocused();
    }
    await page.keyboard.press("Tab");
    await expect(mini.getByRole("button", { name: "Dock" })).toBeFocused();

    // Click to play and Esc to release, as in a pane.
    const banner = mini.getByRole("status");
    if (FUNCTOR) {
      await expect(banner).toHaveText("Click to play");
      await mini.getByTestId("game-view").click();
      await expect(banner).toHaveText("Esc to release");
      await page.keyboard.press("Escape");
      await expect(banner).toHaveText("Click to play");
    }

    // Narrow: it covers neither the composer nor more than the canvas.
    box = await mini.boundingBox();
    room = await canvas.boundingBox();
    expect(inside(box, room), "mini inside the canvas").toBe(true);
    const composer = await page.locator(".composer-wrap").boundingBox();
    expect(overlaps(box, composer), "mini over the composer").toBe(false);
    expect(await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth), "horizontal overflow (px)").toBeLessThanOrEqual(0);

    // A reopen and a second page show the same placement and geometry, from the core.
    const before = JSON.stringify(game());
    await reopen(page, server.url);
    await expect.poll(() => JSON.stringify(game())).toBe(before);
    await expect(mini).toBeVisible();
    await expect.poll(() => tabs(page)).toEqual([["game.fun"], ["functor.json"], []]);
    second = await browser.newPage({ viewport: { width, height } });
    const secondErrors = watchErrors(second);
    await second.goto(server.url);
    await expect(second.getByTestId("mini")).toBeVisible();
    await expect(second.getByTestId("mini")).toHaveClass(/mini-bottom-left/);
    expect(Math.abs((await second.getByTestId("mini").boundingBox()).width - 400)).toBeLessThanOrEqual(1);
    expect(secondErrors, "second page console errors").toEqual([]);
    await second.close();
    second = null;

    // Dock: back to the pane and place it left from, the same view and session.
    await view.evaluate((el) => (el.dataset.mark = "the one"));
    await mini.getByRole("button", { name: "Dock" }).click();
    await expect(mini).toHaveCount(0);
    await expect.poll(() => tabs(page)).toEqual([["Game", "game.fun"], ["functor.json"], []]);
    await expect(page.getByRole("tab", { name: "Game", exact: true })).toHaveAttribute("aria-selected", "true");
    await expect(view).toHaveAttribute("data-mark", "the one");
    expect(app.state.session?.id ?? null, "the same session").toBe(session);
    await step(page, width);

    // A running game keeps its tab: no ×, and the core refuses a close.
    if (FUNCTOR) {
      await expect(page.getByRole("button", { name: "Close Game", exact: true })).toHaveCount(0);
      await page.getByRole("tab", { name: "Game", exact: true }).focus();
      await page.keyboard.press("Delete");
      await page.waitForTimeout(300);
      expect((await tabs(page))[0]).toContain("Game");
    }

    // Own window: no Game tab and no mini; one line says so, with the ways back.
    await control(page.locator(".pane-bar")).getByRole("button", { name: "Own window" }).click();
    const note = page.getByTestId("game-note");
    await expect(note).toBeVisible();
    await expect.poll(() => tabs(page)).toEqual([["game.fun"], ["functor.json"], []]);
    await expect(mini).toHaveCount(0);
    await expect(view).toHaveCount(0);
    await expect(note.getByRole("button", { name: "Bring back to a pane" })).toBeVisible();
    if (FUNCTOR) {
      await expect.poll(() => app.state.session.id, "restarted").toBe(session + 1);
      await expect(note.getByRole("button", { name: "Stop game" })).toBeVisible();
    } else {
      await expect(note.getByRole("button", { name: "Start game" })).toBeVisible();
    }
    expect(inside(await note.boundingBox(), await canvas.boundingBox())).toBe(true);
    await shot(page, `placements-window-${width}`);
    await note.getByRole("button", { name: "Bring back to a pane" }).click();
    await expect(note).toHaveCount(0);
    await expect.poll(() => tabs(page)).toEqual([["Game", "game.fun"], ["functor.json"], []]);
    if (FUNCTOR) {
      await expect.poll(() => app.state.session.id, "restarted").toBe(session + 2);
      await page.getByRole("button", { name: "Stop game" }).click();
      await expect.poll(() => app.state.session.status.kind).toBe("stopped");
    }

    expect(await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth), "horizontal overflow (px)").toBeLessThanOrEqual(0);
    expect(errors, "console errors").toEqual([]);
    expect(external, "requests to non-loopback hosts").toEqual([]);
    console.log(`ok: placements ${tag}`);
  } finally {
    await second?.close();
    await page.close();
    await stopServer(server);
  }
}

const browser = await chromium.launch();
try {
  await placements(browser, 1440, 900);
  await placements(browser, 700, 900);
} catch (e) {
  console.error(`FAIL: ${e.message}`);
  process.exitCode = 1;
} finally {
  await browser.close();
}
