// The canvas as panes of tabs (the layout is the core's state): opening
// files from the explorer, to the side, dragging a tab between panes,
// splitting, resizing, closing, keyboard shortcuts, and the layout surviving
// a reload and showing the same in a second page. Runs at 1440×900 and
// 700×900 against a temp copy of examples/counter.
//
//   npm run test:app-ui
//
// APP_UI_SHOTS=<dir> saves stills at both widths and the frames of an
// open → side → move → split → view-diff loop at 1440×900.
import { chromium, expect } from "@playwright/test";
import { reopen, requireBuilt, tabs, ROOT, shot, startServer, stopServer, tempProject, watchErrors, watchExternal, watchState } from "./app.mjs";

const EDIT = `${ROOT}app/ui/e2e/fixtures/edit.jsonl`;
const PROJECT = "examples/counter";

requireBuilt();

let frame = 0;
async function step(page, width) {
  if (width === 1440) await shot(page, `panes-loop-${String(frame++).padStart(2, "0")}`);
}

/** The selected tab's name in each pane. */
function shown(page) {
  return page.locator(".pane-box").evaluateAll((panes) =>
    panes.map((p) => p.querySelector("[role=tab][aria-selected=true]")?.textContent ?? null),
  );
}

async function panes(browser, width, height) {
  const tag = `${width}x${height}`;
  const dir = tempProject(PROJECT);
  const server = await startServer(0, { dir, fakeAgent: EDIT });
  const page = await browser.newPage({ viewport: { width, height } });
  const app = watchState(page);
  const errors = watchErrors(page);
  const external = watchExternal(page);
  const layout = () => app.state?.layouts[PROJECT];
  const explorer = page.getByRole("complementary", { name: "Explorer" });
  const pane = (i) => page.locator(".pane-box").nth(i);
  const composer = page.getByLabel("Message the agent");
  // A drag scrolling the page midway loses it: at 700 px the canvas is brought into view first.
  const toCanvas = () => page.locator(".canvas").evaluate((el) => el.scrollIntoView());
  let second;
  try {
    await page.goto(server.url);

    // The default: one pane with the Game tab.
    await expect.poll(() => tabs(page)).toEqual([["Game"]]);
    await expect(page.getByRole("tab", { name: "Game", exact: true })).toHaveAttribute("aria-selected", "true");
    await expect(page.getByTestId("game-view")).toHaveText(/No game running/);
    await shot(page, `panes-default-${width}`);
    await step(page, width);

    // A click on a file opens it in the focused pane, read only, line-numbered.
    await explorer.getByRole("button", { name: "game.fun", exact: true }).click();
    await expect.poll(() => tabs(page)).toEqual([["Game", "game.fun"]]);
    const code = page.getByTestId("file-view");
    await expect(code.locator(".code-text")).toContainText("let init = { count: 0.0 }");
    await expect(code.locator(".code-gutter")).toContainText(/^1\n2\n3\n/);
    await step(page, width);

    // To the side: a split right, the file in the new pane.
    await explorer.getByRole("button", { name: "Open functor.json to the side" }).click();
    await expect.poll(() => tabs(page)).toEqual([["Game", "game.fun"], ["functor.json"]]);
    await expect(pane(1).getByTestId("file-view")).toContainText('"language": "functor-lang"');
    await step(page, width);

    // A tab dragged onto another pane's tab moves there, after it.
    await toCanvas();
    const target = pane(1).getByRole("tab", { name: "functor.json" });
    const box = await target.boundingBox();
    await pane(0).getByRole("tab", { name: "game.fun" }).dragTo(target, {
      targetPosition: { x: box.width - 4, y: box.height / 2 },
    });
    await expect.poll(() => tabs(page)).toEqual([["Game"], ["functor.json", "game.fun"]]);
    await expect.poll(() => shown(page)).toEqual(["Game", "game.fun"]);
    await step(page, width);

    // Split down: an empty, focused pane under the second, offering views.
    await pane(1).getByRole("button", { name: "Split down" }).click();
    await expect.poll(() => tabs(page)).toEqual([["Game"], ["functor.json", "game.fun"], []]);
    await expect(pane(2)).toHaveClass(/pane-focused/);
    await expect(pane(2).getByRole("tabpanel")).toContainText("Open a view here");
    // The game view is the one element, moved rather than mounted again.
    await page.getByTestId("game-view").evaluate((el) => (el.dataset.mark = "the one"));
    await toCanvas();
    await page.getByRole("tab", { name: "Game", exact: true }).dragTo(pane(2).locator(".pane-body"));
    // Its pane, left empty, is gone; its sibling took the space.
    await expect.poll(() => tabs(page)).toEqual([["functor.json", "game.fun"], ["Game"]]);
    await expect(page.getByTestId("game-view")).toHaveCount(1);
    await expect(page.getByTestId("game-view")).toHaveAttribute("data-mark", "the one");
    await step(page, width);

    // The splitter: arrow keys step it, a drag sets it; the core holds the ratio.
    const splitter = page.getByRole("separator", { name: "Resize panes" });
    await expect(splitter).toHaveAttribute("aria-orientation", "horizontal");
    await splitter.focus();
    await page.keyboard.press("ArrowDown");
    await page.keyboard.press("ArrowDown");
    await expect.poll(() => layout()?.root?.ratio).toBe(600);
    await expect(splitter).toHaveAttribute("aria-valuenow", "60");
    await toCanvas();
    const s = await splitter.boundingBox();
    const split = await page.locator(".split").first().boundingBox();
    await page.mouse.move(s.x + s.width / 2, s.y + s.height / 2);
    await page.mouse.down();
    await page.mouse.move(s.x + s.width / 2, split.y + split.height * 0.3, { steps: 5 });
    await page.mouse.up();
    await expect.poll(() => layout()?.root?.ratio).toBeGreaterThanOrEqual(280);
    await expect.poll(() => layout()?.root?.ratio).toBeLessThanOrEqual(320);

    // A click in a pane focuses it, the game view (portalled in) included.
    await pane(0).getByTestId("file-view").click();
    await expect(pane(0)).toHaveClass(/pane-focused/);
    // In a corner, clear of its Start game: a running game would keep its tab below.
    await page.getByTestId("game-view").click({ position: { x: 8, y: 8 } });
    await expect(pane(1)).toHaveClass(/pane-focused/);

    // A reload shows exactly this layout, from the core.
    const before = JSON.stringify(layout());
    await reopen(page, server.url);
    await expect.poll(() => JSON.stringify(layout())).toBe(before);
    await expect.poll(() => tabs(page)).toEqual([["functor.json", "game.fun"], ["Game"]]);
    await expect.poll(() => shown(page)).toEqual(["game.fun", "Game"]);

    // A second page shows the same and follows a change made in the first.
    second = await browser.newPage({ viewport: { width, height } });
    const secondErrors = watchErrors(second);
    await second.goto(server.url);
    await expect.poll(() => tabs(second)).toEqual([["functor.json", "game.fun"], ["Game"]]);
    await pane(0).getByRole("tab", { name: "functor.json" }).click();
    await expect.poll(() => shown(second)).toEqual(["functor.json", "Game"]);
    await pane(1).getByRole("button", { name: "Split right" }).click();
    await expect.poll(() => tabs(second)).toEqual([["functor.json", "game.fun"], ["Game"], []]);
    expect(secondErrors, "second page console errors").toEqual([]);
    await second.close();
    second = null;

    // Shortcuts act on the focused pane: Alt+W closes its tab (here, the empty pane).
    await expect.poll(() => tabs(page)).toEqual([["functor.json", "game.fun"], ["Game"], []]);
    const tab = pane(0).getByRole("tab", { name: "functor.json" });
    await page.keyboard.press("Alt+KeyW");
    await expect.poll(() => tabs(page)).toEqual([["functor.json", "game.fun"], ["Game"]]);
    // Roving tabs: arrows move and show; Alt+] and Alt+[ cycle; Alt+O moves to the next pane.
    await tab.focus();
    await page.keyboard.press("ArrowRight");
    await expect(pane(0).getByRole("tab", { name: "game.fun" })).toBeFocused();
    await expect.poll(() => shown(page)).toEqual(["game.fun", "Game"]);
    await page.keyboard.press("Alt+BracketRight");
    await expect.poll(() => shown(page)).toEqual(["functor.json", "Game"]);
    await page.keyboard.press("Alt+BracketLeft");
    await expect.poll(() => shown(page)).toEqual(["game.fun", "Game"]);
    await page.keyboard.press("Alt+KeyO");
    await expect(pane(1)).toHaveClass(/pane-focused/);
    await expect(page.getByRole("tab", { name: "Game", exact: true })).toBeFocused();
    await page.keyboard.press("Alt+Backslash");
    await expect.poll(() => tabs(page)).toEqual([["functor.json", "game.fun"], ["Game"], []]);
    await page.keyboard.press("Alt+KeyW");
    await expect.poll(() => tabs(page)).toEqual([["functor.json", "game.fun"], ["Game"]]);
    // Inert while typing in the composer.
    await composer.click();
    await page.keyboard.press("Alt+Backslash");
    await page.keyboard.press("Alt+KeyW");
    await page.waitForTimeout(300);
    expect(await tabs(page)).toEqual([["functor.json", "game.fun"], ["Game"]]);
    await composer.fill("");

    // "View diff" from the chat opens the Changes tab beside, the composer keeping the keyboard.
    await composer.fill("Start the count at ten");
    await composer.press("Enter");
    await expect(page.getByTestId("task-status")).toHaveText("Ready for review", { timeout: 15_000 });
    await composer.fill("a draft");
    await page.locator(".chat").getByTestId("change-card").getByRole("button", { name: "View diff" }).click();
    const changes = page.getByRole("tab", { name: "Changes · Start the count at ten" });
    await expect(changes).toHaveAttribute("aria-selected", "true");
    await expect(composer).toBeFocused();
    await expect(composer).toHaveValue("a draft");
    // It went beside the focused pane (Game's), not over it.
    await expect.poll(() => shown(page)).toEqual(["Changes · Start the count at ten", "Game"]);
    await expect(page.getByTestId("diff-view").locator(".diff-file")).toBeVisible();
    await composer.fill("");
    await toCanvas();
    await step(page, width);
    await shot(page, `panes-${width}`);

    // Closing tabs until a pane is gone: × and a middle-click.
    await pane(0).getByRole("button", { name: "Close functor.json" }).click();
    await pane(0).getByRole("tab", { name: "game.fun" }).click({ button: "middle" });
    await expect.poll(() => tabs(page)).toEqual([["Changes · Start the count at ten"], ["Game"]]);
    await pane(0).getByRole("button", { name: /^Close Changes/ }).click();
    await expect.poll(() => tabs(page)).toEqual([["Game"]]);
    // The last tab closed: the empty canvas offers its three ways in.
    await page.getByRole("button", { name: "Close Game" }).click();
    await expect(page.locator(".pane-box")).toHaveCount(0);
    const empty = page.locator(".empty-canvas");
    for (const name of ["Start game", "Open a file", "New task"]) await expect(empty.getByRole("button", { name })).toBeVisible();
    await empty.getByRole("button", { name: "Open a file" }).click();
    await empty.getByRole("button", { name: "game.fun" }).click();
    await expect.poll(() => tabs(page)).toEqual([["game.fun"]]);
    // Reset layout, from the open-view menu.
    await page.getByRole("button", { name: "Open a view here" }).click();
    await page.getByRole("menuitem", { name: "Reset layout" }).click();
    await expect.poll(() => tabs(page)).toEqual([["Game"]]);
    await expect(page.getByRole("button", { name: /^Pop out/ })).toHaveAttribute("aria-disabled", "true");

    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
    expect(overflow, "horizontal overflow (px)").toBeLessThanOrEqual(0);
    expect(errors, "console errors").toEqual([]);
    expect(external, "requests to non-loopback hosts").toEqual([]);
    console.log(`ok: panes ${tag}`);
  } finally {
    await second?.close();
    await page.close();
    await stopServer(server);
  }
}

const browser = await chromium.launch();
try {
  if (!process.env.ONLY700) await panes(browser, 1440, 900);
  await panes(browser, 700, 900);
} catch (e) {
  console.error(`FAIL: ${e.message}`);
  process.exitCode = 1;
} finally {
  await browser.close();
}
