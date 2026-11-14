// The separate review (D31) on the decision page, end to end against the
// fake agent and its fake review through the real tool server, in a
// throwaway copy of examples/counter, at 1440×900 and 700×900:
// - preparing → prepared: the page shows the agent's report and "Preparing
//   the summary…" at once, then the review's report, labelled;
// - a review that ends without a report: the agent's report stays, with
//   the reason;
// - Keep while it prepares: kept at once, the review stopped;
// - turned off in the composer's Review panel: the agent's report alone.
//
//   npm run test:app-ui
//
// APP_UI_SHOTS=<dir> saves stills of each case at both widths.
import { chromium, expect } from "@playwright/test";
import { requireBuilt, ROOT, shot, startServer, stopServer, tempProject, watchErrors } from "./app.mjs";

const FIXTURES = `${ROOT}app/ui/e2e/fixtures`;
const WIDTHS = [
  [1440, 900],
  [700, 900],
];
const AGENT_SUMMARY = /^The agent changed one colour value in game\.fun\./;

requireBuilt();

/** Starts a fresh counter whose runs replay `decision-easy` and reviews `review`. */
async function open(browser, review, [width, height]) {
  const dir = tempProject("examples/counter");
  const server = await startServer(0, {
    dir,
    fakeAgent: `${FIXTURES}/decision-easy.jsonl`,
    fakeReview: review && `${FIXTURES}/${review}`,
  });
  const page = await browser.newPage({ viewport: { width, height } });
  const errors = watchErrors(page);
  await page.goto(server.url);
  return { server, page, errors };
}

/** Asks once and opens the task's page once its change waits. */
async function ask(page) {
  const input = page.locator(".chat").getByLabel("Message the agent");
  await input.fill("Change the counter");
  await input.press("Enter");
  await expect(page.locator(".chat").getByTestId("review-card")).toBeVisible({ timeout: 30_000 });
  const pane = page.getByRole("complementary", { name: "Tasks" });
  if (!(await pane.isVisible())) await page.getByRole("navigation", { name: "Views" }).getByRole("button", { name: /^Tasks/ }).click();
  await pane.getByTestId("task-row").first().click();
  await page.getByTestId("task-page").evaluate((el) => el.scrollIntoView({ block: "start" }));
  return page.getByTestId("first-page");
}

async function prepared(browser, size) {
  const { server, page, errors } = await open(browser, "review-prepared.jsonl", size);
  try {
    const p = await ask(page);
    // At once: the size, the verdict so far, the agent's own report, and a quiet line.
    await expect(p.getByTestId("first-review-preparing")).toHaveText("Preparing the summary…");
    await expect(p.getByTestId("first-summary")).toHaveText(AGENT_SUMMARY);
    await expect(p.getByTestId("first-size")).toHaveText("+1 −1 1 file · game logic");
    await expect(page.getByTestId("task-page").getByRole("button", { name: "Keep change" })).toBeEnabled();
    await shot(page, `review-preparing-${size[0]}`);
    // Then the review's report, labelled, and the verdict on it.
    const by = p.getByTestId("first-review-by");
    await expect(by).toHaveText("Prepared by a separate review (claude · claude-review-model · 41.2k tokens in)", { timeout: 15_000 });
    await expect(p.getByTestId("first-review-preparing")).toHaveCount(0);
    await expect(p.getByTestId("first-summary")).toHaveText(/^The agent changed the colour of the cube in `game\.fun` from blue to amber\./);
    await expect(p.getByTestId("first-verdict")).toHaveText("Look into thisThe review notes 1 thing.");
    await expect(p.getByTestId("first-notes")).toHaveText("Nobody looked at the cube in the running game after the change.");
    await expect(p.getByTestId("first-evidence")).toHaveText("functor test 3 expects pass");
    await expect(p.getByTestId("first-style")).toHaveCount(0);
    await shot(page, `review-prepared-${size[0]}`);
    expect(errors, "console errors").toEqual([]);
  } finally {
    await page.close();
    await stopServer(server);
  }
}

async function silent(browser, size) {
  const { server, page, errors } = await open(browser, "review-silent.jsonl", size);
  try {
    const p = await ask(page);
    await expect(p.getByTestId("first-review-failed")).toHaveText(
      "The separate review did not finish: the review ended without a report.",
      { timeout: 15_000 },
    );
    // The page as it was: the agent's report and its verdict.
    await expect(p.getByTestId("first-summary")).toHaveText(AGENT_SUMMARY);
    await expect(p.getByTestId("first-verdict")).toContainText("Approve at a glance");
    await expect(p.getByTestId("first-review-by")).toHaveCount(0);
    await shot(page, `review-failed-${size[0]}`);
    expect(errors, "console errors").toEqual([]);
  } finally {
    await page.close();
    await stopServer(server);
  }
}

async function keptWhilePreparing(browser, size) {
  const { server, page, errors } = await open(browser, "review-slow.jsonl", size);
  try {
    const p = await ask(page);
    await expect(p.getByTestId("first-review-preparing")).toBeVisible();
    await page.getByTestId("task-page").getByRole("button", { name: "Keep change" }).click();
    // Kept at once, not after the review.
    await expect(page.getByTestId("task-page-status")).toHaveText("Kept", { timeout: 3_000 });
    await expect(page.getByTestId("first-page")).toHaveCount(0);
    expect(errors, "console errors").toEqual([]);
  } finally {
    await page.close();
    await stopServer(server);
  }
}

async function off(browser, size) {
  const { server, page, errors } = await open(browser, "review-prepared.jsonl", size);
  try {
    const toggle = page.locator(".chat").getByRole("button", { name: "Review on" });
    await toggle.click();
    const panel = page.getByRole("dialog", { name: "Review settings" });
    await expect(panel.getByLabel("Model")).toHaveAttribute("placeholder", "sonnet (default)");
    await shot(page, `review-settings-${size[0]}`);
    // It shows the core's setting: unchecked once the core has it.
    const on = panel.getByLabel("Prepare a separate review");
    await on.click();
    await expect(on).not.toBeChecked();
    await expect(page.locator(".chat").getByRole("button", { name: "Review off" })).toBeVisible();
    await expect(panel.getByLabel("Model")).toBeDisabled();
    await page.keyboard.press("Escape");
    const p = await ask(page);
    await expect(p.getByTestId("first-summary")).toHaveText(AGENT_SUMMARY);
    await expect(p.getByTestId("first-verdict")).toContainText("Approve at a glance");
    // Nothing is prepared, now or later.
    await page.waitForTimeout(1_500);
    await expect(p.locator(".first-review")).toHaveCount(0);
    expect(errors, "console errors").toEqual([]);
  } finally {
    await page.close();
    await stopServer(server);
  }
}

const browser = await chromium.launch();
try {
  for (const size of WIDTHS) {
    await prepared(browser, size);
    await silent(browser, size);
    await keptWhilePreparing(browser, size);
    await off(browser, size);
  }
} finally {
  await browser.close();
}
console.log("ok: review");
