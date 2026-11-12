// Reproducible capture of the landing page's "It shows its work" GIF: the
// desktop app's end-of-run walkthrough and review, on examples/platformer.
//
// Staged as app-loop.mjs is: a throwaway copy of the platformer with its jump
// weakened to 8.0, and the app's recorded-run seam replaying
// fixtures/jump-walkthrough.jsonl, whose `present` call goes to the run's real
// tool server as an agent's would.
//
// What it records, cropped to the canvas and the chat beside it: the
// walkthrough card's first step, with the changed line of game.fun marked in
// a pane the agent opened beside the game → Next: the change's diff → Keep
// change.
//
// Prereqs: npm run build:app && npm run build:cli, @playwright/test's
// chromium, and ffmpeg on PATH.
//
//   npm run demo:app-review                  # -> site/media/app-review.gif (+ .png)
//   node site/demos/app-review.mjs out.gif   # custom output path
import { chromium } from "@playwright/test";
import {
  ROOT,
  gif,
  outputs,
  platformerCopy,
  record,
  requireBuilt,
  saveStill,
  sleep,
  startApp,
  startGame,
  stopServer,
  watchState,
} from "./lib/app.mjs";

const { out, still } = outputs("app-review");
const WIDTH = 1280; // wide enough for the canvas to split, so the agent's file opens beside the game
const HEIGHT = 720;
const FPS = 8;

requireBuilt();
const server = await startApp({ dir: platformerCopy("8.0"), fakeAgent: `${ROOT}site/demos/fixtures/jump-walkthrough.jsonl` });
const browser = await chromium.launch();
try {
  const page = await browser.newPage({ viewport: { width: WIDTH, height: HEIGHT } });
  const app = watchState(page);
  await startGame(page, app, server);
  const chat = page.locator(".chat");
  const input = chat.getByLabel("Message the agent");
  await input.fill("The jump falls short. Make it clear the gap.");
  await input.press("Enter");

  // The run ends: its first step opens game.fun beside the game, line 22 marked.
  const card = chat.getByTestId("walkthrough");
  await card.locator(".walk-count").filter({ hasText: "Step 1 of 3" }).waitFor({ timeout: 30_000 });
  await page.getByTestId("code-mark").waitFor();
  // From the canvas to the page's right edge, below the title bar.
  const canvas = await page.locator(".canvas").boundingBox();
  const [x, y] = [Math.round(canvas.x), Math.round(canvas.y)];
  const clip = { x, y, width: WIDTH - x, height: HEIGHT - y };
  await sleep(500);
  const rec = await record(page);
  await sleep(2200);
  await saveStill(page, still, clip);

  await card.getByRole("button", { name: "Next" }).click();
  await page.getByTestId("diff-view").waitFor();
  await sleep(2600);

  await chat.getByTestId("review-card").getByRole("button", { name: "Keep change" }).click();
  await chat.getByTestId("task-status").filter({ hasText: "Kept" }).waitFor();
  await sleep(800);
  gif(await rec.stop(), out, { fps: FPS, clip });
} finally {
  await browser.close();
  await stopServer(server);
}
