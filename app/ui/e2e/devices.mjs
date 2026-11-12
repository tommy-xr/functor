// The Devices pane and the pairing prompt (D26, D27), end to end against
// `functor-app device-stub` processes playing headsets over the device link
// on loopback. Empty state → a stub asks to pair: the pill and the prompt
// show the code the stub printed → Allow → Connected with its details →
// the stub stops: Away → an old protocol: Out of date → a link that breaks:
// Unreachable → "Not mine" stores nothing → requests that arrive while the
// user types wait behind the pill, then queue one at a time → Forget.
// Runs at 1440×900 and 700×900.
//
//   npm run test:app-ui
//
// APP_UI_SHOTS=<dir> saves stills of each state and the prompt at both
// widths, and the frames of a pair → connected loop at 1440×900.
import { spawn } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { chromium, expect } from "@playwright/test";
import { APP, requireBuilt, shot, startServer, stopServer, tempProject, watchErrors, watchExternal, watchState } from "./app.mjs";

requireBuilt();

const dirs = [];
process.on("exit", () => dirs.forEach((d) => rmSync(d, { recursive: true, force: true })));
function tempDir(prefix) {
  dirs.push(mkdtempSync(join(tmpdir(), prefix)));
  return dirs.at(-1);
}

/** A stub headset; `line(prefix)` waits for its next line starting so. */
function stub(link, store, name, extra = []) {
  const child = spawn(APP, ["device-stub", "--desktop", link, "--name", name, "--store", store, ...extra], {
    stdio: ["ignore", "pipe", "ignore"],
  });
  const lines = [];
  let buf = "";
  child.stdout.on("data", (d) => {
    buf += d;
    for (let i; (i = buf.indexOf("\n")) >= 0; buf = buf.slice(i + 1)) lines.push(buf.slice(0, i));
  });
  let seen = 0;
  return {
    async line(prefix) {
      let found;
      await expect
        .poll(
          () => {
            const i = lines.slice(seen).findIndex((l) => l.startsWith(prefix));
            if (i < 0) return false;
            found = lines[seen + i];
            seen += i + 1;
            return true;
          },
          { timeout: 20_000, message: `stub ${name}: ${prefix}` },
        )
        .toBe(true);
      return found;
    },
    stop() {
      if (child.exitCode !== null) return Promise.resolve();
      return new Promise((resolve) => {
        child.on("exit", resolve);
        child.kill("SIGTERM");
      });
    },
  };
}

let frame = 0;
async function step(page, width) {
  if (width === 1440) await shot(page, `devices-loop-${String(frame++).padStart(2, "0")}`);
}

async function devicesView(browser, width, height) {
  const server = await startServer(0, { dir: tempProject("examples/counter"), deviceLink: true });
  const page = await browser.newPage({ viewport: { width, height } });
  const app = watchState(page);
  const errors = watchErrors(page);
  const external = watchExternal(page);
  const pill = page.getByTestId("headset-pill");
  const pane = page.getByRole("complementary", { name: "Devices" });
  const card = (name) => pane.getByTestId("device-card").filter({ hasText: name });
  const prompt = page.getByTestId("pair-prompt");
  const pairings = () => {
    try {
      return readFileSync(join(server.home, "devices.json"), "utf8");
    } catch {
      return "";
    }
  };
  try {
    await page.goto(server.url);
    await expect(pill).toHaveText("Headset · Not paired");
    await page.getByRole("navigation", { name: "Views" }).getByRole("button", { name: "Devices" }).click();
    await expect(pane.getByTestId("devices-empty")).toContainText("Open Functor in the headset on the same Wi-Fi");
    await expect(pane.getByTestId("devices-empty")).toContainText("cable");
    await shot(page, `devices-empty-${width}`);
    await step(page, width);

    // A headset asks: the prompt shows the code it shows.
    const questStore = tempDir("functor-stub-");
    const quest = stub(server.link, questStore, "Quest 3");
    const code = (await quest.line("code ")).slice("code ".length);
    await expect(prompt).toBeVisible();
    await expect(prompt.getByRole("heading")).toHaveText("Quest 3 wants to pair");
    await expect(prompt.getByTestId("pair-code")).toHaveText(`${code.slice(0, 3)} ${code.slice(3)}`);
    await expect(prompt).toContainText("Allow it only if the headset shows the same code");
    await expect(prompt).toContainText(/Expires in [12]:\d\d/);
    await expect(pill).toHaveText("Headset · Wants to pair");
    // Focus is on the dialog, not on Allow.
    await expect(prompt).toBeFocused();
    // Not dismissible by accident: Escape and a click outside leave it open, focused.
    await page.keyboard.press("Escape");
    await page.mouse.click(5, height - 5);
    await expect(prompt).toBeVisible();
    await expect(prompt).toBeFocused();
    await shot(page, `devices-prompt-${width}`);
    await step(page, width);
    await prompt.getByRole("button", { name: "Allow" }).click();
    await quest.line("paired ");
    await quest.line("linked ");
    await expect(prompt).toBeHidden();
    await expect(card("Quest 3")).toHaveAttribute("data-state", "connected");
    await expect(pill).toHaveText("Headset · Connected");
    await expect(card("Quest 3")).toContainText("80%");
    await expect(card("Quest 3")).toContainText("stub-0.1.0");
    await expect(card("Quest 3")).toContainText("examples/hello");
    await step(page, width);
    // Latency arrives with the first ping.
    await expect(card("Quest 3")).toContainText(/Wi-Fi · \d+ ms/, { timeout: 15_000 });
    await shot(page, `devices-connected-${width}`);
    await step(page, width);
    // The page never sees the secret.
    const secret = pairings().match(/"secret":"([0-9a-f]{64})"/)[1];
    expect(JSON.stringify(app.state)).not.toContain(secret);

    // It stops: away.
    await quest.stop();
    await expect(card("Quest 3")).toHaveAttribute("data-state", "away");
    await expect(pill).toHaveText("Headset · Away");
    await expect(card("Quest 3")).toContainText("It reconnects by itself");
    await shot(page, `devices-away-${width}`);

    // An old headset app: out of date, saying where to update.
    const old = stub(server.link, questStore, "Quest 3", ["--protocol", "0", "--once"]);
    await old.line("rejected Version");
    await expect(card("Quest 3")).toHaveAttribute("data-state", "needs_update");
    await expect(pill).toHaveText("Headset · Out of date");
    await expect(card("Quest 3")).toContainText("Update the Functor app on the headset");
    await shot(page, `devices-update-${width}`);
    await old.stop();

    // A link that breaks: unreachable.
    const broken = stub(server.link, questStore, "Quest 3", ["--misbehave", "oversize", "--once"]);
    await broken.line("linked ");
    await expect(card("Quest 3")).toHaveAttribute("data-state", "cannot_reach");
    await expect(pill).toHaveText("Headset · Unreachable");
    await expect(card("Quest 3")).toContainText("same Wi-Fi");
    await expect(card("Quest 3")).toContainText("Last error: message too large");
    await shot(page, `devices-unreachable-${width}`);
    await broken.stop();

    // Not mine: nothing stored, no card.
    const stranger = stub(server.link, tempDir("functor-stub-"), "Stranger", ["--once"]);
    await stranger.line("code ");
    await expect(prompt.getByRole("heading")).toHaveText("Stranger wants to pair");
    await prompt.getByRole("button", { name: "Not mine" }).click();
    await stranger.line("ended pairing was refused");
    await expect(prompt).toBeHidden();
    await expect(card("Stranger")).toHaveCount(0);
    expect(pairings()).not.toContain("Stranger");

    // Mid-sentence in the composer: requests wait behind an amber pill and
    // the composer keeps its focus and text.
    const composer = page.getByLabel("Message the agent");
    await composer.click();
    await composer.pressSequentially("make the cube spin fa");
    const one = stub(server.link, tempDir("functor-stub-"), "Quest Pro", ["--once"]);
    await one.line("code ");
    const two = stub(server.link, tempDir("functor-stub-"), "Quest 2", ["--once"]);
    await two.line("code ");
    await expect.poll(() => app.state?.devices.requests.length).toBe(2);
    await expect(prompt).toBeHidden();
    await expect(pill).toHaveClass(/pill-attention/);
    await expect(composer).toBeFocused();
    await expect(composer).toHaveValue("make the cube spin fa");
    await composer.pressSequentially("st");
    await expect(composer).toHaveValue("make the cube spin fast");
    await shot(page, `devices-waiting-${width}`, { fullPage: true });
    // The pill brings them, one at a time.
    await pill.click();
    await expect(prompt.getByRole("heading")).toHaveText("Quest Pro wants to pair");
    await expect(prompt).toContainText("1 more request is waiting");
    await prompt.getByRole("button", { name: "Not mine" }).click();
    await expect(prompt.getByRole("heading")).toHaveText("Quest 2 wants to pair");
    await prompt.getByRole("button", { name: "Not mine" }).click();
    await expect(prompt).toBeHidden();
    await Promise.all([one.line("ended"), two.line("ended")]);
    await expect(composer).toHaveValue("make the cube spin fast");

    // Forget, confirmed in place.
    await card("Quest 3").getByRole("button", { name: "Forget this headset" }).click();
    await expect(card("Quest 3").getByRole("group", { name: "Forget Quest 3" })).toBeVisible();
    await shot(page, `devices-forget-${width}`);
    await card("Quest 3").getByRole("button", { name: "Forget", exact: true }).click();
    await expect(card("Quest 3")).toHaveCount(0);
    await expect(pane.getByTestId("devices-empty")).toBeVisible();
    await expect(pill).toHaveText("Headset · Not paired");
    expect(pairings()).not.toContain("Quest 3");

    expect(errors, "console errors").toEqual([]);
    expect(external, "non-loopback requests from the page").toEqual([]);
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
    await devicesView(browser, w, h);
    console.log(`ok devices at ${w}×${h}`);
  }
} finally {
  await browser.close();
}
