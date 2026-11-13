// Path evidence (D38): the predicted path before and after a task's change,
// on the review card and the task page, against the fake agent in a
// throwaway copy of examples/platformer with its jump weakened to 8.0.
//
// Without a GPU (CI) sessions run headless, so no paths are taken: a run's
// card shows none. The card and page are then checked on a stored task
// carrying the committed captures: the overlay, its alt text, the larger
// view and its keyboard, the page's pair and a withheld reason, at 1440×900
// and 700×900.
//
// FUNCTOR_APP_GPU=1 (with FUNCTOR_BIN a functor built from this checkout)
// adds the real thing on hidden sessions. The game is driven over its debug
// server with the same inputs on the same steps each time, then paused on
// frame 5, before the jump:
// - 8.0 → 16.0: both paths are taken, match the goldens, and only the new
//   one arcs over the gap; the user's session stays on its frame.
// - a comment: the paths are the same, so none show; the page says so.
// - a broken baseline the agent fixes: none show; the page says why.
//
//   npm run test:app-ui
//   UPDATE_GOLDENS=1 FUNCTOR_APP_GPU=1 FUNCTOR_BIN=target/release/functor node app/ui/e2e/paths.mjs
//
// APP_UI_SHOTS=<dir> saves stills of the card, the larger view and the page.
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { chromium, expect } from "@playwright/test";
import {
  clock,
  difference,
  FUNCTOR,
  git,
  ROOT,
  requireBuilt,
  setJump,
  shot,
  startServer,
  stopServer,
  storePaths,
  tempProject,
  watchErrors,
  watchState,
} from "./app.mjs";

const GPU = process.env.FUNCTOR_APP_GPU === "1";
const FIXTURES = `${ROOT}app/ui/e2e/fixtures`;
const GOLDEN = `${ROOT}app/ui/e2e/golden`;
// Above the gap (as in transport.mjs): only an arc that clears it passes there.
const ARC = [0.4, 0.17, 0.6, 0.38];
const ASK = "Make the jump clear the gap";

requireBuilt();

/** A committed copy of the platformer whose jump is 8.0 (the example ships 13.0). */
function platformer() {
  const dir = tempProject("examples/platformer");
  setJump(dir, "13.0", "8.0 ");
  git(dir, "commit", "-qam", "weak jump");
  return dir;
}

const post = async (url, path, body) => {
  const r = await fetch(url + path, { method: "POST", body: JSON.stringify(body) });
  if (!r.ok) throw new Error(`POST ${path}: ${r.status} ${await r.text()}`);
};

/** `frames` steps of 1/60 s, landed. */
async function advance(url, frames) {
  await post(url, "/time", { type: "advance", dts: 1 / 60, frames });
  await expect.poll(async () => (await (await fetch(`${url}/state?model=0`)).json()).pending_steps).toBe(0);
}

/**
 * The same run every time: Right held from step 1, a jump on step 11
 * (short of the edge) that falls in at 8.0, Right let go on step 112, long
 * after a 16.0 jump lands; then back to frame 5, before the jump.
 */
async function play(url) {
  const key = (key, down) => post(url, "/input", { type: "key", key, down });
  await key("Right", true);
  await advance(url, 10);
  await key("Up", true);
  await advance(url, 1);
  await key("Up", false);
  await advance(url, 100);
  await key("Right", false);
  await advance(url, 10);
  await post(url, "/seek", { frame: 5 });
  await expect.poll(async () => (await clock(url)).frame).toBe(5);
}

/** Opens the only task's page from the Tasks pane, on its Captures tab (the change waits). */
async function openTask(page) {
  const pane = page.getByRole("complementary", { name: "Tasks" });
  if (!(await pane.isVisible())) await page.getByRole("navigation", { name: "Views" }).getByRole("button", { name: /^Tasks/ }).click();
  await pane.getByTestId("task-row").first().click();
  await page.getByTestId("task-page").getByRole("tab", { name: "Captures" }).click();
}

/** The card and page without a GPU: none from a run, then a stored pair and a stored reason. */
async function stored(browser) {
  const dir = platformer();
  let server = await startServer(0, { dir, fakeAgent: `${FIXTURES}/paths-jump.jsonl` });
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  const errors = watchErrors(page);
  const dock = page.locator(".chat");
  const card = dock.getByTestId("review-card");
  try {
    await page.goto(server.url);
    const input = dock.getByLabel("Message the agent");
    await input.fill(ASK);
    await input.press("Enter");
    await expect(card).toBeVisible({ timeout: 20_000 });
    // Headless sessions draw nothing: no paths are taken.
    await page.waitForTimeout(500);
    await expect(card.getByTestId("paths-card")).toHaveCount(0);
    await expect(card.getByTestId("paths-taking")).toHaveCount(0);

    // The same task, stored with the committed captures.
    await stopServer(server);
    const { file, saved } = storePaths(server.home);
    server = await startServer(server.port, { dir, home: server.home, token: server.token });

    for (const [width, height] of [
      [1440, 900],
      [700, 900],
    ]) {
      await page.setViewportSize({ width, height });
      await page.goto("about:blank");
      await page.goto(server.url);
      const paths = card.getByTestId("paths-card");
      await expect(paths).toBeVisible({ timeout: 20_000 });
      // Both renders, overlaid: the old one faint.
      const imgs = paths.locator("img");
      await expect(imgs).toHaveCount(2);
      await expect(imgs.nth(0)).toHaveAttribute("alt", "Predicted path after the change, from frame 5, replaying the same 121 recorded input steps");
      await expect(imgs.nth(1)).toHaveAttribute("alt", "Predicted path before the change, from frame 5, replaying the same 121 recorded input steps");
      await expect.poll(() => imgs.evaluateAll((els) => els.every((i) => i.complete && i.naturalWidth > 0))).toBe(true);
      expect(await imgs.nth(1).evaluate((i) => getComputedStyle(i).opacity)).toBe("0.3");
      await expect(paths).toContainText("Predicted from frame 5, replaying the same 121 recorded input steps.");
      await expect(paths).toContainText("overlaid");
      const box = await paths.locator(".paths-overlay").boundingBox();
      expect(box.width, "fills the card").toBeGreaterThan(200);
      await paths.scrollIntoViewIfNeeded();
      await shot(page, `paths-card-${width}`);

      // The larger view: side by side, Escape closes and focus returns.
      const open = paths.getByRole("button", { name: /open larger/ });
      // By keyboard (a click keeps the composer's focus, as the chat's buttons do).
      await open.focus();
      await page.keyboard.press("Enter");
      const dialog = page.getByRole("dialog", { name: "Predicted path" });
      await expect(dialog).toBeVisible();
      await expect(dialog.getByRole("img", { name: /^Predicted path before the change/ })).toBeVisible();
      await expect(dialog.getByRole("img", { name: /^Predicted path after the change/ })).toBeVisible();
      await expect(dialog.getByRole("button", { name: "Close" })).toBeFocused();
      expect(await dialog.evaluate((d) => d.matches(":modal")), "modal: the page behind is inert").toBe(true);
      await shot(page, `paths-dialog-${width}`);
      await page.keyboard.press("Escape");
      await expect(dialog).toHaveCount(0);
      await expect(open).toBeFocused();
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), "no horizontal scroll").toBe(true);

      // The task page's Captures tab: the pair, side by side.
      await openTask(page);
      const section = page.getByTestId("paths-section");
      await expect(section).toBeVisible();
      await expect(section.getByRole("img")).toHaveCount(2);
      await expect(section).toContainText("Before the change");
      await expect(section).toContainText("After the change");
      await section.scrollIntoViewIfNeeded();
      await shot(page, `paths-page-${width}`);
      await section.getByRole("button", { name: /open larger/ }).click();
      await expect(dialog).toBeVisible();
      await dialog.getByRole("button", { name: "Close" }).click();
      await expect(dialog).toHaveCount(0);
    }

    // A reason recorded instead: none on the card, the reason on the page.
    await stopServer(server);
    saved.task.evidence.state = { kind: "withheld", reason: "the old code: the game did not load or replay: error: x" };
    writeFileSync(file, JSON.stringify(saved));
    server = await startServer(server.port, { dir, home: server.home, token: server.token });
    await page.goto("about:blank");
    await page.goto(server.url);
    await expect(card).toBeVisible({ timeout: 20_000 });
    await expect(card.getByTestId("paths-card")).toHaveCount(0);
    await openTask(page);
    await expect(page.getByTestId("paths-note")).toHaveText(
      "No predicted path: the old code: the game did not load or replay: error: x.",
    );
    expect(errors, "console errors").toEqual([]);
    console.log("ok: paths (stored)");
  } finally {
    await page.close();
    await stopServer(server);
  }
}

/**
 * One run on a hidden session: the game played and paused on frame 5,
 * `prepare(dir)` (an edit before asking), then `fixture`'s run; resolves with
 * the task once its paths are settled, the server and the page.
 */
async function run(browser, fixture, prepare = () => {}) {
  const dir = platformer();
  const server = await startServer(0, { dir, gpu: true, fakeAgent: `${FIXTURES}/${fixture}` });
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  const app = watchState(page);
  const errors = watchErrors(page);
  await page.goto(server.url);
  await expect(page.getByRole("group", { name: "Transport" })).toBeVisible({ timeout: 60_000 });
  const debugUrl = app.state.session.status.debug_url;
  await play(debugUrl);
  prepare(dir);
  await page.waitForTimeout(300);
  const log = async () => (await (await fetch(`${debugUrl}/input-log`)).json()).steps.map((s) => [s.frame, s.events]);
  const before = { clock: await clock(debugUrl), log: await log() };
  const input = page.locator(".chat").getByLabel("Message the agent");
  await input.fill(ASK);
  await input.press("Enter");
  await expect(page.locator(".chat").getByTestId("review-card")).toBeVisible({ timeout: 30_000 });
  const ended = Date.now();
  const task = () => app.state.tasks.find((t) => t.id === 1);
  await expect.poll(() => task()?.evidence?.state.kind, { timeout: 70_000 }).not.toBe("taking");
  await expect.poll(() => task()?.evidence?.state.kind).toBeTruthy();
  const took = (Date.now() - ended) / 1000;
  // The user's session: still on its frame, paused, its inputs untouched.
  const after = { clock: await clock(debugUrl), log: await log() };
  expect(after.clock.frame, "the user's frame").toBe(before.clock.frame);
  expect(after.clock.paused, "the user's clock is paused").toBe(true);
  expect(after.log, "the user's recorded inputs").toEqual(before.log);
  expect(errors, "console errors").toEqual([]);
  return { server, page, task: task(), took };
}

async function gpu(browser) {
  if (!GPU || !FUNCTOR) {
    console.log("SKIPPED: paths on hidden sessions — set FUNCTOR_APP_GPU=1 and FUNCTOR_BIN to a functor built from this checkout");
    return;
  }
  // 8.0 → 16.0: two paths that differ where the new arc clears the gap.
  {
    const { server, page, task, took } = await run(browser, "paths-jump.jsonl");
    try {
      expect(task.moment).toEqual({ kind: "taken", frame: 5, steps: 121 });
      expect(task.evidence.state.kind, JSON.stringify(task.evidence.state)).toBe("shown");
      const read = (name) => readFileSync(join(server.home, "evidence", name));
      const [before, after] = [read(task.evidence.state.before), read(task.evidence.state.after)];
      if (process.env.UPDATE_GOLDENS === "1") {
        mkdirSync(GOLDEN, { recursive: true });
        writeFileSync(`${GOLDEN}/paths-before.png`, before);
        writeFileSync(`${GOLDEN}/paths-after.png`, after);
      }
      const arc = await difference(page, before, after, ARC);
      const golden = [
        await difference(page, before, readFileSync(`${GOLDEN}/paths-before.png`)),
        await difference(page, after, readFileSync(`${GOLDEN}/paths-after.png`)),
      ];
      console.log(`paths: taken ${took.toFixed(1)} s after the run, arc ${arc.toFixed(2)}, golden ${golden.map((g) => g.toFixed(3))}`);
      expect(arc, "only the new path arcs over the gap").toBeGreaterThan(2);
      expect(golden[0], "before matches its golden").toBeLessThan(0.5);
      expect(golden[1], "after matches its golden").toBeLessThan(0.5);
      const paths = page.locator(".chat").getByTestId("paths-card");
      await expect(paths).toBeVisible();
      await expect(paths).toContainText("Predicted from frame 5, replaying the same 121 recorded input steps.");
      await shot(page, "paths-run-1440");
    } finally {
      await page.close();
      await stopServer(server);
    }
  }
  // A comment: the paths are the same, so none show.
  {
    const { server, page, task } = await run(browser, "paths-note.jsonl");
    try {
      expect(task.evidence.state).toEqual({ kind: "same" });
      await expect(page.locator(".chat").getByTestId("paths-card")).toHaveCount(0);
      await openTask(page);
      await expect(page.getByTestId("paths-note")).toContainText("The change moves nothing on the predicted path from frame 5");
    } finally {
      await page.close();
      await stopServer(server);
    }
  }
  // A broken baseline (saved before asking; the game keeps the last good
  // program), fixed by the agent: none show, and the page says why.
  {
    const { server, page, task } = await run(browser, "paths-fix.jsonl", (dir) => setJump(dir, "8.0 ", "8.0 +"));
    try {
      const state = task.evidence.state;
      expect(state.kind).toBe("withheld");
      expect(state.reason).toMatch(/^the old code: the game did not load or replay: error: \.?\/?game\.fun:\d+/);
      await expect(page.locator(".chat").getByTestId("paths-card")).toHaveCount(0);
      await openTask(page);
      await expect(page.getByTestId("paths-note")).toContainText("No predicted path: the old code: the game did not load or replay");
      await shot(page, "paths-withheld-1440");
    } finally {
      await page.close();
      await stopServer(server);
    }
  }
  console.log("ok: paths (GPU)");
}

const browser = await chromium.launch();
try {
  // First, so UPDATE_GOLDENS writes what the stored check reads.
  await gpu(browser);
  await stored(browser);
} finally {
  await browser.close();
}
