// The agent cards (D34), end to end against the fake agent: one Claude run
// walks through thinking, working (two calls at once), waiting on a question,
// a refused command (red until the next message) and done, in the dock and
// the Tasks pane's strip; then a change waiting for review, a failed run and
// a Codex run side by side. A card opens its task; nothing moves while
// nothing runs, nor under reduced motion. Runs at 1440×900 and 700×900.
//
//   npm run test:app-ui
//
// APP_UI_SHOTS=<dir> saves a still of every stance and the strip at both
// widths, and the frames of the live run at 1440×900.
import { chromium, expect } from "@playwright/test";
import {
  ROOT,
  requireBuilt,
  shot,
  startServer,
  stopServer,
  tempProject,
  watchErrors,
  watchExternal,
} from "./app.mjs";

const FIXTURES = `${ROOT}app/ui/e2e/fixtures`;
const LIVE = "Start the count higher";
const REVIEW = "Start the count at ten";
const FAILED = "Read the state";
const CODEX = "Think it over";
const PHASE = { timeout: 10_000 };

requireBuilt();

let frame = 0;

async function agentsView(browser, width, height) {
  const dir = tempProject("examples/counter");
  let server = await startServer(0, { dir, fakeAgent: `${FIXTURES}/agents.jsonl` });
  const page = await browser.newPage({ viewport: { width, height } });
  const errors = watchErrors(page);
  const external = watchExternal(page);
  const dock = page.locator(".chat");
  const input = dock.getByLabel("Message the agent");
  const heading = dock.locator("#chat-heading");
  const rail = page.getByRole("navigation", { name: "Views" });
  const pane = page.getByRole("complementary", { name: "Tasks" });
  const strip = pane.getByRole("list", { name: "Agents" });
  const card = dock.getByTestId("agent-card");
  const stance = card.getByTestId("agent-stance");
  const bubble = card.getByTestId("agent-bubble");
  const stripCard = (title) => strip.getByTestId("agent-card").filter({ hasText: title });
  /** The animation running on a card's bubble icon, `none` when still. */
  const motion = (c) => c.locator(".agent-bubble .icon").evaluate((el) => getComputedStyle(el).animationName);
  const still = async (name) => {
    await shot(page, `agent-${name}-${width}`);
    if (width === 1440) await shot(page, `agents-loop-${String(frame++).padStart(2, "0")}`);
  };
  const restart = async (fixture) => {
    await stopServer(server);
    server = await startServer(server.port, { dir, fakeAgent: `${FIXTURES}/${fixture}`, home: server.home, token: server.token });
    await page.goto("about:blank");
    await page.goto(server.url);
    await expect(page.getByRole("tab", { name: "Game" })).toBeVisible();
  };
  const ask = async (text) => {
    await rail.getByRole("button", { name: "Tasks" }).click();
    await pane.getByRole("button", { name: "New task", exact: true }).click();
    await expect(heading).toHaveText("New task");
    await expect(input).toBeFocused();
    await input.fill(text);
    await input.press("Enter");
    await expect(heading).toHaveText(text);
  };
  try {
    await page.goto(server.url);
    await expect(page.getByRole("tab", { name: "Game" })).toBeVisible();
    // No task, no card: nothing is drawn for an agent that is not there.
    await rail.getByRole("button", { name: "Tasks" }).click();
    await expect(strip).toHaveCount(0);
    await expect(card).toHaveCount(0);

    // One live run, stance by stance, in the dock and the strip.
    await ask(LIVE);
    await expect(stance).toHaveText("Thinking", PHASE);
    await expect(bubble).toHaveText("thinking");
    await expect(card.locator(".avatar-claude")).toHaveCount(1);
    await expect(stripCard(LIVE)).toHaveAttribute("data-stance", "thinking");
    expect(await motion(card)).toBe("pulse");
    await still("thinking");

    await expect(stance).toHaveText("Working", PHASE);
    await expect(bubble.locator(".agent-line")).toHaveText("reading game.fun");
    await expect(bubble.getByTestId("agent-count")).toHaveText("2");
    await expect(card).toHaveAccessibleName(`Claude on “${LIVE}”: Working. reading game.fun, 2 calls`);
    await still("working");

    await expect(stance).toHaveText("Waiting for you", PHASE);
    await expect(bubble).toHaveText("asking: Start the count at ten or twent…");
    // The loudest card, and a still one: the user's turn is not motion.
    await expect(card).toHaveCSS("border-color", "rgb(242, 176, 94)");
    expect(await motion(card)).toBe("none");
    await still("waiting");

    await expect(bubble.locator(".agent-line")).toHaveText("running curl functor.games", PHASE);
    await expect(bubble).not.toHaveClass(/agent-bubble-error/);
    // Refused: red, and it stays.
    await expect(bubble).toHaveClass(/agent-bubble-error/, PHASE);
    await expect(bubble).toHaveText("Claude requested permissions to use Bas…");
    await expect(stance).toHaveText("Thinking");
    await still("error");

    await expect(stance).toHaveText("Done", PHASE);
    await expect(bubble).toHaveClass(/agent-bubble-error/);
    // Done leaves the strip; the dock keeps the current task's card.
    await expect(stripCard(LIVE)).toHaveCount(0);
    expect(await motion(card)).toBe("none");
    await still("done");

    // A change waiting for review, a failed run, and a Codex run.
    await restart("edit.jsonl");
    await ask(REVIEW);
    await expect(stance).toHaveText("Waiting for you", { timeout: 15_000 });
    await expect(bubble).toHaveText("change ready for review");
    await restart("fail.jsonl");
    await ask(FAILED);
    await expect(stance).toHaveText("Failed", { timeout: 15_000 });
    await expect(bubble).toHaveClass(/agent-bubble-error/);
    await expect(bubble).toHaveText("No game session is running.");
    await still("failed");
    await restart("slow.jsonl");
    await dock.getByRole("button", { name: "New task", exact: true }).click();
    await dock.getByLabel("Agent", { exact: true }).selectOption("codex");
    await ask(CODEX);
    await expect(card.locator(".avatar-codex")).toHaveCount(1);
    await expect(stance).toHaveText("Thinking", PHASE);

    // The strip: the user's turn first, then the running one, then the failed.
    await rail.getByRole("button", { name: "Tasks" }).click();
    await expect(strip.getByTestId("agent-card")).toHaveCount(3);
    expect(await strip.getByTestId("agent-card").evaluateAll((cs) => cs.map((c) => c.dataset.stance))).toEqual([
      "waiting",
      "thinking",
      "failed",
    ]);
    await expect(stripCard(CODEX).locator(".avatar-codex")).toHaveCount(1);
    await expect(stripCard(REVIEW).locator(".avatar-claude")).toHaveCount(1);
    await still("strip");

    // A card opens its task, by click or by keyboard.
    await stripCard(REVIEW).click();
    await expect(heading).toHaveText(REVIEW);
    await expect(page.getByRole("tab", { name: `Task · ${REVIEW}` })).toHaveAttribute("aria-selected", "true");
    // The dock still shows the running task.
    await expect(card.locator(".avatar-codex")).toHaveCount(1);
    await stripCard(FAILED).focus();
    await page.keyboard.press("Enter");
    await expect(heading).toHaveText(FAILED);
    await card.click();
    await expect(heading).toHaveText(CODEX);

    // Reduced motion: even a running card is still.
    await page.emulateMedia({ reducedMotion: "reduce" });
    expect(await motion(stripCard(CODEX))).toBe("none");
    await page.emulateMedia({ reducedMotion: "no-preference" });

    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
    expect(overflow, "horizontal overflow (px)").toBeLessThanOrEqual(0);
    expect(errors, "console errors").toEqual([]);
    expect(external, "requests to non-loopback hosts").toEqual([]);
    console.log(`ok: agents ${width}x${height}`);
  } finally {
    await page.close();
    await stopServer(server);
  }
}

const browser = await chromium.launch();
try {
  await agentsView(browser, 1440, 900);
  await agentsView(browser, 700, 900);
} catch (e) {
  console.error(`FAIL: ${e.message}`);
  process.exitCode = 1;
} finally {
  await browser.close();
}
