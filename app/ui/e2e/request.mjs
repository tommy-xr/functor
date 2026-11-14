// Every request shows where it is (D43), end to end: `functor-app` with the
// device link on loopback, a `device-stub` headset saying a clip, the fake
// agent (a run that fails) and transcriber. A typed task's line shows where
// it is and how it ended. Said in the headset with nothing chosen lately,
// the dock shows the new task and its line names the failed step. Said
// again while the user reads a task they just chose, the dock stays, and its
// line offers the new task instead; clicked, it shows it. Runs at 1440×900
// and 700×900.
//
//   FUNCTOR_BIN=target/debug/functor npm run test:app-ui
//
// Skips without FUNCTOR_BIN (it runs `functor run vr`). APP_UI_SHOTS=<dir>
// saves the line in the dock and the offer.
import { chromium, expect } from "@playwright/test";
import { FUNCTOR, ROOT, requireBuilt, shot, startServer, stopServer, stub, tempDir, tempProject, watchErrors, watchState } from "./app.mjs";

const PROJECT = "examples/counter";
const SAID = "make the ball red";
const CLIP = `${ROOT}app/functor-stt/fixtures/make-the-ball-red.wav`;

requireBuilt();
if (!FUNCTOR) {
  console.log("SKIPPED: request progress — set FUNCTOR_BIN (npm run build:cli:debug)");
  process.exit(0);
}
process.env.FUNCTOR_APP_FAKE_TRANSCRIPT = SAID;

async function progress(browser, width, height) {
  const server = await startServer(0, {
    dir: tempProject(PROJECT),
    deviceLink: true,
    fakeAgent: `${ROOT}app/ui/e2e/fixtures/fail.jsonl`,
  });
  const page = await browser.newPage({ viewport: { width, height } });
  const app = watchState(page);
  const errors = watchErrors(page);
  const dock = page.locator(".chat");
  const heading = dock.locator("#chat-heading");
  const line = dock.getByTestId("request");
  const store = tempDir("functor-stub-");
  const quests = [];
  try {
    await page.goto(server.url);
    await expect.poll(() => app.state?.session?.status.kind, { timeout: 60_000 }).toBe("running");

    // Typed: its line ends with the step that failed and the reason.
    const input = dock.getByLabel("Message the agent");
    await input.fill("What is the count?");
    await input.press("Enter");
    await expect(heading).toHaveText("What is the count?");
    await expect(line).toContainText("The agent failed: No game session is running.");
    await expect(line).toContainText(/Started \d+\.\d s · Worked \d+\.\d s/);
    await shot(page, `request-typed-${width}`);

    // Said in the headset, nothing chosen lately: the dock shows its task.
    const quest = stub(server.link, store, "Quest 3", ["--say", CLIP]);
    quests.push(quest);
    await quest.line("code ");
    await page.getByTestId("pair-prompt").getByRole("button", { name: "Allow" }).click();
    await quest.line("linked ");
    await page.getByRole("group", { name: "Where the game shows" }).getByRole("button", { name: "Headset" }).click();
    await quest.line("voice 1 t");
    await expect(heading).toHaveText(SAID);
    await expect(line).toContainText(/Received \d+\.\d s · Transcribed \d+\.\d s · Placed/);
    await expect(line).toContainText("The agent failed");
    await shot(page, `request-spoken-${width}`);

    // Said again while the user reads a task they just chose: offered, not switched.
    await dock.getByRole("button", { name: "Recent tasks" }).click();
    await dock.getByRole("menu", { name: "Recent tasks" }).getByRole("menuitem", { name: /What is the count/ }).click();
    await expect(heading).toHaveText("What is the count?");
    const again = stub(server.link, store, "Quest 3", ["--say", CLIP]);
    quests.push(again);
    await again.line("voice 1 t");
    const offer = line.filter({ hasText: `Said in Quest 3: “${SAID}”. Started a new task.` });
    await expect(offer).toBeVisible();
    await expect(heading).toHaveText("What is the count?");
    await shot(page, `request-offered-${width}`);
    await offer.click();
    await expect(heading).toHaveText(SAID);
    expect(app.state.tasks.length).toBe(3);
    expect(errors).toEqual([]);
  } finally {
    await page.close();
    for (const q of quests) await q.stop();
    await stopServer(server);
  }
}

const browser = await chromium.launch();
try {
  for (const [w, h] of [
    [1440, 900],
    [700, 900],
  ]) {
    await progress(browser, w, h);
    console.log(`ok request ${w}×${h}`);
  }
} finally {
  await browser.close();
}
