// The task queue (D30) and where a typed message goes, end to end against
// the fake agent (a slow run that reads and thinks). While a task runs, the
// composer says the message steers it, with a switch to a new task, which
// queues behind it; a message to a queued task is added to it. The Tasks
// pane lists the queue in line with what it waits for; a row moves up and
// is removed; a typed steer stops and resumes the run; stopping the run
// starts the next queued task by itself. Runs at 1440×900 and 700×900.
//
//   npm run test:app-ui
//
// APP_UI_SHOTS=<dir> saves the composer and the queue at both widths.
import { chromium, expect } from "@playwright/test";
import { ROOT, requireBuilt, shot, startServer, stopServer, tempProject, watchErrors } from "./app.mjs";

const SLOW = `${ROOT}app/ui/e2e/fixtures/slow.jsonl`;
const RUNNING = "Think for a while";
const RED = "Make the ball red";
const PAUSE = "Add a pause menu";

requireBuilt();

async function queue(browser, width, height) {
  const server = await startServer(0, { dir: tempProject("examples/counter"), fakeAgent: SLOW });
  const page = await browser.newPage({ viewport: { width, height } });
  const errors = watchErrors(page);
  const dock = page.locator(".chat");
  const input = dock.getByLabel("Message the agent");
  const heading = dock.locator("#chat-heading");
  const badge = dock.getByTestId("task-status");
  const place = dock.getByTestId("composer-place");
  const rail = page.getByRole("navigation", { name: "Views" });
  const pane = page.getByRole("complementary", { name: "Tasks" });
  const row = (title) => pane.getByTestId("task-row").filter({ hasText: title });
  const titles = (g) => pane.locator(`[data-group="${g}"] .task-title`);
  const say = async (text) => {
    await input.fill(text);
    await input.press("Enter");
    await expect(dock.locator(".msg-user").last()).toHaveText(text);
  };
  try {
    await page.goto(server.url);
    await expect(place).toHaveText("Starts a new task");
    await say(RUNNING);
    await expect(badge).toHaveText("In progress");
    await expect(dock.locator(".msg-agent")).toContainText("Still thinking");

    // Running: the message steers it, or switches to a new task that queues.
    await expect(place.locator("span")).toHaveText(`Steers “${RUNNING}”`);
    await input.fill(RED);
    await shot(page, `queue-composer-${width}`);
    await place.getByRole("button", { name: "New task instead" }).click();
    await expect(place.locator("span")).toHaveText(`Queues a new task behind “${RUNNING}”`);
    await place.getByRole("button", { name: `Steer “${RUNNING}” instead` }).click();
    await expect(place.locator("span")).toHaveText(`Steers “${RUNNING}”`);
    await place.getByRole("button", { name: "New task instead" }).click();
    await input.press("Enter");
    await expect(heading).toHaveText(RED);
    await expect(badge).toHaveText("Queued");
    // Said to a queued task: added to what it starts with.
    await expect(place.locator("span")).toHaveText(`Adds to “${RED}”`);
    await say("Actually blue");
    await expect(dock.locator(".msg-user")).toHaveText([RED, "Actually blue"]);
    await place.getByRole("button", { name: "New task instead" }).click();
    await say(PAUSE);
    await expect(heading).toHaveText(PAUSE);

    // The queue in line, with what it waits for; moved and pruned.
    await rail.getByRole("button", { name: "Tasks" }).click();
    await expect(titles("queued")).toHaveText([RED, PAUSE]);
    await expect(row(RED).locator(".task-second")).toHaveText("Next");
    await expect(pane.getByTestId("queue-wait")).toHaveText(`Starts when “${RUNNING}” ends.`);
    await shot(page, `queue-pane-${width}`);
    await row(PAUSE).hover();
    await pane.getByRole("button", { name: `Move ${PAUSE} up` }).click();
    await expect(titles("queued")).toHaveText([PAUSE, RED]);
    await row(RED).hover();
    await pane.getByRole("button", { name: `Delete ${RED}` }).click();
    await pane.getByRole("group", { name: `Delete ${RED}` }).getByRole("button", { name: "Delete" }).click();
    await expect(titles("queued")).toHaveText([PAUSE]);

    // A typed steer: the run stops and resumes with it.
    await row(RUNNING).locator(".task-main").click();
    if (width < 1000) await rail.getByRole("button", { name: "Tasks" }).click();
    await expect(heading).toHaveText(RUNNING);
    await say("and faster");
    await expect(dock.getByTestId("result")).toHaveText(/Stopped/);
    await expect(badge).toHaveText("In progress");

    // Stopped, its change measured (none): the next one starts by itself.
    await dock.getByRole("button", { name: "Stop" }).click();
    await rail.getByRole("button", { name: "Tasks" }).click();
    await expect(titles("progress")).toHaveText([PAUSE], { timeout: 10_000 });
    await expect(pane.locator('[data-group="queued"]')).toHaveCount(0);
    expect(errors).toEqual([]);
  } finally {
    await page.close();
    await stopServer(server);
  }
}

const browser = await chromium.launch();
try {
  for (const [w, h] of [
    [1440, 900],
    [700, 900],
  ]) {
    await queue(browser, w, h);
    console.log(`ok queue ${w}×${h}`);
  }
} finally {
  await browser.close();
}
