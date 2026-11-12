// The new-project modal (D24) against the CLI's real templates: opened from
// the project switcher, its gallery searched, filtered and chosen by
// keyboard, a project created into a temp folder that the app then opens
// and starts (no GPU) with the described task in the chat (fake agent), and
// a target that is not empty refused. 1440×900 and 700×900, temp home.
//
//   npm run test:app-ui     # needs FUNCTOR_BIN (`npm run build:cli:debug`); skipped without it
//
// APP_UI_SHOTS=<dir> saves stills at both widths and, at 1440, the frames
// of create → open → start.
import { mkdtempSync, realpathSync, rmSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { chromium, expect } from "@playwright/test";
import { FUNCTOR, ROOT, requireBuilt, shot, startServer, stopServer, tempProject, watchErrors, watchExternal, watchState } from "./app.mjs";

requireBuilt();
if (!FUNCTOR) {
  console.log("skip: new project (set FUNCTOR_BIN to a built functor)");
  process.exit(0);
}
const CHAT = join(ROOT, "app/ui/e2e/fixtures/chat.jsonl");
const DESCRIBE = "Make the player blue";

let frame = 0;
async function step(page, width) {
  if (width === 1440) await shot(page, `newproject-loop-${String(frame++).padStart(2, "0")}`);
}

async function newProject(browser, width, height) {
  const tag = `${width}x${height}`;
  const parent = realpathSync(mkdtempSync(join(tmpdir(), "functor-app-e2e-new-")));
  process.on("exit", () => rmSync(parent, { recursive: true, force: true }));
  const server = await startServer(0, { dir: tempProject("examples/counter"), fakeAgent: CHAT });
  const page = await browser.newPage({ viewport: { width, height } });
  const app = watchState(page);
  const errors = watchErrors(page);
  const external = watchExternal(page);
  const explorer = page.getByRole("complementary", { name: "Explorer" });
  const dialog = page.getByRole("dialog", { name: "New project" });
  const cards = dialog.getByRole("radiogroup", { name: "Templates" }).getByRole("radio");
  const checked = dialog.getByRole("radio", { checked: true });
  const create = dialog.getByRole("button", { name: "Create and start" });
  const name = dialog.getByRole("textbox", { name: "Name" });
  const folder = dialog.getByRole("textbox", { name: "Parent folder" });
  try {
    await page.goto(server.url);
    await expect(page.locator(".titlebar")).toContainText("counter");
    await explorer.getByRole("button", { name: "New project…" }).click();
    await expect(dialog).toBeVisible();
    await expect(name).toBeFocused();
    // The CLI's list, starters first, 3d chosen.
    await expect.poll(() => cards.count()).toBeGreaterThanOrEqual(6);
    await expect(checked).toHaveAttribute("data-template", "3d");
    const kinds = await cards.locator(".np-kind").allTextContents();
    expect(kinds.indexOf("Example"), "starters before examples").toBeGreaterThan(kinds.lastIndexOf("Starter"));
    await step(page, width);

    // Search and chips narrow it; All brings everything back.
    const all = await cards.count();
    const search = dialog.getByRole("searchbox", { name: "Search templates" });
    await search.fill("puzzle");
    await expect(cards).toHaveCount(1);
    await expect(cards.first()).toHaveAttribute("data-template", "tetris");
    await search.fill("");
    const chips = dialog.getByRole("group", { name: "Filter templates" });
    await chips.getByRole("button", { name: "vr", exact: true }).click();
    await expect(cards).toHaveCount(1);
    await expect(chips.getByRole("button", { name: "vr", exact: true })).toHaveAttribute("aria-pressed", "true");
    await chips.getByRole("button", { name: "All", exact: true }).click();
    await expect(cards).toHaveCount(all);

    // A radio group: Tab reaches the chosen card, arrows move the choice.
    await checked.focus();
    await page.keyboard.press("ArrowRight");
    const second = await cards.nth(1).getAttribute("data-template");
    await expect(checked).toHaveAttribute("data-template", second);
    await expect(cards.nth(1)).toBeFocused();
    await page.keyboard.press("ArrowLeft");
    await expect(checked).toHaveAttribute("data-template", "3d");
    // The gallery scrolls with the app's thin dark scrollbar.
    const bar = await dialog.locator(".np-templates").evaluate((el) => getComputedStyle(el).scrollbarWidth);
    expect(bar).toBe("thin");

    // Outside any repo, git is offered and checked; the plan says what happens.
    await name.fill("Space Race");
    await folder.fill(parent);
    await dialog.getByRole("textbox", { name: "Describe the game (optional)" }).fill(DESCRIBE);
    await expect(dialog.getByRole("checkbox", { name: "Track changes with git" })).toBeChecked();
    const plan = dialog.getByTestId("np-plan");
    await expect(plan).toContainText(`Create ${parent}/Space-Race`);
    await expect(plan).toContainText("Track changes with git");
    await expect(plan).toContainText("Start a task with your description");
    await expect(dialog).toContainText(`Folder: ${parent}/Space-Race`);
    await shot(page, `newproject-${tag}-form`);
    await step(page, width);

    // Create and start: the app switches to it, the game starts, the task is in the chat.
    await create.click();
    await step(page, width);
    await expect(dialog).toBeHidden({ timeout: 30_000 });
    await expect(page).toHaveTitle("Functor — Space-Race");
    await expect.poll(() => app.state.folder).toBe(`${parent}/Space-Race`);
    await expect.poll(() => app.state.session?.status.kind).toMatch(/starting|running/);
    expect(existsSync(join(parent, "Space-Race/functor.json")), "functor.json").toBe(true);
    expect(existsSync(join(parent, "Space-Race/.git")), "git init").toBe(true);
    await expect(page.locator("#chat-heading")).toHaveText(DESCRIBE);
    await expect(page.getByRole("log", { name: "Conversation" }).locator(".msg-user")).toHaveText([DESCRIBE]);
    await step(page, width);
    await expect.poll(() => app.state.tasks.every((t) => !t.run), { timeout: 15_000 }).toBe(true);
    await expect.poll(() => app.state.session?.status.kind, { timeout: 30_000 }).toBe("running");
    // Paused on its first frame (D37): the agent's run left the clock alone.
    await expect(page.getByRole("group", { name: "Transport" }).getByRole("button", { name: "Play" })).toBeVisible();
    await step(page, width);
    await shot(page, `newproject-${tag}-opened`);

    // The same folder again: what is in the way, and no way to create.
    await explorer.getByRole("button", { name: "New project…" }).click();
    await name.fill("Space Race");
    await folder.fill(parent);
    await expect(dialog.locator(".np-status")).toContainText(`${parent}/Space-Race is not empty:`);
    await expect(dialog.locator(".np-status")).toContainText("functor.json");
    await expect(create).toBeDisabled();
    // At 700 px the modal fits the width (it scrolls down, never across).
    const box = await dialog.boundingBox();
    expect(box.x, "modal left").toBeGreaterThanOrEqual(0);
    expect(box.x + box.width, "modal right").toBeLessThanOrEqual(width);
    await shot(page, `newproject-${tag}-taken`);
    await dialog.getByRole("button", { name: "Cancel" }).click();
    await expect(dialog).toBeHidden();

    expect(await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth), "horizontal overflow (px)").toBeLessThanOrEqual(0);
    expect(errors, "console errors").toEqual([]);
    expect(external, "requests to non-loopback hosts").toEqual([]);
    console.log(`ok: new project ${tag}`);
  } finally {
    await page.close();
    await stopServer(server);
  }
}

const browser = await chromium.launch();
try {
  await newProject(browser, 1440, 900);
  await newProject(browser, 700, 900);
} catch (e) {
  console.error(`FAIL: ${e.message}`);
  process.exitCode = 1;
} finally {
  await browser.close();
}
