// Panes in windows of their own (D20), in a browser: the pane bar's pop-out
// opens a second page (`?pane=N`) from the click; both pages show the same
// core layout; Dock back, or closing the window, returns its tabs; the Game
// tab moves between windows with one game view (one stream on a GPU
// session, FUNCTOR_APP_GPU=1). Runs at 1440×900 and 700×900.
// APP_UI_SHOTS=<dir> saves stills of both windows.
import { chromium, expect } from "@playwright/test";
import { FUNCTOR, openStreams, RELEASE_HINT, ROOT, reopen, requireBuilt, sessionInput, shot, startServer, stopServer, tabs, tempProject, watchErrors, watchState } from "./app.mjs";

requireBuilt();
const GPU = process.env.FUNCTOR_APP_GPU === "1" && FUNCTOR;

/** The tab titles of the pane a pop-out page shows. */
const windowTabs = (page) => page.locator(".pane-tabs [role=tab]").allTextContents();

async function run(browser, width, height) {
  const tag = `${width}x${height}`;
  const dir = tempProject(GPU ? "examples/primitives" : "examples/counter");
  const server = await startServer(0, { dir, gpu: !!GPU });
  const context = await browser.newContext({ viewport: { width, height } });
  const main = await context.newPage();
  const app = watchState(main);
  const errors = watchErrors(main);
  const layout = () => app.state.layouts[app.state.active];
  const bar = main.getByTestId("windows-bar");
  /** Clicks the pop-out of the pane showing `tab`; resolves with the window's page. */
  const popOut = async (tab) => {
    const pane = main.locator(".pane-box").filter({ has: main.getByRole("tab", { name: tab, exact: true }) });
    const [page] = await Promise.all([context.waitForEvent("page"), pane.getByRole("button", { name: "Pop out into its own window" }).click()]);
    await page.waitForLoadState();
    page.on("pageerror", (e) => errors.push(`window: ${e}`));
    return page;
  };
  try {
    await main.goto(server.url);
    if (FUNCTOR) await expect.poll(() => app.state?.session?.status.kind, { timeout: 60_000 }).toBe("running");
    await expect.poll(() => tabs(main)).toEqual([["Game"]]);
    // functor.json beside the game, then out to its own window.
    await main.getByRole("list", { name: "Files" }).getByRole("button", { name: "functor.json", exact: true }).click();
    await main.getByRole("button", { name: "Split right" }).first().click();
    await main.getByRole("list", { name: "Files" }).getByRole("button", { name: "game.fun", exact: true }).click();
    await expect.poll(() => tabs(main)).toEqual([["Game", "functor.json"], ["game.fun"]]);
    let win = await popOut("game.fun");
    await expect(win).toHaveTitle("game.fun — Functor");
    await expect.poll(() => windowTabs(win)).toEqual(["game.fun"]);
    await expect(win.getByRole("button", { name: "Dock back" })).toBeVisible();
    await expect(win.getByRole("button", { name: "Split right" })).toHaveCount(0);
    await expect.poll(() => tabs(main)).toEqual([["Game", "functor.json"]]);
    await expect(bar).toContainText("game.fun");
    expect(layout().windows.map((w) => w.pane.tabs.map((t) => t.path))).toEqual([["game.fun"]]);
    await shot(main, `popout-main-${width}`);
    await shot(win, `popout-window-${width}`);

    // A change in one window shows in the other: opened from the window, a
    // tab lands there; opened from the main window, a view already out
    // there is shown in it, not opened twice.
    await win.getByRole("button", { name: "Open a view here" }).click();
    await win.getByRole("menuitem", { name: "functor.json", exact: true }).click();
    await expect.poll(() => windowTabs(win)).toEqual(["game.fun", "functor.json"]);
    await expect.poll(() => tabs(main)).toEqual([["Game"]]);
    await expect(bar).toContainText("functor.json and 1 more");
    await main.getByRole("list", { name: "Files" }).getByRole("button", { name: "game.fun", exact: true }).click();
    await expect(win.getByRole("tab", { name: "game.fun", exact: true })).toHaveAttribute("aria-selected", "true");
    await expect.poll(() => tabs(main)).toEqual([["Game"]]);

    // Dock back: the tabs return beside the game, the window closes itself.
    await win.getByRole("button", { name: "Dock back" }).click();
    await win.waitForEvent("close");
    await expect.poll(() => tabs(main)).toEqual([["Game", "game.fun", "functor.json"]]);
    await expect(bar).toHaveCount(0);

    // Closing the window docks it too.
    await main.getByRole("button", { name: "Split right" }).first().click();
    const fresh = main.locator(".pane-box").nth(1);
    await fresh.getByRole("button", { name: "Open a view here" }).click();
    await main.getByRole("menuitem", { name: "functor.json", exact: true }).click();
    await expect.poll(() => tabs(main)).toEqual([["Game", "game.fun"], ["functor.json"]]);
    win = await popOut("functor.json");
    await expect.poll(() => tabs(main)).toEqual([["Game", "game.fun"]]);
    // A reopened main page keeps listing it; the window it lost track of
    // can be opened again from the list.
    await reopen(main, server.url);
    await expect(bar).toContainText("functor.json");
    await expect(bar.getByRole("button", { name: "Open window" })).toBeVisible();
    await win.close();
    const [again] = await Promise.all([context.waitForEvent("page"), bar.getByRole("button", { name: "Open window" }).click()]);
    await again.waitForLoadState();
    await expect.poll(() => windowTabs(again)).toEqual(["functor.json"]);
    await again.close();
    await expect.poll(() => tabs(main)).toEqual([["Game", "game.fun", "functor.json"]]);

    // Keys held in the game are let go when its pane pops out.
    const held = async () => (await sessionInput(app.state.session.status.debug_url)).held_keys;
    if (FUNCTOR) {
      await main.getByRole("tab", { name: "Game", exact: true }).click();
      await main.getByRole("group", { name: "Transport" }).getByRole("button", { name: "Play" }).click();
      await main.getByTestId("game-view").click();
      await main.keyboard.down("KeyW");
      await expect.poll(held).toEqual(["W"]);
    }
    // The Game tab in a window: the game view is there and only there. The
    // main window, its only pane gone, is an empty canvas.
    win = await popOut("Game");
    if (FUNCTOR) {
      await expect.poll(held).toEqual([]);
      await main.keyboard.up("KeyW");
    }
    await expect.poll(() => tabs(main)).toEqual([]);
    await expect(win).toHaveTitle(/ — Functor$/);
    await win.getByRole("tab", { name: "Game", exact: true }).click();
    await expect(win.getByTestId("game-view")).toHaveCount(1);
    await expect(main.getByTestId("game-view")).toHaveCount(0);
    // It takes keys there too, and docking lets go of them.
    if (FUNCTOR) {
      await win.getByTestId("game-view").click();
      await expect(win.getByTestId("game-view").getByRole("status")).toHaveText(RELEASE_HINT);
      await win.keyboard.down("KeyD");
      await expect.poll(held).toEqual(["D"]);
    }
    if (GPU) {
      const port = Number(new URL(app.state.session.status.debug_url).port);
      await expect(win.getByTestId("game-view").locator("img")).toBeVisible({ timeout: 30_000 });
      await expect.poll(() => openStreams(port)).toBe(1);
      await shot(win, `popout-game-${width}`);
      await win.getByRole("button", { name: "Dock back" }).click();
      await win.waitForEvent("close");
      await main.getByRole("tab", { name: "Game", exact: true }).click();
      await expect(main.getByTestId("game-view").locator("img")).toBeVisible({ timeout: 30_000 });
      await expect.poll(() => openStreams(port)).toBe(1);
    } else {
      console.log(`SKIPPED (${tag}): the stream moving between windows — set FUNCTOR_APP_GPU=1 and FUNCTOR_BIN`);
      await win.getByRole("button", { name: "Dock back" }).click();
      await win.waitForEvent("close");
    }
    await expect.poll(() => layout().windows).toEqual([]);
    if (FUNCTOR) await expect.poll(held).toEqual([]);
    expect(errors, tag).toEqual([]);
    console.log(`ok: pop-out ${tag}`);
  } finally {
    await context.close();
    await stopServer(server);
  }
}

const browser = await chromium.launch();
try {
  for (const [w, h] of [[1440, 900], [700, 900]]) await run(browser, w, h);
} finally {
  await browser.close();
}
