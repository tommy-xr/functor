// The decision page (D35), end to end against the fake agent in a throwaway
// copy of examples/counter, at 1440×900 and 700×900:
// - an easy change (one colour, a passing check, nothing to note): "Approve
//   at a glance", everything on the first page without a click, the review
//   card's verdict, and Keep;
// - a harder one (two hunks, a failed check, four notes): "Look into this",
//   the notes' disclosure, each item opening its tab, the tabs' keyboard;
// - a run that reports nothing: the size and verdict, no summary or notes;
// - late evidence: the predicted path (stored captures, as no GPU draws
//   them) held at "taking" over the WebSocket, then let through — nothing on
//   the page moves.
//
//   npm run test:app-ui
//
// APP_UI_SHOTS=<dir> saves stills of each case at both widths.
import { chromium, expect } from "@playwright/test";
import { requireBuilt, ROOT, shot, startServer, stopServer, storePaths, tempProject, watchErrors } from "./app.mjs";

const FIXTURES = `${ROOT}app/ui/e2e/fixtures`;
const WIDTHS = [
  [1440, 900],
  [700, 900],
];

requireBuilt();

/** Opens the only task's page from the Tasks pane. */
async function openTask(page) {
  const pane = page.getByRole("complementary", { name: "Tasks" });
  if (!(await pane.isVisible())) await page.getByRole("navigation", { name: "Views" }).getByRole("button", { name: /^Tasks/ }).click();
  await pane.getByTestId("task-row").first().click();
  await toPage(page);
}

/** Stacked (700 wide), the window scrolls to the page. */
const toPage = (page) => page.getByTestId("task-page").evaluate((el) => el.scrollIntoView({ block: "start" }));

/** Starts a fresh counter with `fixture`, asks once and waits for its review card. */
async function ask(browser, fixture, [width, height]) {
  const dir = tempProject("examples/counter");
  const server = await startServer(0, { dir, fakeAgent: `${FIXTURES}/${fixture}` });
  const page = await browser.newPage({ viewport: { width, height } });
  const errors = watchErrors(page);
  await page.goto(server.url);
  const input = page.locator(".chat").getByLabel("Message the agent");
  await input.fill("Change the counter");
  await input.press("Enter");
  await expect(page.locator(".chat").getByTestId("review-card")).toBeVisible({ timeout: 30_000 });
  return { dir, server, page, errors };
}

/** Puts the user's layout back once the run's walkthrough has opened beside it: the page gets the canvas. */
async function restoreLayout(page) {
  const restore = page.getByRole("button", { name: "Restore my layout" });
  await expect(restore).toBeVisible();
  await restore.click();
}

const first = (page) => page.getByTestId("first-page");
const noScroll = (page) => page.evaluate(() => document.documentElement.scrollWidth <= innerWidth);

async function easy(browser, size) {
  const { server, page, errors } = await ask(browser, "decision-easy.jsonl", size);
  try {
    await restoreLayout(page);
    const card = page.locator(".chat").getByTestId("review-card");
    await expect(card).toContainText("Review change · 1 file +1 −1 · game logic");
    await expect(card.getByTestId("review-verdict")).toHaveText("Approve at a glance 2 changed lines in one file. 1 check passes.");
    await card.getByRole("button", { name: "Open the decision page" }).click();
    const tab = page.getByRole("tablist", { name: "Decision page" });
    await expect(tab.getByRole("tab", { name: "Decision" })).toHaveAttribute("aria-selected", "true");
    // Everything is there without a click, and in view.
    const p = first(page);
    await expect(p.getByTestId("first-size")).toHaveText("+1 −1 1 file · game logic");
    await expect(p.getByTestId("first-verdict")).toContainText("Approve at a glance");
    await expect(p.getByTestId("first-summary")).toHaveText(/^The agent changed one colour value in game\.fun\. The cube is now amber\./);
    await expect(p.getByTestId("first-evidence")).toHaveText("functor test 3 expects pass");
    await expect(p.getByText("Nothing to note.")).toBeVisible();
    await toPage(page);
    await shot(page, `decision-easy-${size[0]}`);
    // At 1440 wide all of it is on screen at once; narrower, the panel scrolls.
    const ids = size[0] === 1440 ? ["first-size", "first-verdict", "first-summary", "first-evidence"] : ["first-size", "first-verdict"];
    for (const id of ids) await expect(p.getByTestId(id)).toBeInViewport();
    // No pair without a predicted path: no placeholder either.
    await expect(p.getByTestId("decision-pair")).toHaveCount(0);
    expect(await noScroll(page), "no horizontal scroll").toBe(true);

    // Keep, from the page's header: the review closes.
    await page.getByTestId("task-page").getByRole("button", { name: "Keep change" }).click();
    await expect(page.getByTestId("task-page-status")).toHaveText("Kept");
    await expect(tab).toHaveCount(0);
    await expect(page.getByTestId("task-change-closed")).toBeVisible();
    expect(errors, "console errors").toEqual([]);
  } finally {
    await page.close();
    await stopServer(server);
  }
}

async function hard(browser, size) {
  const { dir, server, page, errors } = await ask(browser, "decision-hard.jsonl", size);
  try {
    await openTask(page);
    const p = first(page);
    await expect(p.getByTestId("first-verdict")).toHaveText("Look into thisA check failed: functor test.");
    await expect(p.getByTestId("first-size")).toHaveText("+2 −2 1 file · game logic");
    const evidence = p.getByTestId("first-evidence").locator("li");
    await expect(evidence).toHaveText(["functor build no errors", "functor test 1 of 3 expects pass"]);
    await expect(evidence.nth(1)).toHaveClass(/first-line-danger/);
    // Three notes, the fourth behind the disclosure.
    const notes = p.getByTestId("first-notes").locator("li");
    await expect(notes).toHaveCount(3);
    const more = p.getByRole("button", { name: "Show 1 more" });
    await expect(more).toHaveAttribute("aria-expanded", "false");
    await shot(page, `decision-hard-${size[0]}`);
    await more.click();
    await expect(notes).toHaveCount(4);
    await expect(notes.nth(3)).toHaveText("The HUD text did not change.");
    await expect(p.getByRole("button", { name: "Show fewer" })).toHaveAttribute("aria-expanded", "true");

    // Each item opens the tab that backs it; focus lands on that tab.
    const tabs = page.getByRole("tablist", { name: "Decision page" });
    const selected = (name) => expect(tabs.getByRole("tab", { name, exact: true })).toHaveAttribute("aria-selected", "true");
    await p.locator(".first-size").getByRole("button", { name: "Open Changes" }).click();
    await selected("Changes");
    await expect(tabs.getByRole("tab", { name: "Changes" })).toBeFocused();
    const diff = page.getByTestId("depth-changes").getByTestId("diff-view");
    await expect(diff).toContainText("count: m.count + 2.0");
    await expect(diff.locator(".diff-head")).toHaveCount(0);
    await shot(page, `decision-changes-${size[0]}`);

    await tabs.getByRole("tab", { name: "Decision" }).click();
    await p.locator(".first-summary").getByRole("button", { name: "Open Highlights" }).click();
    await selected("Highlights");
    await expect(page.getByTestId("depth-highlights").locator(".walk-step")).toHaveCount(3);

    // Keyboard: arrows move and show, Home and End jump.
    await expect(tabs.getByRole("tab", { name: "Highlights" })).toBeFocused();
    await page.keyboard.press("ArrowRight");
    await selected("Captures");
    await expect(page.getByTestId("depth-captures")).toHaveText("There are no captures of this change.");
    await page.keyboard.press("End");
    await selected("Changes");
    await page.keyboard.press("ArrowRight");
    await selected("Decision");
    await page.keyboard.press("ArrowLeft");
    await selected("Changes");
    await page.keyboard.press("Home");
    await selected("Decision");
    await expect(tabs.getByRole("tab", { name: "Decision" })).toBeFocused();
    expect(await noScroll(page), "no horizontal scroll").toBe(true);
    expect(errors, "console errors").toEqual([]);
    return { dir, server };
  } finally {
    await page.close();
  }
}

async function bare(browser, size) {
  const { server, page, errors } = await ask(browser, "decision-bare.jsonl", size);
  try {
    await openTask(page);
    const p = first(page);
    await expect(p.getByTestId("first-verdict")).toHaveText("Look into thisThe agent gave no report.");
    await expect(p.getByTestId("first-size")).toHaveText("+1 −1 1 file · game logic");
    await expect(p.getByTestId("first-summary")).toHaveCount(0);
    await expect(p.getByRole("heading", { name: "Things to note" })).toHaveCount(0);
    await expect(p.locator(".first-list")).toContainText("No check was reported and no path was compared.");
    await shot(page, `decision-bare-${size[0]}`);
    expect(errors, "console errors").toEqual([]);
  } finally {
    await page.close();
    await stopServer(server);
  }
}

/**
 * The hard task again, its predicted path stored from the committed
 * captures (no GPU draws them here) and held at "taking" over the WebSocket
 * until let through, as when the paths are taken after the run: the pair's
 * frames are reserved, so nothing on the page moves when they arrive. Then
 * the pair and the path's evidence line each open Captures.
 */
async function late(browser, { dir, server }, [width, height]) {
  await stopServer(server);
  storePaths(server.home);
  server = await startServer(server.port, { dir, home: server.home, token: server.token });

  const page = await browser.newPage({ viewport: { width, height } });
  const errors = watchErrors(page);
  let hold = true;
  let toServer = null;
  await page.routeWebSocket(/\/ws/, (ws) => {
    toServer = ws.connectToServer();
    toServer.onMessage((text) => {
      const msg = JSON.parse(text);
      if (hold && msg.type === "state") {
        for (const t of msg.state.tasks) if (t.evidence) t.evidence.state = { kind: "taking" };
      }
      ws.send(JSON.stringify(msg));
    });
  });
  try {
    await page.goto(server.url);
    await restoreLayout(page);
    await openTask(page);
    const p = first(page);
    const pair = p.getByTestId("decision-pair");
    await expect(pair).toHaveAttribute("aria-busy", "true");
    await expect(pair).toContainText("Predicting…");
    await expect(pair).toContainText("Predicted path from frame 5, replaying the same 121 recorded input steps, under the old code and the new.");
    await expect(p.getByTestId("first-evidence").locator("li").last()).not.toHaveClass(/first-line-ok/);
    const boxes = () =>
      Promise.all(
        [pair.locator(".pair-frame").first(), p.getByTestId("first-summary"), p.getByTestId("first-evidence"), p.getByTestId("first-notes")].map(
          (l) => l.boundingBox(),
        ),
      );
    const held = await boxes();
    await shot(page, `decision-late-taking-${width}`);

    // Let the state through, as the shell would broadcast it.
    hold = false;
    toServer.send(JSON.stringify({ type: "get_state" }));
    await expect(pair).toHaveAttribute("aria-busy", "false");
    const imgs = pair.locator("img");
    await expect(imgs).toHaveCount(2);
    await expect.poll(() => imgs.evaluateAll((els) => els.every((i) => i.complete && i.naturalWidth > 0))).toBe(true);
    await expect(p.getByTestId("first-evidence").locator("li").last()).toHaveClass(/first-line-ok/);
    expect(await boxes(), "nothing moved when the captures arrived").toEqual(held);
    await shot(page, `decision-late-shown-${width}`);

    // The pair opens Captures, larger; so does the path's evidence line.
    const tabs = page.getByRole("tablist", { name: "Decision page" });
    await pair.getByRole("button", { name: /open on the Captures tab/ }).click();
    await expect(tabs.getByRole("tab", { name: "Captures" })).toHaveAttribute("aria-selected", "true");
    await expect(page.getByTestId("depth-captures").getByTestId("paths-section").getByRole("img")).toHaveCount(2);
    await shot(page, `decision-captures-${width}`);
    await tabs.getByRole("tab", { name: "Decision" }).click();
    await p.getByTestId("first-evidence").getByRole("button", { name: "Open Captures" }).click();
    await expect(tabs.getByRole("tab", { name: "Captures" })).toHaveAttribute("aria-selected", "true");
    expect(await noScroll(page), "no horizontal scroll").toBe(true);
    expect(errors, "console errors").toEqual([]);
  } finally {
    await page.close();
    await stopServer(server);
  }
}

const browser = await chromium.launch();
try {
  for (const size of WIDTHS) {
    await easy(browser, size);
    await bare(browser, size);
    await late(browser, await hard(browser, size), size);
  }
} finally {
  await browser.close();
}
console.log("ok: decision page");
