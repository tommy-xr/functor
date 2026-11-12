// The Tasks pane and the task page, end to end against the fake agent in a
// throwaway git repo. Four tasks, each from its own fixture (the app is
// restarted on the same home between them): one kept, one ready for review,
// one failed and one still running. They land in their groups with the
// rail's badge; a row is the chat's current task and opens its Task tab;
// the page's Keep moves its task to Done; rename, delete (refused while
// running), search, keyboard and a reopened URL. Runs at 1440×900 and 700×900.
//
//   npm run test:app-ui
//
// APP_UI_SHOTS=<dir> saves stills of the pane and the page at both widths,
// and the frames of a select → review → keep loop at 1440×900.
import { existsSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { chromium, expect } from "@playwright/test";
import {
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

const FIXTURES = `${ROOT}app/ui/e2e/fixtures`;
const KEPT = "Start the count at ten";
const READY = "Label the counter";
const FAILED = "Read the state";
const RUNNING = "Think for a while";

requireBuilt();

let frame = 0;
/** A numbered frame of the GIF loop (1440 only). */
async function step(page, width) {
  if (width === 1440) await shot(page, `tasks-loop-${String(frame++).padStart(2, "0")}`);
}

/** Every task file under the app's home, as `<id>.json`. */
function taskFiles(home) {
  const projects = join(home, "projects");
  return readdirSync(projects).flatMap((p) => {
    const dir = join(projects, p, "tasks");
    return existsSync(dir) ? readdirSync(dir).filter((f) => f.endsWith(".json")) : [];
  });
}

async function tasksView(browser, width, height) {
  const dir = tempProject("examples/counter");
  let server = await startServer(0, { dir, fakeAgent: `${FIXTURES}/edit.jsonl` });
  const page = await browser.newPage({ viewport: { width, height } });
  const app = watchState(page);
  const errors = watchErrors(page);
  const external = watchExternal(page);
  const dock = page.locator(".chat");
  const input = dock.getByLabel("Message the agent");
  const heading = dock.locator("#chat-heading");
  const badge = dock.getByTestId("task-status");
  const rail = page.getByRole("navigation", { name: "Views" });
  const pane = page.getByRole("complementary", { name: "Tasks" });
  const row = (title) => pane.getByTestId("task-row").filter({ hasText: title });
  const group = (g) => pane.locator(`[data-group="${g}"]`);
  const titles = (g) => group(g).locator(".task-title");
  const taskPage = page.getByTestId("task-page");
  /** Restarts the app on the same home with `fixture`, and opens the page again. */
  const restart = async (fixture) => {
    await stopServer(server);
    server = await startServer(server.port, {
      dir,
      fakeAgent: `${FIXTURES}/${fixture}`,
      home: server.home,
      token: server.token,
    });
    await reopen(page, server.url);
    await expect(page.getByRole("tab", { name: "Game" })).toBeVisible();
  };
  // From the pane's "New task", which puts the composer in new-task mode.
  const ask = async (text) => {
    await rail.getByRole("button", { name: "Tasks" }).click();
    await pane.getByRole("button", { name: "New task" }).click();
    await expect(heading).toHaveText("New task");
    await expect(input).toBeFocused();
    await input.fill(text);
    await input.press("Enter");
    await expect(heading).toHaveText(text);
  };
  try {
    await page.goto(server.url);
    await expect(page.getByRole("tab", { name: "Game" })).toBeVisible();
    // An empty list says what will be here.
    await rail.getByRole("button", { name: "Tasks" }).click();
    await expect(pane.getByTestId("tasks-empty")).toContainText("No tasks yet");

    // Four tasks, one per fixture: kept, ready for review, failed, running.
    await input.fill(KEPT);
    await input.press("Enter");
    await expect(badge).toHaveText("Ready for review", { timeout: 15_000 });
    await dock.getByTestId("review-card").getByRole("button", { name: "Keep change" }).click();
    await expect(badge).toHaveText("Kept");
    await restart("label.jsonl");
    await ask(READY);
    await expect(badge).toHaveText("Ready for review", { timeout: 15_000 });
    await restart("fail.jsonl");
    await ask(FAILED);
    await expect(badge).toHaveText("Failed", { timeout: 15_000 });
    await restart("slow.jsonl");
    await ask(RUNNING);
    await expect(badge).toHaveText("In progress");
    await expect(dock.locator(".msg-agent")).toContainText("Still thinking");

    // The groups, in order; Done collapsed; the badge counts failed + ready.
    await rail.getByRole("button", { name: "Tasks" }).click();
    await expect(pane.locator(".tasks-group")).toHaveCount(4);
    expect(await pane.locator(".tasks-group").evaluateAll((gs) => gs.map((g) => g.dataset.group))).toEqual([
      "waiting",
      "review",
      "progress",
      "done",
    ]);
    await expect(titles("waiting")).toHaveText([FAILED]);
    await expect(titles("review")).toHaveText([READY]);
    await expect(titles("progress")).toHaveText([RUNNING]);
    await expect(titles("done")).toHaveCount(0);
    const done = pane.getByRole("button", { name: "Done · 1" });
    await expect(done).toHaveAttribute("aria-expanded", "false");
    await expect(rail.getByTestId("tasks-badge")).toHaveText("2");
    await expect(rail.getByRole("button", { name: "Tasks, 2 need you" })).toBeVisible();
    // Each row: its pill, its second line and when it was last active.
    await expect(row(READY).getByTestId("task-row-status")).toHaveText("Ready for review");
    await expect(row(READY).locator(".task-second")).toHaveText("+1 −1 in 1 file");
    await expect(row(FAILED).locator(".task-second")).toHaveText("No game session is running.");
    await expect(row(RUNNING).locator(".task-second")).toHaveText("Still thinking about the count.");
    await expect(row(READY).locator(".task-when")).toHaveText(/just now|min ago/);
    // The running task is the current one, in the pane as in the dock.
    await expect(row(RUNNING).locator(".task-main")).toHaveAttribute("aria-current", "true");
    await done.click();
    await expect(titles("done")).toHaveText([KEPT]);
    await expect(row(KEPT).getByTestId("task-row-status")).toHaveText("Kept");
    // The pane scrolls with the themed scrollbar.
    await page.mouse.move(0, 0);
    const bar = await pane.evaluate((el) => {
      const s = getComputedStyle(el);
      return { width: s.scrollbarWidth, color: s.scrollbarColor, overflow: s.overflowY };
    });
    expect(bar).toEqual({ width: "thin", color: "rgb(42, 51, 64) rgba(0, 0, 0, 0)", overflow: "auto" });
    await shot(page, `tasks-pane-${width}`);
    await step(page, width);

    // A row is the chat's current task and opens its page in the focused pane.
    await row(READY).locator(".task-main").click();
    await expect(heading).toHaveText(READY);
    const readyTab = page.getByRole("tab", { name: `Task · ${READY}` });
    await expect(readyTab).toHaveAttribute("aria-selected", "true");
    await expect(row(READY).locator(".task-main")).toHaveAttribute("aria-current", "true");
    await expect(taskPage.getByTestId("task-page-status")).toHaveText("Ready for review");
    await expect(taskPage.locator(".task-meta")).toContainText("claude · claude-model");
    await expect(taskPage.locator(".task-meta")).toContainText(/Started (just now|1 min ago)/);
    const section = (name) => taskPage.getByRole("region", { name });
    await expect(section("What the agent was given").locator(".task-given")).toHaveText(READY);
    await expect(section("Steps").getByTestId("task-step")).toHaveText(["Readgame.fun", "Editedgame.fun"]);
    await expect(section("Change").getByTestId("task-change")).toContainText("1 file +1 −1");
    await expect(section("Change").locator(".task-change-files")).toContainText("examples/counter/game.fun");
    await expect(section("Usage").getByTestId("task-usage").locator("tbody tr")).toHaveText(["14.1k52"]);
    await expect(section("Usage").locator("tfoot")).toHaveText("1 turn4.1k52");
    await expect(taskPage.getByRole("region", { name: "Walkthrough" })).toHaveCount(0);
    await taskPage.evaluate((el) => el.scrollTo(0, 0));
    await shot(page, `task-page-${width}`);
    await step(page, width);

    // Open changes puts the Changes tab beside the page.
    await section("Change").getByRole("button", { name: "Open changes" }).click();
    await expect(page.getByRole("tab", { name: `Changes · ${READY}` })).toHaveAttribute("aria-selected", "true");
    await expect(readyTab).toHaveAttribute("aria-selected", "true");
    await step(page, width);

    // Keep from the page: the same keep as the Changes tab, and the task is done.
    await section("Change").getByRole("button", { name: "Keep change" }).click();
    await expect(taskPage.getByTestId("task-page-status")).toHaveText("Kept");
    await expect(section("Change").getByTestId("task-change-closed")).toHaveText("The change was kept just now.");
    await expect(page.getByTestId("diff-empty")).toHaveText("The change was kept. Nothing left to review.");
    await expect(titles("done")).toHaveText([READY, KEPT]);
    await expect(pane.locator('[data-group="review"]')).toHaveCount(0);
    await expect(rail.getByTestId("tasks-badge")).toHaveText("1");
    await expect(badge).toHaveText("Kept");
    expect(app.state.tasks.find((t) => t.title === READY).baseline).toBe(null);
    await step(page, width);

    // Alt-click: the failed task's page opens to the side; its steps mark the failure.
    await row(FAILED).locator(".task-main").click({ modifiers: ["Alt"] });
    await expect(heading).toHaveText(FAILED);
    const failedTab = page.getByRole("tab", { name: `Task · ${FAILED}` });
    await expect(failedTab).toHaveAttribute("aria-selected", "true");
    const panes = await tabs(page);
    const where = (title) => panes.findIndex((p) => p.includes(title));
    expect(where(`Task · ${FAILED}`), JSON.stringify(panes)).not.toBe(where(`Task · ${READY}`));
    const failedPage = page.getByTestId("task-page").filter({ hasText: FAILED });
    await expect(failedPage.locator(".task-step-failed")).toHaveText(/Read the game state.*failed/);
    await expect(failedPage.locator(".task-run-head")).toContainText("Failed: No game session is running.");

    // Rename in place, from the row: the tab and the dock follow.
    await row(READY).getByRole("button", { name: `Rename ${READY}` }).click();
    const field = pane.getByLabel("Task title");
    await field.fill("Label the counter clearly");
    await field.press("Enter");
    await expect(page.getByRole("tab", { name: "Task · Label the counter clearly" })).toBeVisible();
    await expect(row("Label the counter clearly")).toHaveCount(1);
    // Escape leaves it as it was.
    await row(KEPT).locator(".task-main").focus();
    await page.keyboard.press("F2");
    await field.fill("Not this");
    await field.press("Escape");
    await expect(titles("done")).toHaveText(["Label the counter clearly", KEPT]);

    // Delete is refused while the task runs, saying why.
    await row(RUNNING).getByRole("button", { name: `Delete ${RUNNING}` }).click();
    const refuse = pane.getByRole("group", { name: `Delete ${RUNNING}` });
    await expect(refuse).toContainText("Cannot delete this task yet. It is running: stop it first.");
    await expect(refuse.getByRole("button", { name: "Delete" })).toHaveCount(0);
    await refuse.getByRole("button", { name: "OK" }).click();
    await expect(refuse).toHaveCount(0);
    // Delete asks in place; Cancel keeps it; Delete removes its file and closes its tab.
    const before = taskFiles(server.home).sort();
    expect(before).toEqual(["1.json", "2.json", "3.json", "4.json"]);
    const failedId = app.state.tasks.find((t) => t.title === FAILED).id;
    await row(FAILED).getByRole("button", { name: `Delete ${FAILED}` }).click();
    const confirm = pane.getByRole("group", { name: `Delete ${FAILED}` });
    await expect(confirm).toContainText("Its conversation is removed from the app; your files stay as they are.");
    await confirm.getByRole("button", { name: "Cancel" }).click();
    await expect(row(FAILED)).toHaveCount(1);
    await row(FAILED).getByRole("button", { name: `Delete ${FAILED}` }).click();
    await confirm.getByRole("button", { name: "Delete" }).click();
    await expect(row(FAILED)).toHaveCount(0);
    await expect(failedTab).toHaveCount(0);
    await expect(heading).toHaveText("New task");
    await expect(rail.getByTestId("tasks-badge")).toHaveCount(0);
    await expect.poll(() => taskFiles(server.home).sort()).toEqual(before.filter((f) => f !== `${failedId}.json`));
    expect(app.state.tasks.map((t) => t.title).sort()).toEqual([KEPT, "Label the counter clearly", RUNNING].sort());

    // Search over titles; no match says so.
    const search = pane.getByLabel("Search tasks");
    await search.fill("COUNT");
    await expect(pane.locator(".task-title")).toHaveText(["Label the counter clearly", KEPT]);
    await search.fill("zzz");
    await expect(pane.getByTestId("tasks-empty")).toHaveText("No task titles match “zzz”.");
    await search.fill("");

    // Keyboard: arrows move between rows, Enter opens one.
    await row(RUNNING).locator(".task-main").focus();
    await page.keyboard.press("ArrowDown");
    await expect(row("Label the counter clearly").locator(".task-main")).toBeFocused();
    await page.keyboard.press("End");
    await expect(row(KEPT).locator(".task-main")).toBeFocused();
    await page.keyboard.press("Home");
    await expect(row(RUNNING).locator(".task-main")).toBeFocused();
    await page.keyboard.press("ArrowDown");
    await page.keyboard.press("Enter");
    await expect(heading).toHaveText("Label the counter clearly");
    await expect(page.getByRole("tab", { name: "Task · Label the counter clearly" })).toHaveAttribute("aria-selected", "true");

    // Continue in chat: the composer, on this task.
    await row(RUNNING).locator(".task-main").click();
    await expect(heading).toHaveText(RUNNING);
    await page.getByRole("tab", { name: "Task · Label the counter clearly" }).click();
    const label = page.getByTestId("task-page").filter({ hasText: "Label the counter clearly" });
    await label.getByRole("button", { name: "Continue in chat" }).click();
    await expect(heading).toHaveText("Label the counter clearly");
    await expect(input).toBeFocused();

    // A reopened URL has the same current task, in the pane and the dock.
    await reopen(page, server.url);
    await expect(heading).toHaveText("Label the counter clearly");
    await rail.getByRole("button", { name: "Tasks" }).click();
    await pane.getByRole("button", { name: /^Done/ }).click();
    await expect(row("Label the counter clearly").locator(".task-main")).toHaveAttribute("aria-current", "true");
    await expect(pane.locator('.task-main[aria-current="true"]')).toHaveCount(1);

    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
    expect(overflow, "horizontal overflow (px)").toBeLessThanOrEqual(0);
    expect(errors, "console errors").toEqual([]);
    expect(external, "requests to non-loopback hosts").toEqual([]);
    console.log(`ok: tasks ${width}x${height}`);
  } finally {
    await page.close();
    await stopServer(server);
  }
}

const browser = await chromium.launch();
try {
  await tasksView(browser, 1440, 900);
  await tasksView(browser, 700, 900);
} catch (e) {
  console.error(`FAIL: ${e.message}`);
  process.exitCode = 1;
} finally {
  await browser.close();
}
