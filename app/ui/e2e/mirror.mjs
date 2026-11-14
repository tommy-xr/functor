// The mirror (D5, D18), end to end with a GPU: `functor-app` with the device
// link on loopback, a `device-stub` headset answering `GET /mirror` with a
// recorded frame of examples/primitives, and the real `functor run vr` and
// `functor run native --mirror` children. Headset chosen → the Game tab
// streams the mirror (frames, rate, the headset's facts and the way back)
// → a parked camera changes the picture, Head brings it back → the page
// goes away: the headset is no longer polled → the link drops: the one fix
// (D27) → a headset that cannot mirror: why, over the last frame. Runs at
// 1440×900 and 700×900.
//
//   FUNCTOR_APP_GPU=1 FUNCTOR_BIN=target/release/functor node app/ui/e2e/mirror.mjs
//
// Skips without both (it needs a GL display). APP_UI_SHOTS=<dir> saves
// stills of each state.
import { chromium, expect } from "@playwright/test";
import { difference, FUNCTOR, requireBuilt, ROOT, shot, startServer, stopServer, stub, tempDir, tempProject, watchErrors } from "./app.mjs";

const PROJECT = "examples/primitives";
const RECORDED = `${ROOT}runtime/functor-runtime-desktop/tests/fixtures/primitives-t2.mirror.json`;

requireBuilt();
if (!FUNCTOR || process.env.FUNCTOR_APP_GPU !== "1") {
  console.log("SKIPPED: mirror — set FUNCTOR_APP_GPU=1 and FUNCTOR_BIN to a functor built from this checkout");
  process.exit(0);
}

async function mirror(browser, width, height) {
  const server = await startServer(0, { dir: tempProject(PROJECT), deviceLink: true, gpu: true });
  let page = await browser.newPage({ viewport: { width, height } });
  const app = { state: null };
  const watch = (p) =>
    p.on("websocket", (ws) =>
      ws.on("framereceived", ({ payload }) => {
        const msg = JSON.parse(payload);
        if (msg.type === "state") app.state = msg.state;
      }),
    );
  watch(page);
  const errors = watchErrors(page);
  const view = () => page.getByTestId("game-view");
  const camera = () => view().getByRole("group", { name: "Camera" });
  const picture = async () => view().locator("img").screenshot();
  const store = tempDir("functor-stub-");
  let quest;
  try {
    await page.goto(server.url);
    await expect.poll(() => app.state?.session?.status.kind, { timeout: 60_000 }).toBe("running");
    quest = stub(server.link, store, "Quest 3", ["--mirror", RECORDED]);
    await quest.line("code ");
    await page.getByTestId("pair-prompt").getByRole("button", { name: "Allow" }).click();
    await quest.line("linked ");

    // Headset: the game runs there and the Game tab mirrors it.
    await page.getByRole("group", { name: "Where the game shows" }).getByRole("button", { name: "Headset" }).click();
    await quest.line("request POST /load-project");
    await expect.poll(() => app.state?.session?.mirror?.kind, { timeout: 30_000 }).toBe("running");
    await quest.line("request GET /mirror");
    await expect(view().getByRole("status")).toHaveText("Mirrored from headset");
    await expect(view()).toContainText("Running on Quest 3");
    await expect(view()).toContainText(/Frame \d+ · Playing/);
    await expect(view()).toContainText(/\d+ fps · \d+ ms/, { timeout: 15_000 });
    await expect(view().locator("img")).toBeVisible();
    await expect(view().getByRole("button", { name: "Run it on this computer" })).toBeVisible();
    await expect(camera().getByRole("button", { name: "Head" })).toHaveAttribute("aria-pressed", "true");
    const head = await picture();
    await shot(page, `mirror-head-${width}`);

    // A parked camera: the same frame from above; Head brings the head view back.
    await camera().getByRole("button", { name: "Above" }).click();
    await expect(camera().getByRole("button", { name: "Above" })).toHaveAttribute("aria-pressed", "true");
    await expect.poll(async () => difference(page, head, await picture()), { timeout: 10_000 }).toBeGreaterThan(5);
    await shot(page, `mirror-above-${width}`);
    await camera().getByRole("button", { name: "Head" }).click();
    await expect.poll(async () => difference(page, head, await picture()), { timeout: 10_000 }).toBeLessThan(1);
    expect(errors, "console errors").toEqual([]);

    // Nobody looking: the headset is no longer polled.
    await page.close();
    await new Promise((r) => setTimeout(r, 1500));
    const polls = quest.count("request GET /mirror");
    await new Promise((r) => setTimeout(r, 1500));
    expect(quest.count("request GET /mirror"), "polls after the page left").toBe(polls);

    // Back, then the link drops: the one fix, no stale mirror.
    page = await browser.newPage({ viewport: { width, height } });
    watch(page);
    await page.goto(server.url);
    await expect(view()).toContainText(/\d+ fps/, { timeout: 15_000 });
    await quest.stop();
    await expect(view()).toContainText("Quest 3: Away");
    await expect(view()).toContainText("Put the headset on and open Functor");
    await shot(page, `mirror-away-${width}`);

    // A headset that cannot mirror (no `GET /mirror`): said, over the last frame.
    quest = stub(server.link, store, "Quest 3");
    await quest.line("linked ");
    await expect(view()).toContainText("No frames from the headset: it runs a Functor too old to mirror", {
      timeout: 15_000,
    });
    await expect(view()).toContainText("Running on Quest 3");
    await shot(page, `mirror-cannot-${width}`);
    await quest.stop();
  } finally {
    await page.close();
    await stopServer(server);
  }
}

const browser = await chromium.launch();
try {
  for (const [w, h] of [
    [1440, 900],
    [700, 900],
  ]) {
    await mirror(browser, w, h);
    console.log(`ok mirror at ${w}×${h}`);
  }
} finally {
  await browser.close();
}
