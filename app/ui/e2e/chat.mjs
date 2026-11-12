// The chat dock against the fake agent (`FUNCTOR_APP_FAKE_AGENT`), which
// replays a recorded run with its timing: a tool call, a capture, a read, an
// edit, then a streamed Markdown reply. Runs at 1440×900 and 700×900.
//
//   npm run test:app-ui     # builds the UI and functor-app, then the shell and chat tests
//
// APP_UI_SHOTS=<dir> saves the empty state and a finished conversation.
import { chromium, expect } from "@playwright/test";
import {
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

const SHOTS = process.env.APP_UI_SHOTS;
// A copy in a throwaway repo: a run snapshots its repo's working tree.
const PROJECT = tempProject("examples/counter");
const CHAT = `${ROOT}app/ui/e2e/fixtures/chat.jsonl`;
const FAIL = `${ROOT}app/ui/e2e/fixtures/fail.jsonl`;
const FIRST = "What frame is the game on?";

requireBuilt();


/** The kinds of the conversation's rows, in order: user, step, capture, change, agent, result, line. */
function rows(log) {
  return log.evaluate((el) =>
    [...el.children].map((c) => {
      const id = c.getAttribute("data-testid");
      if (id === "step") return "step";
      if (id === "capture-card") return "capture";
      if (id === "change-card") return "change";
      if (c.classList.contains("msg-user")) return "user";
      if (c.classList.contains("msg-agent")) return "agent";
      if (c.classList.contains("outcome")) return "result";
      if (c.classList.contains("conn-line")) return "line";
      return c.className;
    }),
  );
}

async function conversation(browser, width, height) {
  const tag = `${width}x${height}`;
  let server = await startServer(0, { dir: PROJECT, fakeAgent: CHAT });
  const page = await browser.newPage({ viewport: { width, height } });
  const app = watchState(page);
  const errors = watchErrors(page);
  const external = watchExternal(page);
  const dock = page.locator(".chat");
  const log = dock.getByRole("log", { name: "Conversation" });
  const input = dock.getByLabel("Message the agent");
  const send = dock.getByRole("button", { name: "Send" });
  const stop = dock.getByRole("button", { name: "Stop", exact: true });
  const heading = dock.locator("#chat-heading");
  const badge = dock.getByTestId("task-status");
  const results = log.getByTestId("result");
  const notice = dock.getByTestId("chat-notice");
  const idle = () => expect.poll(() => app.state?.tasks.every((t) => !t.run), { timeout: 15_000 }).toBe(true);
  try {
    await page.goto(server.url);

    // The empty state: what the agent will see, the inert attach and mic, the agent picker.
    await expect(heading).toHaveText("New task");
    await expect(log).toContainText("Ask the agent for a change");
    await expect(dock.getByRole("list", { name: "What the agent will see" }).getByRole("listitem")).toHaveText([
      "examples/counter",
      "No game running",
      "Game tab",
    ]);
    await expect(send).toBeDisabled();
    for (const name of [/^Attach/, /^Hold to talk/]) {
      await expect(dock.getByRole("button", { name })).toHaveAttribute("aria-disabled", "true");
    }
    const picker = dock.getByLabel("Agent", { exact: true });
    await expect(picker.locator("option")).toHaveText(["claude", "codex"]);
    // The pick is the server's, per project: it survives a reload.
    await picker.selectOption("codex");
    await expect.poll(() => app.state?.harness_choice["examples/counter"]).toBe("codex");
    await page.reload();
    await expect(picker).toHaveValue("codex");
    await picker.selectOption("claude");
    await expect.poll(() => app.state?.harness_choice["examples/counter"]).toBe("claude");
    if (width === 1440) await shot(page, "chat-empty-1440");

    // Sending from the empty state starts a task: bubble, steps, cards, streamed text, result.
    await input.fill(FIRST);
    await input.press("Enter");
    await expect(log.locator(".msg-user")).toHaveText([FIRST]);
    await expect(input).toHaveValue("");
    await expect(heading).toHaveText(FIRST);
    await expect(badge).toHaveText("In progress");
    await expect(stop).toBeVisible();
    await expect(send).toHaveCount(0);
    // A message while the run is going is refused, and stays in the composer.
    await input.fill("And the count?");
    await input.press("Enter");
    await expect(notice).toContainText("still running");
    await expect(input).toHaveValue("And the count?");
    await expect(log.getByTestId("step").first()).toContainText("Read the game state");
    // Streamed: the reply shows while it is still arriving.
    await expect(log.locator(".msg-agent[aria-busy=true]")).toContainText("The game is on");
    await expect(results).toHaveText(["Done"], { timeout: 15_000 });
    await expect(log.locator(".msg-agent")).toContainText("frame 1842");
    expect(await rows(log)).toEqual(["user", "step", "capture", "step", "change", "agent", "result"]);
    await expect(log.locator(".usage")).toHaveText("24.1k tokens in · 410 out");
    await expect(log.getByRole("link", { name: "language guide" })).toHaveAttribute("rel", "noopener noreferrer");
    await expect(badge).toHaveText("Waiting on you");
    await expect(send).toBeEnabled();
    // Steps expand to their input and result; long results are cut until asked.
    const read = log.getByRole("button", { name: /^Read game\.fun/ });
    await read.click();
    await expect(read).toHaveAttribute("aria-expanded", "true");
    await expect(log.locator(".clipped pre").last()).toContainText("// line 10");
    await expect(log.locator(".clipped pre").last()).not.toContainText("// line 40");
    await log.getByRole("button", { name: "Show more" }).click();
    await expect(log.locator(".clipped pre").last()).toContainText("// line 40");
    await read.click();

    // The kept text is a follow-up in the same task; a reload mid-run restores it.
    await input.press("Enter");
    await expect(log.locator(".msg-user")).toHaveText([FIRST, "And the count?"]);
    await expect(log.getByTestId("step")).toHaveCount(4);
    await page.reload();
    await expect(heading).toHaveText(FIRST);
    await expect(log.locator(".msg-user")).toHaveText([FIRST, "And the count?"]);
    await expect(results).toHaveText(["Done", "Done"], { timeout: 15_000 });
    await expect(log.locator(".msg-agent")).toHaveCount(2);

    // Stop ends the run with a stopped result.
    await input.fill("Stop this one");
    await input.press("Enter");
    await expect(log.getByTestId("step")).toHaveCount(5);
    await stop.click();
    await expect(results).toHaveText(["Done", "Done", "Stopped"]);
    await expect(badge).toHaveText("Waiting on you");
    await idle();

    // Cards switch the canvas.
    await log.getByRole("button", { name: "View diff" }).first().click();
    await expect(page.getByRole("tab", { name: "Diff" })).toHaveAttribute("aria-selected", "true");
    await expect(dock.getByRole("listitem").filter({ hasText: "Diff tab" })).toHaveCount(1);
    await log.getByRole("button", { name: "Open in canvas" }).first().click();
    await expect(page.getByRole("tab", { name: "Game" })).toHaveAttribute("aria-selected", "true");

    // Scrolled up, the log stops following and offers the way back.
    const scroll = () => log.evaluate((el) => ({ top: el.scrollTop, max: el.scrollHeight - el.clientHeight }));
    expect((await scroll()).max, "the conversation overflows").toBeGreaterThan(100);
    await log.evaluate((el) => el.scrollTo(0, 0));
    const jump = dock.getByRole("button", { name: "Jump to latest" });
    await expect(jump).toBeVisible();
    await input.fill("Once more");
    await input.press("Enter");
    await expect(results).toHaveCount(4, { timeout: 15_000 });
    expect((await scroll()).top, "stayed where the reader was").toBe(0);
    await jump.click();
    await expect.poll(async () => (await scroll()).max - (await scroll()).top).toBeLessThanOrEqual(1);
    await expect(jump).toHaveCount(0);
    if (SHOTS) await shot(page, `chat-${width}`);

    // A restarted server: connection lines, and the transcript fetched again.
    await stopServer(server);
    await expect(log.locator(".conn-line")).toHaveText(["Connection lost, reconnecting…"]);
    server = await startServer(server.port, { dir: PROJECT, fakeAgent: CHAT, home: server.home });
    await expect(log.locator(".conn-line")).toHaveText(["Connection lost, reconnecting…", "Reconnected"], {
      timeout: 15_000,
    });
    await expect(results).toHaveCount(4);

    // A new task, then switching between the two from the recent list.
    await dock.getByRole("button", { name: "New task" }).click();
    await expect(heading).toHaveText("New task");
    await input.fill("Make the count go up by two");
    await input.press("Enter");
    await expect(heading).toHaveText("Make the count go up by two");
    // One run per project: the first task cannot run alongside it.
    await dock.getByRole("button", { name: "Recent tasks" }).click();
    const menu = dock.getByRole("menu", { name: "Recent tasks" });
    await expect(menu.getByRole("menuitem")).toHaveCount(2);
    await menu.getByRole("menuitem", { name: new RegExp(FIRST.replace("?", "\\?")) }).click();
    await expect(heading).toHaveText(FIRST);
    await input.fill("Meanwhile?");
    await input.press("Enter");
    await expect(notice).toContainText("one run per project");
    await expect(input).toHaveValue("Meanwhile?");
    await idle();
    await dock.getByRole("button", { name: "Recent tasks" }).click();
    await menu.getByRole("menuitem", { name: /Make the count/ }).click();
    await expect(heading).toHaveText("Make the count go up by two");
    await expect(log.locator(".msg-user")).toHaveText(["Make the count go up by two"]);
    await expect(results).toHaveText(["Done"]);

    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
    expect(overflow, "horizontal overflow (px)").toBeLessThanOrEqual(0);
    expect(errors, "console errors").toEqual([]);
    expect(external, "requests to non-loopback hosts").toEqual([]);
    console.log(`ok: chat ${tag}`);
  } finally {
    await page.close();
    await stopServer(server);
  }
}

/** A failed run shows its summary's first line; with FUNCTOR_BIN, session changes show as lines. */
async function failure(browser) {
  const server = await startServer(0, { dir: PROJECT, fakeAgent: FAIL });
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  const errors = watchErrors(page);
  const dock = page.locator(".chat");
  try {
    await page.goto(server.url);
    const input = dock.getByLabel("Message the agent");
    await input.fill("What is the count?");
    await input.press("Enter");
    await expect(dock.getByTestId("result")).toHaveText("Failed: No game session is running.");
    await expect(dock.getByTestId("task-status")).toHaveText("Failed");
    if (FUNCTOR) {
      const lines = dock.locator(".conn-line");
      await page.getByTestId("game-view").getByRole("button", { name: "Start game" }).click();
      await expect(lines).toHaveText(["Game session started"], { timeout: 60_000 });
      await expect(dock.getByRole("listitem").filter({ hasText: "Game running" })).toHaveCount(1);
      await page.getByRole("button", { name: "Stop game" }).click();
      await expect(lines).toHaveText(["Game session started", "Game session stopped"]);
    } else {
      console.log("SKIPPED: session lines — set FUNCTOR_BIN (npm run build:cli:debug)");
    }
    expect(errors, "console errors").toEqual([]);
    console.log("ok: chat failure");
  } finally {
    await page.close();
    await stopServer(server);
  }
}

const browser = await chromium.launch();
try {
  await conversation(browser, 1440, 900);
  await conversation(browser, 700, 900);
  await failure(browser);
} catch (e) {
  console.error(`FAIL: ${e.message}`);
  process.exitCode = 1;
} finally {
  await browser.close();
}
