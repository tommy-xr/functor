// Voice in the chat. A message said in a headset (plan 4.3) shows as
// spoken: a task's stored first message is marked as said in "Quest 3" with
// its capsule (as the device link stores one), and after a restart the chat
// shows where it was said, with what the user was looking at on hover (the
// link's side is `cargo test -p functor-app --test device_link
// a_spoken_clip`). The composer's mic (D15) records (a tone standing in
// for the microphone), is transcribed by the app (its fake transcriber) and puts
// the words in the draft, unsent; an app without speech-to-text shows no
// mic. Runs at 1440×900 and 700×900.
//
//   npm run test:app-ui
//
// APP_UI_SHOTS=<dir> saves the spoken message at both widths.
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { chromium, expect } from "@playwright/test";
import { ROOT, reopen, requireBuilt, shot, startServer, stopServer, tempProject, watchErrors } from "./app.mjs";

const PROJECT = tempProject("examples/counter");
const FAIL = `${ROOT}app/ui/e2e/fixtures/fail.jsonl`;
const SAID = "Make the ball red";
const CONTEXT = 'at frame 1234 (20.5 s in), pointing at "ball"';

requireBuilt();

/** The only stored task's file under `home`. */
function taskFile(home) {
  const projects = join(home, "projects");
  for (const p of readdirSync(projects)) {
    const dir = join(projects, p, "tasks");
    const file = readdirSync(dir).find((f) => f.endsWith(".json"));
    if (file) return join(dir, file);
  }
  throw new Error("no stored task");
}

async function spoken(browser, width, height) {
  let server = await startServer(0, { dir: PROJECT, fakeAgent: FAIL });
  const page = await browser.newPage({ viewport: { width, height } });
  const errors = watchErrors(page);
  const dock = page.locator(".chat");
  try {
    await page.goto(server.url);
    const input = dock.getByLabel("Message the agent");
    await input.fill(SAID);
    await input.press("Enter");
    await expect(dock.locator(".msg-user")).toHaveText([SAID]);
    // Typed: no mark.
    await expect(dock.getByTestId("spoken")).toHaveCount(0);
    await expect(dock.getByTestId("result")).toBeVisible();

    await stopServer(server);
    const file = taskFile(server.home);
    const saved = JSON.parse(readFileSync(file, "utf8"));
    saved.transcript[0].spoken = { headset: "Quest 3", context: CONTEXT };
    writeFileSync(file, JSON.stringify(saved));
    server = await startServer(server.port, { dir: PROJECT, fakeAgent: FAIL, home: server.home, token: server.token });
    await reopen(page, server.url);
    await dock.getByRole("button", { name: "Recent tasks" }).click();
    await dock.getByRole("menu", { name: "Recent tasks" }).getByRole("menuitem", { name: new RegExp(SAID) }).click();

    const said = dock.getByTestId("spoken");
    await expect(said).toHaveText(`Said in Quest 3${SAID}`);
    await expect(said.locator(".spoken-from")).toHaveAttribute("title", CONTEXT);
    await shot(page, `voice-spoken-${width}`);
    expect(errors).toEqual([]);
  } finally {
    await page.close();
    stopServer(server);
  }
}

/** The composer's mic: hidden without speech-to-text; else a click records, the next puts the words in the draft. */
async function mic(browser, width, height) {
  const DRAFT = "Then";
  const HEARD = "make the ball red";
  let server = await startServer(0, { dir: PROJECT, fakeAgent: FAIL });
  const page = await browser.newPage({ viewport: { width, height } });
  // A tone for the microphone: headless Chromium's fake device never answers here.
  await page.addInitScript(() => {
    navigator.mediaDevices.getUserMedia = async () => {
      const context = new AudioContext();
      const tone = context.createOscillator();
      const out = context.createMediaStreamDestination();
      tone.connect(out);
      tone.start();
      return out.stream;
    };
  });
  const errors = watchErrors(page);
  const dock = page.locator(".chat");
  const speak = dock.getByRole("button", { name: "Speak a message" });
  try {
    await page.goto(server.url);
    await expect(dock.getByLabel("Message the agent")).toBeEnabled();
    // This build has no speech-to-text: no mic, rather than a dead one.
    await expect(speak).toHaveCount(0);
    stopServer(server);

    process.env.FUNCTOR_APP_FAKE_TRANSCRIPT = HEARD;
    server = await startServer(0, { dir: PROJECT, fakeAgent: FAIL });
    delete process.env.FUNCTOR_APP_FAKE_TRANSCRIPT;
    await page.goto(server.url);
    const input = dock.getByLabel("Message the agent");
    await input.fill(DRAFT);
    await speak.click();
    const done = dock.getByRole("button", { name: "Stop and transcribe" });
    await expect(done).toHaveAttribute("aria-pressed", "true");
    await shot(page, `voice-mic-recording-${width}`);
    await page.waitForTimeout(700);
    await done.click();
    // Added to the draft, never sent.
    await expect(input).toHaveValue(`${DRAFT} ${HEARD}`);
    await expect(speak).toBeEnabled();
    await expect(dock.locator(".msg-user")).toHaveCount(0);
    expect(errors).toEqual([]);
  } finally {
    await page.close();
    stopServer(server);
  }
}

const browser = await chromium.launch();
try {
  await spoken(browser, 1440, 900);
  await spoken(browser, 700, 900);
  await mic(browser, 1440, 900);
  await mic(browser, 700, 900);
  console.log("voice: ok");
} finally {
  await browser.close();
}
