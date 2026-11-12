// A task's change, end to end against the fake agent in a throwaway git
// repo: the run edits game.fun, the task is ready for review, the chat card
// and the Diff tab show the edit, Revert (confirmed in place) puts the file
// back, and a second run's change is kept. Runs at 1440×900 and 700×900.
//
//   npm run test:app-ui
//
// APP_UI_SHOTS=<dir> saves stills of the Diff tab and the frames of a
// review → view diff → revert loop at 1440×900.
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { chromium, expect } from "@playwright/test";
import {
  ROOT,
  git,
  requireBuilt,
  shot,
  startServer,
  stopServer,
  tempProject,
  watchErrors,
  watchExternal,
  watchState,
} from "./app.mjs";

const EDIT = `${ROOT}app/ui/e2e/fixtures/edit.jsonl`;
const BEFORE = "let init = { count: 0.0 }";
const AFTER = "let init = { count: 10.0 }";

requireBuilt();

let frame = 0;
/** A numbered frame of the GIF loop (1440 only). */
async function step(page, width) {
  if (width === 1440) await shot(page, `loop-${String(frame++).padStart(2, "0")}`);
}

async function review(browser, width, height) {
  const dir = tempProject("examples/counter");
  const repo = join(dir, "../..");
  const game = join(dir, "game.fun");
  const original = readFileSync(game, "utf8");
  const server = await startServer(0, { dir, fakeAgent: EDIT });
  const page = await browser.newPage({ viewport: { width, height } });
  const app = watchState(page);
  const errors = watchErrors(page);
  const external = watchExternal(page);
  const dock = page.locator(".chat");
  const log = dock.getByRole("log", { name: "Conversation" });
  const input = dock.getByLabel("Message the agent");
  const badge = dock.getByTestId("task-status");
  const card = log.getByTestId("review-card");
  const diffTab = page.getByRole("tab", { name: "Diff" });
  const canvas = page.locator("#canvas-panel");
  try {
    await page.goto(server.url);
    // No task yet: the Diff tab says so in one line.
    await diffTab.click();
    await expect(canvas.getByTestId("diff-empty")).toHaveText("Pick a task to review its change.");
    await page.getByRole("tab", { name: "Game" }).click();

    await input.fill("Start the count at ten");
    await input.press("Enter");
    await expect(badge).toHaveText("Ready for review", { timeout: 15_000 });
    expect(readFileSync(game, "utf8")).toContain(AFTER);
    // The change card counts the file it names; the review card sums the change.
    const change = log.getByTestId("change-card");
    await expect(change.locator(".counts")).toHaveText("+81 −1");
    await expect(card).toContainText("Review change · 1 file +81 −1");
    await expect(dock.getByTestId("task-status")).toHaveText("Ready for review");
    await step(page, width);

    // The recent-tasks menu says it too.
    await dock.getByRole("button", { name: "Recent tasks" }).click();
    await expect(dock.getByRole("menu").getByRole("menuitem")).toContainText(["Ready for review"]);
    await page.keyboard.press("Escape");

    // "View diff" opens the Diff tab at that file, its lines coloured by kind.
    await change.getByRole("button", { name: "View diff" }).click();
    await expect(diffTab).toHaveAttribute("aria-selected", "true");
    const file = canvas.getByRole("region", { name: "examples/counter/game.fun" });
    await expect(file).toBeInViewport();
    await expect(canvas.getByTestId("diff-status")).toHaveText("Ready for review");
    await expect(file.locator(".dl-removed .dl-text")).toHaveText([BEFORE]);
    await expect(file.locator(".dl-added .dl-text").first()).toHaveText(AFTER);
    await expect(file.locator(".dl-hunk").first()).toContainText("@@ -10,7 +10,87 @@");
    // Stills from the top of the change.
    await canvas.getByTestId("diff-view").evaluate((el) => el.scrollTo(0, 0));
    await page.locator(".canvas").evaluate((el) => el.scrollIntoView());
    await step(page, width);
    await shot(page, `diff-${width}`);
    // Long files collapse until asked.
    const more = file.getByRole("button", { name: /^Show \d+ more lines$/ });
    await expect(more).toBeVisible();
    const before = await file.locator(".dl").count();
    await more.click();
    await expect(file.getByRole("button", { name: /more lines/ })).toHaveCount(0);
    expect(await file.locator(".dl").count()).toBeGreaterThan(before);

    // Every scroller is themed: a thin dark thumb on a clear track.
    const view = canvas.getByTestId("diff-view");
    // Not hovered: a hovered scroller's thumb is lighter.
    await page.mouse.move(0, 0);
    const bar = await view.evaluate((el) => {
      const s = getComputedStyle(el);
      return { width: s.scrollbarWidth, color: s.scrollbarColor };
    });
    expect(bar.width).toBe("thin");
    expect(bar.color).toBe("rgb(42, 51, 64) rgba(0, 0, 0, 0)");
    if (width === 1440) {
      await view.evaluate((el) => el.scrollTo(0, el.scrollHeight / 3));
      await shot(page, "diff-scrolled-1440");
      await view.evaluate((el) => el.scrollTo(0, 0));
    }

    // Revert asks in place, saying what it will do; Cancel leaves everything.
    const head = canvas.locator(".diff-head");
    await head.getByRole("button", { name: "Revert", exact: true }).click();
    const confirm = canvas.getByRole("group", { name: "Confirm revert" });
    await expect(confirm).toContainText("Revert puts the 1 file this task changed back as it was before it started, undoing any other edits made to it since.");
    await step(page, width);
    await confirm.getByRole("button", { name: "Cancel" }).click();
    await expect(confirm).toHaveCount(0);
    await head.getByRole("button", { name: "Revert", exact: true }).click();
    await confirm.getByRole("button", { name: "Revert 1 file" }).click();
    await expect(canvas.getByTestId("diff-empty")).toHaveText("The change was reverted. Nothing left to review.");
    await expect(badge).toHaveText("Reverted");
    expect(readFileSync(game, "utf8")).toBe(original);
    expect(git(repo, "status", "--porcelain")).toBe("");
    await expect(log.getByTestId("result")).toHaveText(["Done", "Change reverted"]);
    await expect(card).toHaveCount(0);
    await step(page, width);

    // Another run, kept from the chat's review card.
    await input.fill("Again, please");
    await input.press("Enter");
    await expect(badge).toHaveText("Ready for review", { timeout: 15_000 });
    await card.getByRole("button", { name: "Keep change" }).click();
    await expect(badge).toHaveText("Kept");
    await expect(canvas.getByTestId("diff-empty")).toHaveText("The change was kept. Nothing left to review.");
    expect(readFileSync(game, "utf8")).toContain(AFTER);
    expect(app.state.tasks[0].status.kind).toBe("kept");
    expect(app.state.tasks[0].baseline).toBe(null);

    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
    expect(overflow, "horizontal overflow (px)").toBeLessThanOrEqual(0);
    expect(errors, "console errors").toEqual([]);
    expect(external, "requests to non-loopback hosts").toEqual([]);
    console.log(`ok: changes ${width}x${height}`);
  } finally {
    await page.close();
    await stopServer(server);
  }
}

/** Outside git the feature is off, saying so in one line. */
async function untracked(browser) {
  const { mkdtempSync, cpSync, rmSync } = await import("node:fs");
  const { tmpdir } = await import("node:os");
  const dir = mkdtempSync(join(tmpdir(), "functor-app-e2e-plain-"));
  process.on("exit", () => rmSync(dir, { recursive: true, force: true }));
  cpSync(`${ROOT}examples/counter`, dir, { recursive: true });
  const server = await startServer(0, { dir, fakeAgent: EDIT });
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  const errors = watchErrors(page);
  try {
    await page.goto(server.url);
    const input = page.getByLabel("Message the agent");
    await input.fill("Start the count at ten");
    await input.press("Enter");
    await expect(page.getByTestId("task-status")).toHaveText("Waiting on you", { timeout: 15_000 });
    await page.getByRole("tab", { name: "Diff" }).click();
    await expect(page.getByTestId("diff-empty")).toHaveText(
      "This folder is not a git repository, so changes are not tracked.",
    );
    expect(errors, "console errors").toEqual([]);
    console.log("ok: changes outside git");
  } finally {
    await page.close();
    await stopServer(server);
  }
}

const browser = await chromium.launch();
try {
  await review(browser, 1440, 900);
  await review(browser, 700, 900);
  await untracked(browser);
} catch (e) {
  console.error(`FAIL: ${e.message}`);
  process.exitCode = 1;
} finally {
  await browser.close();
}
