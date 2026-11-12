// The agent drives the view (D25) against the fake agent, whose child calls
// the run's real tool server (`show_view`, `present`) as an agent would, so
// every request crosses the app's view route with the run's secret. Needs
// FUNCTOR_BIN (npm run build:cli:debug). Runs at 1440×900 and 700×900, plus
// a stacked canvas and the setting turned off.
//
// APP_UI_SHOTS=<dir> saves stills; APP_UI_FRAMES=<dir> saves numbered frames
// of the 1440 run for a GIF.
import { chromium, expect } from "@playwright/test";
import { mkdirSync } from "node:fs";
import {
  FUNCTOR,
  ROOT,
  reopen,
  requireBuilt,
  shot,
  startServer,
  stopServer,
  tabs,
  tempProject,
  watchErrors,
  watchExternal,
  watchState,
} from "./app.mjs";

const FIXTURE = `${ROOT}app/ui/e2e/fixtures/views.jsonl`;
const ASK = "Make the counter count by two, and show me";
const DRAFT = "Next, make the cube";

requireBuilt();
if (!FUNCTOR) {
  console.log("SKIPPED: agent views — set FUNCTOR_BIN (npm run build:cli:debug) for the real tool server");
  process.exit(0);
}

/** Saves numbered frames for the GIF (APP_UI_FRAMES), each held `hold` frames. */
function framer(page) {
  const dir = process.env.APP_UI_FRAMES;
  let n = 0;
  return async (hold = 1) => {
    if (!dir) return;
    mkdirSync(dir, { recursive: true });
    const png = await page.screenshot();
    const { writeFileSync } = await import("node:fs");
    for (let i = 0; i < hold; i++) writeFileSync(`${dir}/${String(n++).padStart(4, "0")}.png`, png);
  };
}

/** The user's pane: its id, its shown tab's title, and whether it is focused. */
function userPane(page) {
  return page.locator(".pane-box.pane-focused").evaluate((p) => ({
    pane: p.getAttribute("data-pane"),
    tab: p.querySelector("[role=tab][aria-selected=true]")?.textContent ?? null,
  }));
}

async function setUp(browser, width, height) {
  const server = await startServer(0, { dir: tempProject("examples/counter"), fakeAgent: FIXTURE });
  const page = await browser.newPage({ viewport: { width, height } });
  const app = watchState(page);
  const errors = watchErrors(page);
  const external = watchExternal(page);
  await page.goto(server.url);
  const dock = page.locator(".chat");
  const input = dock.getByLabel("Message the agent");
  await expect(input).toBeEnabled();
  return { server, page, app, errors, external, dock, input, log: dock.getByRole("log", { name: "Conversation" }) };
}

/**
 * The user reads functor.json in their pane and types in the composer while
 * the run shows game.fun: nothing of theirs moves.
 */
async function walkthrough(browser, width, height) {
  const tag = `${width}x${height}`;
  const { server, page, app, errors, external, dock, input, log } = await setUp(browser, width, height);
  const frame = framer(width === 1440 ? page : null);
  try {
    await page.getByRole("list", { name: "Files" }).getByRole("button", { name: "functor.json", exact: true }).click();
    await expect(page.getByTestId("file-view")).toHaveAttribute("aria-label", "functor.json");
    await input.fill(ASK);
    await input.press("Enter");
    await expect(log.locator(".msg-user")).toHaveText([ASK]);
    // The user goes on typing a draft, the caret mid-word.
    await input.fill(DRAFT);
    await input.evaluate((el) => el.setSelectionRange(6, 6));
    const before = { pane: await userPane(page), scroll: await page.evaluate(() => scrollY) };
    expect(before.pane.tab).toBe("functor.json");
    await frame(4);

    // Mid-run, show_view opens game.fun beside, lines 15–17 highlighted.
    const mark = page.getByTestId("code-mark");
    await expect(mark).toHaveAttribute("data-lines", "15-17", { timeout: 15_000 });
    expect(app.state.tasks[0].run, "while the run goes on").not.toBeNull();
    expect(await tabs(page)).toEqual([["Game", "functor.json"], ["game.fun"]]);
    await expect(page.getByTestId("agent-mark")).toHaveCount(1);
    await expect(log.getByTestId("step").filter({ hasText: /Showed\s*game\.fun, lines 15–17/ })).toHaveCount(1);
    await frame(6);
    // Never the wheel: focus, caret, draft, the user's pane and tab, the page's scroll.
    expect(await input.evaluate((el) => [document.activeElement === el, el.selectionStart, el.value])).toEqual([true, 6, DRAFT]);
    expect(await userPane(page)).toEqual(before.pane);
    expect(await page.evaluate(() => scrollY)).toBe(before.scroll);

    // The card arrives during the run; at its end its first step opens (line 17).
    const card = log.getByTestId("walkthrough");
    await expect(card).toBeVisible({ timeout: 15_000 });
    await expect(card.locator(".walk-title")).toHaveText(["Inc counts by two", "The change", "The counter"]);
    await expect(mark).toHaveAttribute("data-lines", "17-17", { timeout: 15_000 });
    await expect(card.locator("[aria-current=step] .walk-title")).toHaveText("Inc counts by two");
    await expect(card.locator(".walk-count")).toHaveText("Step 1 of 3");
    expect(app.state.tasks[0].run).toBeNull();
    expect(await input.evaluate((el) => [document.activeElement === el, el.selectionStart, el.value])).toEqual([true, 6, DRAFT]);
    expect(await userPane(page)).toEqual(before.pane);
    await frame(8);
    if (width === 1440) await shot(page, "agent-walkthrough-1440");
    else await shot(page, `agent-walkthrough-${width}`);

    // Stepping: the changes scrolled to game.fun, then the game; back again. The composer keeps the focus.
    const next = card.getByRole("button", { name: "Next" });
    const prev = card.getByRole("button", { name: "Prev" });
    await expect(prev).toBeDisabled();
    await next.click();
    await expect(card.locator(".walk-count")).toHaveText("Step 2 of 3");
    await expect(page.getByTestId("diff-view")).toBeVisible();
    await frame(6);
    await next.click();
    await expect(card.locator(".walk-count")).toHaveText("Step 3 of 3");
    await expect(page.locator(".pane-box.pane-focused [role=tab][aria-selected=true]")).toHaveText("Game");
    await expect(next).toBeDisabled();
    await frame(6);
    await prev.click();
    await expect(card.locator(".walk-count")).toHaveText("Step 2 of 3");
    await card.locator(".walk-step").first().click();
    await expect(mark).toHaveAttribute("data-lines", "17-17");
    await expect(card.locator(".walk-count")).toHaveText("Step 1 of 3");
    expect(await input.evaluate((el) => document.activeElement === el)).toBe(true);
    await frame(6);

    // Replayable after reopening the URL: the card, its current step, its steps.
    await reopen(page, server.url);
    await expect(card.locator(".walk-count")).toHaveText("Step 1 of 3");
    await card.getByRole("button", { name: "Next" }).click();
    await expect(card.locator(".walk-count")).toHaveText("Step 2 of 3");

    // Restore my layout: back to the user's, in one step.
    await page.getByRole("button", { name: "Restore my layout" }).click();
    await expect.poll(() => tabs(page)).toEqual([["Game", "functor.json"]]);
    await expect(page.getByTestId("agent-bar")).toHaveCount(0);
    await frame(10);

    expect(errors, `console errors (${tag})`).toEqual([]);
    expect(external, `non-loopback requests (${tag})`).toEqual([]);
    console.log(`ok: agent views ${tag}`);
  } finally {
    await page.close();
    await stopServer(server);
  }
}

/** A canvas too narrow to split: the agent's file goes behind the user's tab. */
async function stacked(browser) {
  const { server, page, errors, input, log } = await setUp(browser, 540, 900);
  try {
    await page.getByRole("list", { name: "Files" }).getByRole("button", { name: "functor.json", exact: true }).click();
    await input.fill(ASK);
    await input.press("Enter");
    await expect(log.getByTestId("step").filter({ hasText: /Showed\s*game\.fun/ })).toHaveCount(1, { timeout: 15_000 });
    await expect.poll(() => tabs(page)).toEqual([["Game", "functor.json", "game.fun"]]);
    await expect(page.locator(".pane-box [role=tab][aria-selected=true]")).toHaveText("functor.json");
    await expect(log.getByTestId("walkthrough")).toBeVisible({ timeout: 15_000 });
    await expect(log.getByTestId("result")).toHaveText(["Done"], { timeout: 15_000 });
    // The end-of-run step goes behind too: the user's tab stays shown.
    await expect(page.locator(".pane-box [role=tab][aria-selected=true]")).toHaveText("functor.json");
    expect(errors, "console errors (stacked)").toEqual([]);
    console.log("ok: agent views stacked");
  } finally {
    await page.close();
    await stopServer(server);
  }
}

/** "Let the agent arrange views" off: the card only, nothing moves, nothing auto-opens. */
async function settingOff(browser) {
  const { server, page, app, errors, input, log } = await setUp(browser, 1440, 900);
  try {
    await page.getByRole("button", { name: "Open a view here" }).click();
    await page.getByRole("menuitemcheckbox", { name: "Let the agent arrange views" }).click();
    await expect.poll(() => app.state?.layouts["examples/counter"].agent.arranges).toBe(false);
    await input.fill(ASK);
    await input.press("Enter");
    const card = log.getByTestId("walkthrough");
    await expect(card).toBeVisible({ timeout: 15_000 });
    await expect(log.getByTestId("result")).toHaveText(["Done"], { timeout: 15_000 });
    expect(await tabs(page)).toEqual([["Game"]]);
    await expect(card.locator(".walk-count")).toHaveText("3 steps");
    await expect(card.getByRole("button", { name: "Start" })).toBeEnabled();
    await expect(page.getByTestId("agent-bar")).toHaveCount(0);
    expect(errors, "console errors (off)").toEqual([]);
    console.log("ok: agent views off");
  } finally {
    await page.close();
    await stopServer(server);
  }
}

const browser = await chromium.launch();
try {
  await walkthrough(browser, 1440, 900);
  await walkthrough(browser, 700, 900);
  await stacked(browser);
  await settingOff(browser);
} catch (e) {
  console.error(`FAIL: ${e.message}`);
  process.exitCode = 1;
} finally {
  await browser.close();
}
