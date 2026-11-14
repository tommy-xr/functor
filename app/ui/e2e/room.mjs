// The agent card's context bar and subagents (D34), end to end against the
// fake agent. A model's first run shows no bar until it ends (the window
// comes only then); a subagent's call is a smaller figure under its parent,
// not a second call; a command that failed is not red. After a restart the
// window is still known: the next run's bar shows at once, amber at 70%,
// red at 90%. Runs at 1440×900 and 700×900.
//
//   npm run test:app-ui
//
// APP_UI_SHOTS=<dir> saves stills of the subagent, the bar after the first
// run, and the amber and red bars, at both widths.
import { chromium, expect } from "@playwright/test";
import { ROOT, reopen, requireBuilt, shot, startServer, stopServer, tempProject, watchErrors, watchExternal } from "./app.mjs";

const FIXTURES = `${ROOT}app/ui/e2e/fixtures`;
const FIRST = "Where does the count start";
const AGAIN = "Read it all again";
const PHASE = { timeout: 10_000 };

requireBuilt();

async function roomView(browser, width, height) {
  const dir = tempProject("examples/counter");
  let server = await startServer(0, { dir, fakeAgent: `${FIXTURES}/room-first.jsonl` });
  const page = await browser.newPage({ viewport: { width, height } });
  const errors = watchErrors(page);
  const external = watchExternal(page);
  const dock = page.locator(".chat");
  const input = dock.getByLabel("Message the agent");
  const heading = dock.locator("#chat-heading");
  const rail = page.getByRole("navigation", { name: "Views" });
  const pane = page.getByRole("complementary", { name: "Tasks" });
  const card = dock.getByTestId("agent-card");
  const stance = card.getByTestId("agent-stance");
  const bubble = card.getByTestId("agent-bubble");
  const sub = card.getByTestId("agent-sub");
  const bar = card.getByTestId("agent-room");
  const ask = async (text) => {
    await rail.getByRole("button", { name: "Tasks" }).click();
    await pane.getByRole("button", { name: "New task", exact: true }).click();
    await expect(heading).toHaveText("New task");
    await input.fill(text);
    await input.press("Enter");
    await expect(heading).toHaveText(text);
  };
  try {
    await page.goto(server.url);
    await expect(page.getByRole("tab", { name: "Game" })).toBeVisible();

    // First run of the model: its usage is known, its window is not.
    await ask(FIRST);
    await expect(stance).toHaveText("Thinking", PHASE);
    await expect(bar).toHaveCount(0);

    // The subagent: its own figure and bubble, under the call that started it.
    await expect(sub.locator(".agent-line")).toHaveText("reading game.fun", PHASE);
    await expect(sub.locator(".avatar-claude")).toHaveCount(1);
    await expect(bubble.locator(".agent-line")).toHaveText("subagent: Find the spawn code");
    // One call of the main agent's, not two.
    await expect(bubble.getByTestId("agent-count")).toHaveCount(0);
    await expect(card).toHaveAccessibleName(
      `Claude on “${FIRST}”: Working. subagent: Find the spawn code; subagent “Find the spawn code”: reading game.fun`,
    );
    await expect(bar).toHaveCount(0);
    await shot(page, `agent-subagent-${width}`);
    // Its hand-back ends it.
    await expect(sub).toHaveCount(0, PHASE);

    // A command that failed and was worked past: flagged, but not red.
    await expect(bubble.locator(".agent-line")).toHaveText("running cargo test", PHASE);
    await expect(stance).toHaveText("Thinking", PHASE);
    await expect(bubble).not.toHaveClass(/agent-bubble-error/);

    // The run ends with the window: the bar, at the last prompt's 30%.
    await expect(stance).toHaveText("Done", PHASE);
    await expect(bar).toHaveAttribute("data-level", "ok");
    await expect(bar).toHaveText("30%");
    await expect(bar).toHaveAttribute("title", "Context 30% full: 300k of 1M tokens");
    await expect(bubble).not.toHaveClass(/agent-bubble-error/);
    await shot(page, `agent-room-${width}`);

    // A restart: the window is remembered, so the next run's bar shows at once.
    await stopServer(server);
    server = await startServer(server.port, { dir, fakeAgent: `${FIXTURES}/room-filling.jsonl`, home: server.home, token: server.token });
    await reopen(page, server.url);
    await expect(page.getByRole("tab", { name: "Game" })).toBeVisible();
    await ask(AGAIN);
    await expect(bar).toHaveAttribute("data-level", "low", PHASE);
    await expect(bar).toHaveText("75%");
    await expect(bar.locator(".agent-room-fill")).toHaveCSS("background-color", "rgb(242, 176, 94)");
    await shot(page, `agent-room-low-${width}`);
    await expect(bar).toHaveAttribute("data-level", "out", PHASE);
    await expect(bar).toHaveText("92%");
    await expect(bar.locator(".agent-room-fill")).toHaveCSS("background-color", "rgb(242, 139, 130)");
    await expect(card).toHaveAccessibleName(/; context 92% full$/);
    await shot(page, `agent-room-out-${width}`);
    // The run ends; the bar keeps its last reading.
    await expect(stance).toHaveText("Done", PHASE);
    await expect(bar).toHaveText("92%");

    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
    expect(overflow, "horizontal overflow (px)").toBeLessThanOrEqual(0);
    expect(errors, "console errors").toEqual([]);
    expect(external, "requests to non-loopback hosts").toEqual([]);
    console.log(`ok: room ${width}x${height}`);
  } finally {
    await page.close();
    await stopServer(server);
  }
}

const browser = await chromium.launch();
try {
  await roomView(browser, 1440, 900);
  await roomView(browser, 700, 900);
} catch (e) {
  console.error(`FAIL: ${e.message}`);
  process.exitCode = 1;
} finally {
  await browser.close();
}
