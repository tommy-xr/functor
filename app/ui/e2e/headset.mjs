// The Headset placement (D21, D5, D27), end to end: `functor-app` with the
// device link on loopback, `device-stub` processes playing headsets, and the
// real `functor run vr` pushing to them through their tunnels. No Headset
// choice until one is connected → paired: Headset shows → chosen: the game
// runs there (the stub gets the load, its card names the project) and the
// Game tab says so with the clock read through the tunnel; pause works, the
// Extrapolate switch is not offered → the stub stops: Away with its fix, the
// choice kept → it relinks: Running again → a second headset: each by name →
// Pane: the headset is sent back to its boot scene and the game is here
// again, paused on frame 0 (D37). Runs at 1440×900 and 700×900.
//
//   FUNCTOR_BIN=target/debug/functor npm run test:app-ui
//
// Skips without FUNCTOR_BIN (it runs `functor run vr`). APP_UI_SHOTS=<dir>
// saves stills of each state at both widths.
import { chromium, expect } from "@playwright/test";
import { FUNCTOR, requireBuilt, shot, startServer, stopServer, stub, tempDir, tempProject, watchErrors, watchExternal, watchState } from "./app.mjs";

const PROJECT = "examples/counter";

requireBuilt();
if (!FUNCTOR) {
  console.log("SKIPPED: headset placement — set FUNCTOR_BIN (npm run build:cli:debug)");
  process.exit(0);
}

async function headset(browser, width, height) {
  const server = await startServer(0, { dir: tempProject(PROJECT), deviceLink: true });
  const page = await browser.newPage({ viewport: { width, height } });
  const app = watchState(page);
  const errors = watchErrors(page);
  const external = watchExternal(page);
  const control = page.getByRole("group", { name: "Where the game shows" });
  const view = page.getByTestId("game-view");
  const transport = page.getByRole("group", { name: "Transport" });
  const devices = page.getByRole("complementary", { name: "Devices" });
  const card = (name) => devices.getByTestId("device-card").filter({ hasText: name });
  const prompt = page.getByTestId("pair-prompt");
  const status = () => app.state?.session?.status;
  const game = () => app.state?.layouts[PROJECT]?.game;
  /** Pairs a stub called `name`, allowed from the prompt; it keeps running. */
  const pair = async (name, store) => {
    const quest = stub(server.link, store, name);
    await quest.line("code ");
    await prompt.getByRole("button", { name: "Allow" }).click();
    await quest.line("linked ");
    return quest;
  };
  try {
    await page.goto(server.url);
    // Here, paused on frame 0 (D37); no headset, no Headset choice.
    await expect.poll(() => status()?.kind, { timeout: 60_000 }).toBe("running");
    await expect(control.getByRole("button")).toHaveText(["Pane", "Mini", "Own window"]);

    const store = tempDir("functor-stub-");
    let quest = await pair("Quest 3", store);
    await expect(control.getByRole("button", { name: "Headset" })).toBeVisible();
    await shot(page, `headset-offered-${width}`);

    // Chosen: the game runs on the headset, which now owns it.
    const native = app.state.session.id;
    await control.getByRole("button", { name: "Headset" }).click();
    expect(await quest.line("request POST /load-project")).toContain("name=counter");
    await expect.poll(() => status()?.debug_url).toBe(app.state.devices.list[0].tunnel);
    expect(app.state.session.id).toBeGreaterThan(native);
    await expect(control.getByRole("button", { name: "Headset" })).toHaveAttribute("aria-pressed", "true");
    await expect(view).toContainText("Running on Quest 3");
    await expect(view.getByRole("status")).toHaveText("On the headset");
    await expect(page.getByText("Headset session · running")).toBeVisible();
    await expect(view).toContainText(/Frame \d+ · Playing/);
    await expect(transport).toBeVisible();
    await expect(transport.getByRole("button", { name: /Extrapolate/ })).toHaveCount(0);
    // Its card names the project with the next heartbeat.
    await page.getByRole("navigation", { name: "Views" }).getByRole("button", { name: "Devices" }).click();
    await expect(card("Quest 3")).toContainText("counter", { timeout: 15_000 });
    await shot(page, `headset-running-${width}`);
    await page.getByRole("navigation", { name: "Views" }).getByRole("button", { name: "Devices" }).click();

    // The transport drives the headset through its tunnel.
    await transport.getByRole("button", { name: "Pause" }).click();
    await quest.line("request POST /time");
    await expect(view).toContainText(/Frame \d+ · Paused/);
    await transport.getByRole("button", { name: "Play" }).click();
    await expect(view).toContainText(/Frame \d+ · Playing/);

    // The link drops: said, with its one fix; the choice is kept.
    await quest.stop();
    await expect(view).toContainText("Quest 3: Away");
    await expect(view).toContainText("Put the headset on and open Functor");
    await expect(control.getByRole("button", { name: "Headset" })).toHaveAttribute("aria-pressed", "true");
    await expect(transport).toHaveCount(0);
    expect(game().placement).toBe("headset");
    await shot(page, `headset-away-${width}`);

    // It relinks, its app restarted (a new stub): the same session loads
    // the game there again and follows it.
    const session = app.state.session.id;
    quest = stub(server.link, store, "Quest 3");
    await quest.line("linked ");
    expect(await quest.line("request POST /load-project")).toContain("name=counter");
    const project = (name) => app.state?.devices.list.find((d) => d.name === name)?.state.project;
    await expect.poll(() => project("Quest 3"), { timeout: 15_000 }).toBe("counter");
    await expect(view).toContainText("Running on Quest 3");
    await expect(view).toContainText(/Frame \d+ · Playing/);
    expect(app.state.session.id).toBe(session);

    // Two connected: each is a choice by name.
    const pro = await pair("Quest Pro", tempDir("functor-stub-"));
    await expect(control.getByRole("button")).toHaveText(["Pane", "Mini", "Own window", "Quest 3", "Quest Pro"]);
    await expect(control.getByRole("button", { name: "Quest 3" })).toHaveAttribute("aria-pressed", "true");
    await shot(page, `headset-two-${width}`);
    await control.getByRole("button", { name: "Quest Pro" }).click();
    await pro.line("request POST /load-project");
    await quest.line("request POST /load-project 2");
    await expect(view).toContainText("Running on Quest Pro");

    // Back here: the headset returns to its boot scene (an empty load), and
    // the game runs on this computer, paused on frame 0.
    await control.getByRole("button", { name: "Pane" }).click();
    await pro.line("request POST /load-project 2");
    await expect.poll(() => status()?.debug_url).not.toBe(app.state.devices.list[1].tunnel);
    await expect.poll(() => status()?.kind, { timeout: 60_000 }).toBe("running");
    await expect(transport.getByRole("button", { name: "Play" })).toBeVisible();
    await expect(view).not.toContainText("headset");
    expect(game().headset).toBeNull();
    // Neither headset runs it now, as their next heartbeats say.
    for (const name of ["Quest 3", "Quest Pro"]) {
      await expect.poll(() => project(name), { timeout: 15_000 }).toBeNull();
    }
    await shot(page, `headset-back-${width}`);

    expect(errors, "console errors").toEqual([]);
    expect(external, "non-loopback requests from the page").toEqual([]);
    await quest.stop();
    await pro.stop();
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
    await headset(browser, w, h);
    console.log(`ok headset at ${w}×${h}`);
  }
} finally {
  await browser.close();
}
