// The instruction scope (D39), end to end against the fake agent: the
// composer's control shows the project's level; one request can read less,
// and that level reaches the runner; the project's default is kept across a
// restart; Codex offers only the full level, and says why; the review's
// panel says it always runs isolated. Runs at 1440×900 and 700×900.
//
//   npm run test:app-ui
//
// APP_UI_SHOTS=<dir> saves stills of the composer, the open panel (Claude,
// Codex, and a task that keeps its level) and the review's panel, at both
// widths.
import { chromium, expect } from "@playwright/test";
import { ROOT, reopen, requireBuilt, shot, startServer, stopServer, tempProject, watchErrors, watchExternal } from "./app.mjs";

const FIXTURE = `${ROOT}app/ui/e2e/fixtures/scope.jsonl`;
const PHASE = { timeout: 10_000 };

requireBuilt();

/** The levels the fake agent was run at, as it logs them. */
function setups(server) {
  const seen = [];
  server.child.stderr.on("data", (d) => {
    for (const m of String(d).matchAll(/\[fake agent\] setup: (\w+)/g)) seen.push(m[1]);
  });
  return seen;
}

async function scopeView(browser, width, height) {
  const dir = tempProject("examples/counter");
  let server = await startServer(0, { dir, fakeAgent: FIXTURE });
  let seen = setups(server);
  const page = await browser.newPage({ viewport: { width, height } });
  const errors = watchErrors(page);
  const external = watchExternal(page);
  const dock = page.locator(".chat");
  const input = dock.getByLabel("Message the agent");
  const toggle = dock.getByRole("button", { name: /^(Setup|Project only|Isolated)$/ });
  const panel = dock.getByRole("dialog", { name: "Instruction scope" });
  const level = (name) => panel.getByRole("radio", { name: new RegExp(`^${name}`) });
  // Disabled when the chat is already on a new task (after a reopen, say).
  const newTask = async () => {
    const button = dock.getByRole("button", { name: "New task", exact: true });
    if (await button.isEnabled()) await button.click();
    await expect(dock.locator("#chat-heading")).toHaveText("New task");
  };
  const ask = async (text) => {
    await input.fill(text);
    await input.press("Enter");
    await expect(dock.getByTestId("agent-stance")).toHaveText("Done", PHASE);
  };
  try {
    await page.goto(server.url);
    await expect(page.getByRole("tab", { name: "Game" })).toBeVisible();

    // The project's level: all of the user's setup.
    await expect(toggle).toHaveText("Setup");
    await shot(page, `scope-composer-${width}`);
    await toggle.click();
    await expect(level("Yours and the project's")).toBeChecked();
    await expect(panel).toContainText("nor the permission rules and hooks in your user settings");
    await expect(panel).toContainText("a shell command your settings allow is denied");
    await shot(page, `scope-panel-${width}`);

    // A new task reads less, and the runner gets that level.
    await level("Isolated").check();
    await expect(toggle).toHaveText("Isolated");
    await page.keyboard.press("Escape");
    await ask("Where does the count start?");
    await expect.poll(() => seen.at(-1)).toBe("Isolated");
    // The task keeps it, fixed (its session holds what it read); a new task
    // goes back to the project's.
    await expect(toggle).toHaveText("Isolated");
    await toggle.click();
    await expect(level("Yours and the project's")).toBeDisabled();
    await expect(panel).toContainText("A task keeps the level it started with");
    await shot(page, `scope-kept-${width}`);
    await page.keyboard.press("Escape");
    await newTask();
    await expect(toggle).toHaveText("Setup");

    // The project's default, kept across a restart.
    await toggle.click();
    await panel.getByLabel("Project default").selectOption("project");
    await expect(level("The project's only")).toBeChecked();
    await stopServer(server);
    server = await startServer(server.port, { dir, fakeAgent: FIXTURE, home: server.home, token: server.token });
    seen = setups(server);
    await reopen(page, server.url);
    await expect(page.getByRole("tab", { name: "Game" })).toBeVisible();
    await newTask();
    await expect(toggle).toHaveText("Project only");
    await ask("And now?");
    await expect.poll(() => seen.at(-1)).toBe("Project");

    // Codex: only the full level, and why.
    await newTask();
    await dock.getByLabel("Agent", { exact: true }).selectOption("codex");
    await expect(toggle).toHaveText("Setup");
    await toggle.click();
    await expect(level("Yours and the project's")).toBeChecked();
    await expect(level("The project's only")).toBeDisabled();
    await expect(level("Isolated")).toBeDisabled();
    await expect(panel).toContainText("Codex always reads your own instructions and skills");
    await shot(page, `scope-panel-codex-${width}`);
    await page.keyboard.press("Escape");
    await ask("Count?");
    await expect.poll(() => seen.at(-1)).toBe("UserAndProject");

    // The review's panel: isolated, as a fact.
    await dock.getByRole("button", { name: /^Review (on|off)$/ }).click();
    await expect(dock.getByRole("dialog", { name: "Review settings" })).toContainText("a review always runs isolated");
    await shot(page, `scope-review-${width}`);
    await page.keyboard.press("Escape");

    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
    expect(overflow, "horizontal overflow (px)").toBeLessThanOrEqual(0);
    expect(errors, "console errors").toEqual([]);
    expect(external, "requests to non-loopback hosts").toEqual([]);
    console.log(`ok: scope ${width}x${height}`);
  } finally {
    await page.close();
    await stopServer(server);
  }
}

const browser = await chromium.launch();
try {
  await scopeView(browser, 1440, 900);
  await scopeView(browser, 700, 900);
} catch (e) {
  console.error(`FAIL: ${e.message}`);
  process.exitCode = 1;
} finally {
  await browser.close();
}
