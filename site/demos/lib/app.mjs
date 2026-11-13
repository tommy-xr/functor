// Shared by the desktop-app captures (app-loop, app-review, app-transport):
// start the release `functor-app` headless on a throwaway copy of
// examples/platformer, drive it through its own UI, record the page in real
// time, and assemble the frames into a GIF + still. Nothing is drawn over the
// page: every frame is the app's own picture.
//
// Prereqs: npm run build:app && npm run build:cli (release: the game renders
// live), @playwright/test's chromium, and ffmpeg on PATH.
import { execFileSync, spawn } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve as resolvePath } from "node:path";
import { GIT_ENV, ROOT, git, setJump, tempDir, tempHome, tempProject } from "../../../app/ui/e2e/app.mjs";

export { ROOT, setJump, stopServer, watchState } from "../../../app/ui/e2e/app.mjs";

const APP = join(ROOT, "target/release/functor-app");
const FUNCTOR = join(ROOT, "target/release/functor");

export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** Exits unless the release app and CLI are built. */
export function requireBuilt() {
  for (const [path, what] of [
    [APP, "npm run build:app"],
    [FUNCTOR, "npm run build:cli"],
  ]) {
    if (existsSync(path)) continue;
    console.error(`missing ${path} — run ${what}`);
    process.exit(1);
  }
}

/** The output GIF (argv[2], else site/media/`name`.gif) and its still beside it. */
export function outputs(name) {
  const out = process.argv[2] ? resolvePath(process.argv[2]) : join(ROOT, `site/media/${name}.gif`);
  if (!out.endsWith(".gif")) {
    console.error(`the output path must end in .gif (got ${out})`);
    process.exit(1);
  }
  return { out, still: out.replace(/\.gif$/, ".png") };
}

/**
 * Starts the app on `dir` with a fresh home and no device link, rendering its
 * game sessions on the GPU; `fakeAgent` is the recorded run its agent replays.
 */
export function startApp({ dir, fakeAgent }) {
  const home = tempHome();
  const args = ["--headless", "--port", "0", "--no-device-link", "-d", dir, "--functor", FUNCTOR];
  const env = { ...process.env, ...GIT_ENV, XDG_CONFIG_HOME: home, FUNCTOR_APP_HOME: home };
  if (fakeAgent) env.FUNCTOR_APP_FAKE_AGENT = fakeAgent;
  else delete env.FUNCTOR_APP_FAKE_AGENT;
  delete env.FUNCTOR_APP_TOKEN;
  const child = spawn(APP, args, { stdio: ["ignore", "ignore", "pipe"], env });
  process.on("exit", () => child.kill());
  let err = "";
  return new Promise((resolve, reject) => {
    child.stderr.on("data", (d) => {
      err += d;
      const m = err.match(/^\[functor-app\] listening on (http:\/\/\S+)\r?\n/m);
      if (m) resolve({ child, url: m[1] });
    });
    child.on("exit", (code) => reject(new Error(`functor-app exited (${code}) before listening:\n${err}`)));
  });
}

/** A committed copy of examples/platformer whose jump is `velocity` (13.0 in the example). */
export function platformerCopy(velocity) {
  const dir = tempProject("examples/platformer");
  if (velocity) {
    setJump(dir, "13.0   //", `${velocity.padEnd(7)}//`);
    git(dir, "commit", "-qam", "jump velocity");
  }
  return dir;
}

/** The session's frame, clock and model. */
export async function gameState(debugUrl) {
  return (await fetch(`${debugUrl}/state`)).json();
}

/** Resolves with the session's state once `test(state)` holds. */
export async function untilState(debugUrl, test, what, timeout = 20_000) {
  const end = Date.now() + timeout;
  for (;;) {
    const state = await gameState(debugUrl);
    if (test(state)) return state;
    if (Date.now() > end) throw new Error(`timed out waiting for ${what}`);
    await sleep(8);
  }
}

/**
 * Opens the app (its game opens paused on frame 0), plays the game and
 * resolves with its bar, view and debug server.
 */
export async function startGame(page, app, server) {
  await page.goto(server.url);
  const view = page.getByTestId("game-view");
  const bar = page.getByRole("group", { name: "Transport" });
  await bar.waitFor({ timeout: 60_000 });
  await view.locator("img").waitFor({ timeout: 60_000 });
  const debugUrl = app.state.session.status.debug_url;
  await bar.getByRole("button", { name: "Play" }).click();
  await untilState(debugUrl, (s) => !s.clock.paused, "play");
  // The page has seen it play (a click on the view now captures input); the
  // pointer leaves the bar so no tip stays up.
  await bar.getByRole("button", { name: "Pause" }).waitFor();
  await page.mouse.move(0, 0);
  return { bar, view, debugUrl };
}

/** Sets the bar's Extrapolate switch to `on`. */
export async function extrapolate(bar, on) {
  const button = bar.getByRole("button", { name: /^Extrapolate/ });
  if ((await button.getAttribute("aria-pressed")) === String(on)) return;
  await button.click();
  await bar.locator(`.transport-extrapolate[aria-pressed="${on}"]`).waitFor();
}

/**
 * Plays the platformer through the view: holds Right from the spawn point and
 * jumps on the way to the left platform's edge (x = -3), resolving once the
 * hero is in the air. A 16.0 jump is 8.5 long, so it clears the 6.0 gap from
 * anywhere past x = -5.4; the weakened 8.0 jump falls in from anywhere.
 */
export async function runAndJump(page, view, debugUrl) {
  await view.click();
  await page.keyboard.down("ArrowRight");
  await untilState(debugUrl, (s) => s.model.x >= -4.6, "the run-up");
  await page.keyboard.press("ArrowUp");
  await untilState(debugUrl, (s) => !s.model.grounded, "takeoff");
}

/**
 * Waits out the jump and lets go of the view; resolves with whether the hero
 * landed on the far side (else he fell in and respawned).
 */
export async function land(page, debugUrl) {
  const { model } = await untilState(debugUrl, (s) => s.model.grounded, "a landing or a respawn");
  // A tap of Right clears the held direction on either outcome.
  await page.keyboard.up("ArrowRight");
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("Escape");
  return model.x > 3;
}

/** Drags the transport's playhead to `unit` (0–1) of its rail, slowly enough to follow. */
export async function dragPlayhead(page, bar, unit) {
  const head = await bar.getByRole("slider", { name: "Selected frame" }).boundingBox();
  const rail = await bar.locator(".transport-rail").boundingBox();
  const y = head.y + head.height / 2;
  const from = head.x + head.width / 2;
  const to = rail.x + unit * rail.width;
  await page.mouse.move(from, y);
  await page.mouse.down();
  for (let i = 1; i <= 20; i++) {
    await page.mouse.move(from + ((to - from) * i) / 20, y);
    await sleep(40);
  }
  await page.mouse.up();
}

/**
 * Paused: drags the playhead back to the first `Right down` mark (the run;
 * `land` taps Right after it), then walks to a frame before it, where the
 * hero still stands at the spawn point.
 */
export async function dragBeforeRun(page, bar, debugUrl) {
  const rail = await bar.locator(".transport-rail").boundingBox();
  const down = bar.getByRole("button", { name: /^frame \d+, Right down/ }).first();
  const pressed = Number((await down.getAttribute("aria-label")).match(/^frame (\d+)/)[1]);
  const mark = await down.boundingBox();
  await dragPlayhead(page, bar, (mark.x + mark.width / 2 - rail.x) / rail.width);
  await bar.getByRole("slider", { name: "Selected frame" }).focus();
  for (let i = 0; i < 120; i++) {
    const { frame, model } = await gameState(debugUrl);
    // Before the press: a respawn after it also stands at the spawn point.
    if (frame < pressed && !model.rightHeld && model.grounded && model.x === -6) return;
    await page.keyboard.press("ArrowLeft");
    await untilState(debugUrl, (s) => s.frame !== frame, "a frame step");
  }
  throw new Error("no standing frame before the Right down mark");
}

/** Clicks the bar's Pause and waits for the session to hold. */
export async function pause(bar, debugUrl) {
  await bar.getByRole("button", { name: "Pause" }).click();
  await untilState(debugUrl, (s) => s.clock.paused, "the pause");
}

/** Types `text` into the focused field at a readable pace. */
export async function type(page, text) {
  await page.keyboard.type(text, { delay: 32 });
}

/**
 * Records `page` in real time until `stop()`: the browser's own screencast,
 * each frame saved with the time it was painted, so nothing here slows the
 * page or the script driving it. `stop()` resolves with the recording for
 * [`gif`].
 */
export async function record(page) {
  const dir = tempDir("functor-app-demo-");
  const cdp = await page.context().newCDPSession(page);
  const start = Date.now();
  const frames = [];
  cdp.on("Page.screencastFrame", ({ data, sessionId }) => {
    const file = join(dir, `s${String(frames.length).padStart(5, "0")}.png`);
    writeFileSync(file, Buffer.from(data, "base64"));
    frames.push({ at: (Date.now() - start) / 1000, file });
    cdp.send("Page.screencastFrameAck", { sessionId }).catch(() => {});
  });
  // At the viewport's own size: the screencast otherwise scales a tall page down.
  const { width, height } = page.viewportSize();
  await cdp.send("Page.startScreencast", { format: "png", maxWidth: width, maxHeight: height });
  return {
    stop: async () => {
      await cdp.send("Page.stopScreencast");
      return { dir, frames, length: (Date.now() - start) / 1000 };
    },
  };
}

/** Saves `page` (cropped to `clip`, when given) as a capture's still. */
export async function saveStill(page, path, clip) {
  mkdirSync(dirname(path), { recursive: true });
  await page.screenshot({ path, ...(clip && { clip }) });
  console.log(`wrote ${path}`);
}

/**
 * Assembles a recording into a looping GIF at `fps`, cropped to `clip` (page
 * pixels) when given, each frame shown for as long as it was on screen and
 * the last one held `hold` seconds.
 */
export function gif({ dir, frames, length }, out, { fps, clip, hold = 2 }) {
  const list = frames
    .map(({ at, file }, i) => `file '${file}'\nduration ${((frames[i + 1]?.at ?? length + hold) - at).toFixed(3)}`)
    .join("\n");
  // The concat demuxer needs the last file named again to honour its duration.
  writeFileSync(join(dir, "frames.txt"), `${list}\nfile '${frames.at(-1).file}'\n`);
  const crop = clip ? `crop=${clip.width}:${clip.height}:${clip.x}:${clip.y},` : "";
  mkdirSync(dirname(out), { recursive: true });
  execFileSync(
    "ffmpeg",
    [
      "-y", "-loglevel", "error",
      "-f", "concat", "-safe", "0",
      "-i", join(dir, "frames.txt"),
      "-vf",
      `fps=${fps},${crop}split[s0][s1];[s0]palettegen=stats_mode=diff[p];[s1][p]paletteuse=dither=bayer:bayer_scale=5:diff_mode=rectangle`,
      "-loop", "0",
      out,
    ],
    { stdio: "inherit" },
  );
  console.log(`wrote ${out} (${(length + hold).toFixed(1)}s, ${frames.length} painted frames)`);
}
