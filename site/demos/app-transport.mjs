// Reproducible capture of the landing page's "Rewind before you decide" GIF:
// the desktop app's transport bar under a live game, on a throwaway copy of
// examples/platformer. No agent runs; the one edit is this script saving
// game.fun (jumpVelocity 13.0 → 16.0) as an editor would.
//
// What it records, cropped to the game and its bar: the save hot-reloads the
// game (a mark lands on the rail) → the hero runs and jumps (key marks) → the
// game is paused in mid-air and stepped a frame at a time → the playhead is
// dragged back to before the run → resumed, the run and jump are played again.
//
// Prereqs: npm run build:app && npm run build:cli, @playwright/test's
// chromium, and ffmpeg on PATH.
//
//   npm run demo:app-transport                  # -> site/media/app-transport.gif (+ .png)
//   node site/demos/app-transport.mjs out.gif   # custom output path
import { chromium } from "@playwright/test";
import {
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
  setJump,
  sleep,
  startApp,
  startGame,
  stopServer,
  untilState,
  watchState,
} from "./lib/app.mjs";

const { out, still } = outputs("app-transport");
const WIDTH = 1280;
const HEIGHT = 560;
const FPS = 12;

requireBuilt();
const dir = platformerCopy();
const server = await startApp({ dir });
const browser = await chromium.launch();
try {
  const page = await browser.newPage({ viewport: { width: WIDTH, height: HEIGHT } });
  const app = watchState(page);
  const { bar, view, debugUrl } = await startGame(page, app, server);
  const box = await page.locator(".game-stack").boundingBox();
  const clip = { x: Math.round(box.x), y: Math.round(box.y), width: Math.round(box.width), height: Math.round(box.height) };
  await sleep(800);
  const rec = await record(page);
  await sleep(600);

  // A save: the running game reloads, and the rail marks where.
  setJump(dir, "13.0", "16.0");
  await bar.getByRole("button", { name: /^frame \d+, hot reload/ }).waitFor({ timeout: 20_000 });
  await sleep(900);

  // Play, pause in mid-air, and step on a frame at a time.
  await runAndJump(page, view, debugUrl);
  await untilState(debugUrl, (s) => s.model.vy < 9, "the jump's rise");
  await pause(bar, debugUrl);
  await page.keyboard.up("ArrowRight");
  await sleep(900);
  for (let i = 0; i < 8; i++) {
    await bar.getByRole("button", { name: "Step one frame forward" }).click();
    await sleep(220);
  }
  await sleep(500);
  await saveStill(page, still, clip);

  // Drag back to before the run, then play it again from there.
  await dragBeforeRun(page, bar, debugUrl);
  await sleep(900);
  await bar.getByRole("button", { name: "Play" }).click();
  await sleep(300);
  await runAndJump(page, view, debugUrl);
  if (!(await land(page, debugUrl))) throw new Error("the jump fell short");
  await sleep(900);
  gif(await rec.stop(), out, { fps: FPS, clip });
} finally {
  await browser.close();
  await stopServer(server);
}
