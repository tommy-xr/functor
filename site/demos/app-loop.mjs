// Reproducible capture of the landing page's lead GIF: the desktop app's
// describe → edit → reload → scrub → keep loop, on examples/platformer.
//
// The app, the game and every click are real; two things are staged, and
// named here. The project is a throwaway copy of the platformer whose jump is
// weakened to 8.0 (the example ships 13.0), so there is something to fix. And
// the agent is the app's recorded-run seam (FUNCTOR_APP_FAKE_AGENT) replaying
// fixtures/jump.jsonl through the code path a real run takes: its get_state
// reads the live game over the run's own tool server, and its edit is a real
// write the running game hot-reloads.
//
// What it records: the hero jumps and falls into the gap → the request is
// typed → the run edits jumpVelocity and the game reloads (a mark lands on
// the transport rail) → the game is paused and the playhead dragged back to
// before the run → resumed, the same run and jump clear the gap → Keep change.
//
// Prereqs: npm run build:app && npm run build:cli, @playwright/test's
// chromium, and ffmpeg on PATH.
//
//   npm run demo:app-loop                  # -> site/media/app-loop.gif (+ .png)
//   node site/demos/app-loop.mjs out.gif   # custom output path
import { chromium } from "@playwright/test";
import {
  ROOT,
  dragBeforeRun,
  gif,
  land,
  outputs,
  pause,
  platformerCopy,
  record,
  requireBuilt,
  runAndJump,
  saveStill,
  sleep,
  startApp,
  startGame,
  stopServer,
  type,
  untilState,
  watchState,
} from "./lib/app.mjs";

const { out, still } = outputs("app-loop");
const WIDTH = 1200; // past the app's 1100px stacking breakpoint: the chat sits beside the game
const HEIGHT = 675;
const FPS = 12;
const ASK = "The jump falls short. Make it clear the gap.";

requireBuilt();
const server = await startApp({ dir: platformerCopy("8.0"), fakeAgent: `${ROOT}site/demos/fixtures/jump.jsonl` });
const browser = await chromium.launch();
try {
  const page = await browser.newPage({ viewport: { width: WIDTH, height: HEIGHT } });
  const app = watchState(page);
  const { bar, view, debugUrl } = await startGame(page, app, server);
  const chat = page.locator(".chat");
  await sleep(800);
  const rec = await record(page);
  await sleep(700);

  // The problem: the jump falls into the gap.
  await runAndJump(page, view, debugUrl);
  if (await land(page, debugUrl)) throw new Error("the weakened jump cleared the gap");
  await sleep(900);

  // Describe it; the run reads the game, edits game.fun, and the game reloads.
  await chat.getByLabel("Message the agent").click();
  await type(page, ASK);
  await sleep(400);
  await page.keyboard.press("Enter");
  await chat.getByTestId("task-status").filter({ hasText: "Ready for review" }).waitFor({ timeout: 30_000 });
  await bar.getByRole("button", { name: /^frame \d+, hot reload/ }).waitFor();
  await sleep(1800);

  // Scrub back to before the run and play it again under the new code.
  await pause(bar, debugUrl);
  await sleep(600);
  await dragBeforeRun(page, bar, debugUrl);
  await sleep(900);
  await bar.getByRole("button", { name: "Play" }).click();
  await sleep(300);
  await runAndJump(page, view, debugUrl);
  if (!(await land(page, debugUrl))) throw new Error("the fixed jump fell short");
  await sleep(1200);
  await saveStill(page, still);

  await chat.getByTestId("review-card").getByRole("button", { name: "Keep change" }).click();
  await chat.getByTestId("task-status").filter({ hasText: "Kept" }).waitFor();
  await untilState(debugUrl, (s) => s.model.x > 3, "the hero still across the gap");
  await sleep(600);
  gif(await rec.stop(), out, { fps: FPS });
} finally {
  await browser.close();
  await stopServer(server);
}
