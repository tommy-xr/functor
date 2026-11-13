// Pairing over a USB cable (task 2.4) in the Devices pane, against a fake
// `adb` (e2e/fixtures/fake-adb.sh) playing one headset whose app storage is
// a `device-stub --wait-for-pairing`'s: each state that cannot pair says
// why → ready, and never paired by itself → one click pairs it, no code →
// the stub links: Connected, on the cable → paired with another desktop
// meanwhile: the card says so and asks before replacing → Forget clears the
// headset's pairing too. Runs at 1440×900 and 700×900.
//
//   npm run test:app-ui
//
// APP_UI_SHOTS=<dir> saves stills of each state at both widths.
import { chmodSync, copyFileSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { chromium, expect } from "@playwright/test";
import { ROOT, requireBuilt, shot, startServer, stopServer, stub, tempDir, tempProject, watchErrors, watchExternal, watchState } from "./app.mjs";

requireBuilt();

/** A fake adb in a temp dir, with one headset on the cable. */
function fakeAdb() {
  const dir = tempDir("functor-fake-adb-");
  const program = join(dir, "adb");
  copyFileSync(join(ROOT, "app/ui/e2e/fixtures/fake-adb.sh"), program);
  chmodSync(program, 0o755);
  mkdirSync(join(dir, "app/files"), { recursive: true });
  let transport = 0;
  const fake = {
    program,
    store: join(dir, "app/files"),
    set: (name, text) => writeFileSync(join(dir, name), text),
    // A new transport id each time, as a replug gives: the app looks again at once.
    devices: (state, details = "model:Quest_3") => fake.set("devices", `2G0FAKE\t${state} usb:1-1 ${details} transport_id:${++transport}\n`),
    flag: (name, on) => (on ? fake.set(name, "") : rmSync(join(dir, name), { force: true })),
    // The headset's pairing file; `{}` until its app first runs.
    file: () => {
      try {
        return JSON.parse(readFileSync(join(dir, "app/files/device-link.json"), "utf8"));
      } catch {
        return {};
      }
    },
    calls: () => readFileSync(join(dir, "calls"), "utf8"),
  };
  fake.set("pm", "package:dev.functor.runner\n");
  return fake;
}

async function cableView(browser, width, height) {
  const fake = fakeAdb();
  fake.devices("unauthorized", "");
  const server = await startServer(0, { dir: tempProject("examples/counter"), deviceLink: true, adb: fake.program });
  // The headset's app has run: it has made its id.
  let quest = stub(server.link, fake.store, "Quest 3", ["--wait-for-pairing"]);
  await expect.poll(() => fake.file().device).toMatch(/^[0-9a-f]{32}$/);
  const id = fake.file().device;
  const page = await browser.newPage({ viewport: { width, height } });
  const app = watchState(page);
  const errors = watchErrors(page);
  const external = watchExternal(page);
  const pill = page.getByTestId("headset-pill");
  const pane = page.getByRole("complementary", { name: "Devices" });
  const cable = pane.getByTestId("cable-card");
  const card = pane.getByTestId("device-card").filter({ hasText: "Quest 3" });
  const pairings = () => {
    try {
      return readFileSync(join(server.home, "devices.json"), "utf8");
    } catch {
      return "";
    }
  };
  try {
    await page.goto(server.url);
    await page.getByRole("navigation", { name: "Views" }).getByRole("button", { name: "Devices" }).click();

    // Each state that cannot pair: one line, one fix, no button.
    const pairButton = cable.getByRole("button", { name: "Pair over the cable" });
    for (const [state, fix, setup] of [
      ["unauthorized", "choose Allow on its USB debugging prompt", () => {}],
      ["no_app", "adb install", () => (fake.set("pm", ""), fake.devices("device"))],
      [
        "not_debuggable",
        "hold the left menu button",
        () => (fake.set("pm", "package:dev.functor.runner\n"), fake.flag("not-debuggable", true), fake.devices("device")),
      ],
    ]) {
      setup();
      await expect(cable).toHaveAttribute("data-state", state);
      await expect(cable).toContainText(fix);
      await expect(pairButton).toHaveCount(0);
      await shot(page, `cable-${state}-${width}`);
    }
    await expect(pill).toHaveText("Headset · Not paired");

    // Ready: offered, never done by itself.
    fake.flag("not-debuggable", false);
    fake.devices("device");
    await expect(cable).toHaveAttribute("data-state", "ready");
    await expect(cable).toContainText("needs no code");
    await expect(cable).toContainText("USB cable");
    await shot(page, `cable-ready-${width}`);
    await page.waitForTimeout(2500);
    expect(pairings()).not.toContain(id);
    expect(fake.file().secret).toBeUndefined();

    // One click: paired, no code, and the headset links by itself.
    await pairButton.click();
    await quest.line("linked ");
    await expect(card).toHaveAttribute("data-state", "connected");
    await expect(cable).toHaveCount(0);
    await expect(card).toContainText("Plugged in");
    await expect(card).toContainText(/Wi-Fi/);
    await expect(pill).toHaveText("Headset · Connected");
    await shot(page, `cable-connected-${width}`);
    const secret = fake.file().secret;
    expect(pairings()).toContain(secret);
    expect(JSON.stringify(app.state)).not.toContain(secret);
    expect(fake.calls()).not.toContain(secret);

    // Meanwhile another desktop paired it: the card says so, and asks first.
    await quest.stop();
    await expect(card).toHaveAttribute("data-state", "away");
    const elsewhere = { device: id, desktop: "ee".repeat(16), desktop_name: "Studio Mac", secret: "11".repeat(32) };
    writeFileSync(join(fake.store, "device-link.json"), JSON.stringify(elsewhere));
    fake.devices("device");
    await expect(card).toContainText("The headset is now paired with Studio Mac.");
    await shot(page, `cable-elsewhere-${width}`);
    await card.getByRole("button", { name: "Pair over the cable" }).click();
    const replace = card.getByRole("group", { name: "Replace the pairing with Studio Mac" });
    await expect(replace).toContainText("Studio Mac will no longer reach it");
    await shot(page, `cable-replace-${width}`);
    await replace.getByRole("button", { name: "Replace" }).click();
    await expect.poll(() => fake.file().desktop_name).not.toBe("Studio Mac");
    quest = stub(server.link, fake.store, "Quest 3", ["--wait-for-pairing"]);
    await quest.line("linked ");
    await expect(card).toHaveAttribute("data-state", "connected");
    await expect(card).not.toContainText("Studio Mac");

    // Forget: gone here, and its pairing gone from the headset (its id kept).
    await card.getByRole("button", { name: "Forget this headset" }).click();
    await card.getByRole("button", { name: "Forget", exact: true }).click();
    await expect(card).toHaveCount(0);
    await expect.poll(() => fake.file().secret).toBeUndefined();
    expect(fake.file().device).toBe(id);
    await expect(cable).toHaveAttribute("data-state", "ready");
    await expect(pairButton).toBeVisible();

    expect(errors, "console errors").toEqual([]);
    expect(external, "non-loopback requests from the page").toEqual([]);
  } finally {
    await page.close();
    await quest.stop();
    await stopServer(server);
  }
}

const browser = await chromium.launch();
try {
  for (const [w, h] of [
    [1440, 900],
    [700, 900],
  ]) {
    await cableView(browser, w, h);
    console.log(`ok cable at ${w}×${h}`);
  }
} finally {
  await browser.close();
}
