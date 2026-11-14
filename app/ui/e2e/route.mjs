// Where speech goes (D29), end to end: `functor-app` with the device link
// on loopback, a `device-stub` headset saying a clip twice, the fake agent
// (a slow run), transcriber and model router (which answers "a new task").
// The Devices pane turns the model router on for the project; the first
// clip starts a task (nothing to choose between); the second, said while it
// runs, is queued as a new task, its line saying so with a move to steer the
// running task instead; the move steers it and the queued task is gone.
// Runs at 1440×900 and 700×900.
//
//   FUNCTOR_BIN=target/debug/functor npm run test:app-ui
//
// Skips without FUNCTOR_BIN (it runs `functor run vr`). APP_UI_SHOTS=<dir>
// saves the speech setting and the decision before and after the move.
import { chromium, expect } from "@playwright/test";
import { FUNCTOR, ROOT, requireBuilt, shot, startServer, stopServer, stub, tempDir, tempProject, watchErrors, watchState } from "./app.mjs";

const PROJECT = "examples/counter";
const SAID = "make the ball red";
const CLIP = `${ROOT}app/functor-stt/fixtures/make-the-ball-red.wav`;

requireBuilt();
if (!FUNCTOR) {
  console.log("SKIPPED: speech routing — set FUNCTOR_BIN (npm run build:cli:debug)");
  process.exit(0);
}
process.env.FUNCTOR_APP_FAKE_TRANSCRIPT = SAID;
process.env.FUNCTOR_APP_FAKE_ROUTE = '{"route":"new_task","title":"Paint the ball"}';

async function routed(browser, width, height) {
  const server = await startServer(0, {
    dir: tempProject(PROJECT),
    deviceLink: true,
    fakeAgent: `${ROOT}app/ui/e2e/fixtures/slow.jsonl`,
  });
  const page = await browser.newPage({ viewport: { width, height } });
  const app = watchState(page);
  const errors = watchErrors(page);
  const rail = page.getByRole("navigation", { name: "Views" });
  const devices = page.getByRole("complementary", { name: "Devices" });
  const pane = page.getByRole("complementary", { name: "Tasks" });
  const dock = page.locator(".chat");
  const route = dock.getByTestId("route-line");
  let quest;
  try {
    await page.goto(server.url);
    await expect.poll(() => app.state?.session?.status.kind, { timeout: 60_000 }).toBe("running");
    quest = stub(server.link, tempDir("functor-stub-"), "Quest 3", ["--say", CLIP, "--say-again"]);
    await quest.line("code ");
    await page.getByTestId("pair-prompt").getByRole("button", { name: "Allow" }).click();
    await quest.line("linked ");

    // The project's speech goes through the model router.
    if (!(await devices.isVisible())) await rail.getByRole("button", { name: "Devices" }).click();
    const speech = devices.getByTestId("speech-card");
    await speech.getByLabel("Let a model decide where it goes").click();
    await expect(speech.getByLabel("Let a model decide where it goes")).toBeChecked();
    await expect.poll(() => app.state?.model_routes).toEqual([PROJECT]);
    await shot(page, `route-setting-${width}`);

    await page.getByRole("group", { name: "Where the game shows" }).getByRole("button", { name: "Headset" }).click();
    expect(await quest.line("voice 1 w")).toBe("voice 1 working");
    await quest.line("voice 2 t");

    // Queued as a new task behind the first, with a move to steer it.
    await expect(dock.locator("#chat-heading")).toHaveText("Paint the ball");
    await expect(route.locator("span")).toHaveText(`Queued behind “${SAID}”`);
    const steer = route.getByRole("button", { name: `Steer “${SAID}” instead` });
    await expect(steer).toBeVisible();
    await shot(page, `route-queued-${width}`);

    // Moved: it steers the first; the queued task is gone.
    await steer.click();
    expect(await quest.line("voice 2 w")).toBe("voice 2 working");
    await expect.poll(() => app.state?.tasks.length).toBe(1);
    if (!(await pane.isVisible())) await rail.getByRole("button", { name: "Tasks" }).click();
    await pane.getByTestId("task-row").first().locator(".task-main").click();
    if (width < 1000) await rail.getByRole("button", { name: "Tasks" }).click();
    await expect(dock.locator("#chat-heading")).toHaveText(SAID);
    await expect(route).toHaveCount(2);
    await expect(route.last().locator("span")).toHaveText(`Moved: steered “${SAID}”`);
    await expect(route.last().getByRole("button", { name: "Make it a new task" })).toBeVisible();
    await route.last().scrollIntoViewIfNeeded();
    await shot(page, `route-moved-${width}`);
    expect(errors).toEqual([]);
  } finally {
    await page.close();
    await quest?.stop();
    await stopServer(server);
  }
}

const browser = await chromium.launch();
try {
  for (const [w, h] of [
    [1440, 900],
    [700, 900],
  ]) {
    await routed(browser, w, h);
    console.log(`ok route ${w}×${h}`);
  }
} finally {
  await browser.close();
}
