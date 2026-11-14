// Keyboard and mouse into the game view (1.4b), headless: the exact input
// the page sends (its socket's `input` messages, each one `POST /input` on
// the session) and what the session then holds. A captured-mouse game
// (examples/hello) gets the pointer locked (the native window's lock, its
// side played by the test) and relative moves; a visible-cursor one
// (examples/pointer-2d) absolute positions through the view. Each runs at
// 1440×900 and 700×900, in a pane and in Mini; then an agent's view opening
// beside the game must not take the user's input (D25).
// Needs FUNCTOR_BIN (npm run build:cli:debug).
import { chromium, expect } from "@playwright/test";
import { FUNCTOR, RELEASE_HINT, ROOT, requireBuilt, sessionInput, startServer, stopServer, tabs, tempProject, watchErrors, watchState } from "./app.mjs";

requireBuilt();
if (!FUNCTOR) {
  console.log("SKIPPED: game input — set FUNCTOR_BIN (npm run build:cli:debug)");
  process.exit(0);
}

const BESIDE = `${ROOT}app/ui/e2e/fixtures/views-beside.jsonl`;

/** The page's `input` messages, in order; `take()` returns those since the last take, once they stop. */
function watchInputs(page) {
  const sent = [];
  let seen = 0;
  page.on("websocket", (ws) =>
    ws.on("framesent", ({ payload }) => {
      const msg = JSON.parse(payload);
      if (msg.type === "input") sent.push(msg.input);
    }),
  );
  // Moves go once per animation frame, and frames are seen a little after
  // they go; wait for the sends to stop.
  const settle = async () => {
    let n = -1;
    while (n !== sent.length) {
      n = sent.length;
      await page.waitForTimeout(150);
    }
  };
  return {
    take: async () => {
      await settle();
      return sent.slice(seen, (seen = sent.length));
    },
    settle,
  };
}

const key = (k, down) => ({ type: "key", key: k, down });

/** Opens `project`, plays it, and returns what the scenarios need. */
async function open(browser, project, width, height, { before, ...extra } = {}) {
  const server = await startServer(0, { dir: project, ...extra });
  const page = await browser.newPage({ viewport: { width, height } });
  await before?.(page);
  const app = watchState(page);
  const errors = watchErrors(page);
  const inputs = watchInputs(page);
  await page.goto(server.url);
  await expect.poll(() => app.state?.session?.status.kind, { timeout: 60_000 }).toBe("running");
  // The pointer arrives with the first poll.
  await expect.poll(() => app.state.session.status.surface, { timeout: 30_000 }).not.toBeNull();
  const transport = page.getByRole("group", { name: "Transport" });
  await transport.getByRole("button", { name: "Play" }).click();
  await expect(transport.getByRole("button", { name: "Pause" })).toBeVisible();
  return { server, page, app, errors, inputs };
}

/** The game view in a pane, or moved to Mini. */
async function place(page, where) {
  if (where === "mini") {
    await page.locator(".pane-bar").getByRole("group", { name: "Where the game shows" }).getByRole("button", { name: "Mini" }).click();
    await expect(page.getByTestId("mini")).toBeVisible();
    return page.getByTestId("mini").getByTestId("game-view");
  }
  return page.getByTestId("game-view");
}

/** Where a page point lands in the game's surface: no GPU here, so no frame and the whole view maps. */
async function surfacePoint(view, app, x, y) {
  const r = await view.evaluate((el) => {
    const b = el.getBoundingClientRect();
    return { x: b.left + el.clientLeft, y: b.top + el.clientTop, w: el.clientWidth, h: el.clientHeight };
  });
  const [sw, sh] = app.state.session.status.surface;
  const to = (v, size) => Math.min(Math.max(Math.round(v * size), 0), size - 1);
  return { x: to((x - r.x) / r.w, sw), y: to((y - r.y) / r.h, sh) };
}

/**
 * The native window's pointer lock, from the page's side: WKWebView refuses
 * the Pointer Lock API (and headless Chromium does too), so the window holds
 * the cursor and hands the page deltas. Here the page gets the same hooks the
 * window installs — `functorNativePointer`, an `ipc` recording what it asks
 * — and the test plays the window, sending deltas to `functorPointer`.
 */
async function nativeWindow(page) {
  await page.addInitScript(() => {
    window.functorNativePointer = true;
    window.__pointer = [];
    window.ipc = {
      postMessage: (m) => {
        const msg = JSON.parse(m);
        if (msg.pointer) window.__pointer.push(msg.pointer);
      },
    };
  });
  return {
    asked: () => page.evaluate(() => window.__pointer.splice(0)),
    /** Each `[dx, dy]` in turn, within one task (so one animation frame). */
    move: (...deltas) => page.evaluate((deltas) => deltas.forEach(([dx, dy]) => window.functorPointer({ dx, dy })), deltas),
    lose: () => page.evaluate(() => window.functorPointer({ lost: true })),
  };
}

/** examples/hello: free-look, so the pointer locks and moves are relative. */
async function captured(browser, width, height, where) {
  const tag = `captured ${width}x${height} ${where}`;
  let shell;
  const { server, page, app, errors, inputs } = await open(browser, `${ROOT}examples/hello`, width, height, {
    before: async (page) => (shell = await nativeWindow(page)),
  });
  try {
    expect(app.state.session.status.pointer, tag).toBe("captured");
    const view = await place(page, where);
    const banner = view.getByRole("status");
    await expect(banner).toHaveText("Click to play");
    await view.scrollIntoViewIfNeeded();
    const box = await view.boundingBox();
    const [cx, cy] = [Math.round(box.x + box.width / 2), Math.round(box.y + box.height / 2)];
    await page.mouse.move(cx, cy);
    await inputs.settle();
    await inputs.take();
    // The click captures and asks the window for the lock; it is not forwarded.
    await page.mouse.click(cx, cy);
    await expect(banner).toHaveText(RELEASE_HINT);
    await expect(view).toHaveClass(/game-locked/);
    expect(await shell.asked(), `${tag}: lock`).toEqual(["lock"]);
    await inputs.settle();
    expect(await inputs.take(), `${tag}: the capturing click`).toEqual([]);

    // Locked, the pointer's own moves are ignored; the window's deltas move
    // a cursor from where it was clicked, unbounded.
    const from = await surfacePoint(view, app, cx, cy);
    // (The native lock holds the real cursor inside the view.)
    await page.mouse.move(cx + 8, cy + 6);
    await shell.move([30, -10]);
    await inputs.settle();
    await shell.move([-20, 0], [-2000, 0]);
    await inputs.settle();
    expect(await inputs.take(), `${tag}: relative moves`).toEqual([
      { type: "mouse_move", x: from.x + 30, y: from.y - 10 },
      // Two in one animation frame: one move.
      { type: "mouse_move", x: from.x - 1990, y: from.y - 10 },
    ]);
    await expect.poll(async () => {
      const { mouse } = await sessionInput(app.state.session.status.debug_url);
      return [mouse.x, mouse.y];
    }).toEqual([from.x - 1990, from.y - 10]);
    // Buttons and the wheel, at the locked cursor.
    await page.mouse.down({ button: "left" });
    await page.mouse.up({ button: "left" });
    await page.mouse.wheel(0, -100);
    await inputs.settle();
    expect(await inputs.take(), `${tag}: button and wheel`).toEqual([
      { type: "mouse_button", button: "left", down: true },
      { type: "mouse_button", button: "left", down: false },
      { type: "mouse_wheel", delta: 1 },
    ]);

    // Tab, function keys, Escape and modifiers reach the game, each press once.
    for (const code of ["Tab", "F1", "ShiftLeft", "ControlLeft", "AltLeft", "KeyW", "Minus", "Escape"]) await page.keyboard.down(code);
    // Auto-repeat and the other Shift add nothing.
    await page.keyboard.down("KeyW");
    await page.keyboard.down("ShiftRight");
    expect(await inputs.take(), `${tag}: presses`).toEqual(
      ["tab", "f1", "shift", "control", "alt", "w", "minus", "escape"].map((k) => key(k, true)),
    );
    await expect
      .poll(async () => (await sessionInput(app.state.session.status.debug_url)).held_keys)
      .toEqual(["W", "Escape", "Tab", "Shift", "Control", "Alt", "F1", "Minus"]);
    // The page keeps them: Tab did not move focus, Escape did not release.
    expect(await view.evaluate((el) => document.activeElement === el)).toBe(true);
    await expect(banner).toHaveText(RELEASE_HINT);
    await page.keyboard.up("F1");
    await page.keyboard.up("Escape");
    expect(await inputs.take()).toEqual([key("f1", false), key("escape", false)]);
    // ⌘ is the app's: going down it lets go of every held key.
    await page.keyboard.down("Meta");
    expect(await inputs.take(), `${tag}: ⌘ down`).toEqual(
      ["tab", "shift", "control", "alt", "w", "minus"].map((k) => key(k, false)),
    );
    // ⌘Esc releases, and the window lets go of the cursor.
    await page.keyboard.press("Escape");
    await page.keyboard.up("Meta");
    await expect(banner).toHaveText("Click to play");
    expect(await shell.asked(), `${tag}: unlock`).toEqual(["unlock"]);
    for (const code of ["Tab", "ShiftLeft", "ShiftRight", "ControlLeft", "AltLeft", "KeyW", "Minus"]) await page.keyboard.up(code);
    expect(await inputs.take(), `${tag}: after release`).toEqual([]);
    await expect.poll(async () => (await sessionInput(app.state.session.status.debug_url)).held_keys).toEqual([]);
    // Unlocked, deltas move nothing.
    await shell.move([5, 5]);
    await inputs.settle();
    expect(await inputs.take()).toEqual([]);

    // The window losing focus lets go of the cursor first, and says so:
    // everything held is released.
    await page.mouse.click(cx, cy);
    await expect(banner).toHaveText(RELEASE_HINT);
    await page.keyboard.down("KeyD");
    await page.mouse.down({ button: "right" });
    await shell.lose();
    await expect(banner).toHaveText("Click to play");
    await inputs.settle();
    expect(await inputs.take(), `${tag}: lost`).toEqual([
      key("d", true),
      { type: "mouse_button", button: "right", down: true },
      key("d", false),
      { type: "mouse_button", button: "right", down: false },
    ]);
    expect(await shell.asked()).toEqual(["lock", "unlock"]);
    await page.mouse.up({ button: "right" });
    await page.keyboard.up("KeyD");
    // And a blur of the page itself, as another app taking focus.
    await page.mouse.click(cx, cy);
    await page.keyboard.down("KeyS");
    await page.evaluate(() => window.dispatchEvent(new Event("blur")));
    await expect(banner).toHaveText("Click to play");
    await expect.poll(async () => (await sessionInput(app.state.session.status.debug_url)).held_keys).toEqual([]);
    await page.keyboard.up("KeyS");
    await inputs.settle();
    expect(await inputs.take(), `${tag}: blur`).toEqual([key("s", true), key("s", false)]);
    expect(await shell.asked()).toEqual(["lock", "unlock"]);
    expect(errors, tag).toEqual([]);
    console.log(`ok: ${tag}`);
  } finally {
    await page.close();
    await stopServer(server);
  }
}

/** In a browser, a refused lock (headless Chromium refuses it) leaves moves absolute. */
async function refusedLock(browser) {
  const tag = "refused lock";
  const { server, page, app, errors, inputs } = await open(browser, `${ROOT}examples/hello`, 1440, 900);
  try {
    const view = page.getByTestId("game-view");
    await view.scrollIntoViewIfNeeded();
    const box = await view.boundingBox();
    await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
    await expect(view.getByRole("status")).toHaveText(RELEASE_HINT);
    await expect(view).not.toHaveClass(/game-locked/);
    await inputs.settle();
    await inputs.take();
    const [x, y] = [Math.round(box.x + box.width * 0.25), Math.round(box.y + box.height * 0.75)];
    await page.mouse.move(x, y);
    await inputs.settle();
    expect(await inputs.take(), tag).toEqual([{ type: "mouse_move", ...(await surfacePoint(view, app, x, y)) }]);
    expect(errors, tag).toEqual([]);
    console.log(`ok: ${tag}`);
  } finally {
    await page.close();
    await stopServer(server);
  }
}
/** examples/pointer-2d: a visible cursor, so moves are absolute and Escape is the game's. */
async function visible(browser, width, height, where) {
  const tag = `visible ${width}x${height} ${where}`;
  const { server, page, app, errors, inputs } = await open(browser, `${ROOT}examples/pointer-2d`, width, height);
  try {
    expect(app.state.session.status.pointer, tag).toBe("visible");
    const view = await place(page, where);
    const banner = view.getByRole("status");
    await view.scrollIntoViewIfNeeded();
    const box = await view.boundingBox();
    await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
    await expect(banner).toHaveText(/⌘Esc to release|Meta\+Esc to release/);
    expect(await page.evaluate(() => document.pointerLockElement)).toBeNull();
    await inputs.settle();
    await inputs.take();
    // Each point maps through the view to the surface.
    for (const [fx, fy] of [[0.25, 0.6], [0.75, 0.3]]) {
      // Whole pixels, as mouse events report them.
      const [x, y] = [Math.round(box.x + fx * box.width), Math.round(box.y + fy * box.height)];
      await page.mouse.move(x, y);
      await inputs.settle();
      const at = await surfacePoint(view, app, x, y);
      expect(await inputs.take(), `${tag}: move to ${fx},${fy}`).toEqual([{ type: "mouse_move", ...at }]);
      await page.mouse.down();
      await page.mouse.up();
      // A press sends its own position first.
      expect(await inputs.take(), `${tag}: click at ${fx},${fy}`).toEqual([
        { type: "mouse_move", ...at },
        { type: "mouse_button", button: "left", down: true },
        { type: "mouse_button", button: "left", down: false },
      ]);
      await expect.poll(async () => {
        const { mouse } = await sessionInput(app.state.session.status.debug_url);
        return [mouse.x, mouse.y];
      }).toEqual([at.x, at.y]);
    }
    // Escape is the game's here; ⌘Esc still releases.
    await page.keyboard.press("Escape");
    expect(await inputs.take()).toEqual([key("escape", true), key("escape", false)]);
    await expect(banner).toHaveText(/to release/);
    await page.keyboard.down("Meta");
    await page.keyboard.press("Escape");
    await page.keyboard.up("Meta");
    await expect(banner).toHaveText("Click to play");
    expect(errors, tag).toEqual([]);
    console.log(`ok: ${tag}`);
  } finally {
    await page.close();
    await stopServer(server);
  }
}

/** An agent opening a view beside the game, mid-play, takes nothing from the user (D25). */
async function agentBeside(browser) {
  const tag = "agent beside";
  const { server, page, app, errors, inputs } = await open(browser, tempProject("examples/counter"), 1440, 900, { fakeAgent: BESIDE });
  try {
    const view = page.getByTestId("game-view");
    const banner = view.getByRole("status");
    const composer = page.getByLabel("Message the agent");
    await composer.fill("Show me functor.json");
    await composer.press("Enter");
    await view.click();
    await expect(banner).toHaveText(/to release/);
    await page.keyboard.down("KeyW");
    // The run's view opens beside, splitting the game's pane.
    await expect.poll(() => tabs(page), { timeout: 15_000 }).toEqual([["Game"], ["functor.json"]]);
    await page.waitForTimeout(300);
    await expect(banner).toHaveText(/to release/);
    expect(await view.evaluate((el) => document.activeElement === el), `${tag}: focus`).toBe(true);
    await page.keyboard.down("KeyD");
    await expect.poll(async () => (await sessionInput(app.state.session.status.debug_url)).held_keys).toEqual(["D", "W"]);
    await page.keyboard.up("KeyD");
    await page.keyboard.up("KeyW");
    await inputs.settle();
    expect(await inputs.take(), tag).toEqual([key("w", true), key("d", true), key("d", false), key("w", false)]);
    expect(errors, tag).toEqual([]);
    console.log(`ok: ${tag}`);
  } finally {
    await page.close();
    await stopServer(server);
  }
}

/** The session's model (`/state`). */
async function model(app) {
  return (await (await fetch(`${app.state.session.status.debug_url}/state`)).json()).model;
}

/**
 * On a GPU session (FUNCTOR_APP_GPU=1): free-look in examples/hello turns
 * the camera and changes the picture; in examples/pointer-2d a click on a
 * target's pixels in the stream selects that target. APP_UI_FRAMES=<dir>
 * saves numbered frames of the free-look for a GIF.
 */
async function gpu(browser) {
  if (process.env.FUNCTOR_APP_GPU !== "1") {
    console.log("SKIPPED: input on a GPU session — set FUNCTOR_APP_GPU=1");
    return;
  }
  const frames = process.env.APP_UI_FRAMES;
  let n = 0;
  {
    let shell;
    const { server, page, app, errors } = await open(browser, `${ROOT}examples/hello`, 1440, 900, {
      gpu: true,
      before: async (page) => (shell = await nativeWindow(page)),
    });
    try {
      const view = page.getByTestId("game-view");
      const img = view.locator("img");
      await expect(img).toBeVisible({ timeout: 60_000 });
      const box = await img.boundingBox();
      await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
      await expect(view).toHaveClass(/game-locked/);
      const before = await img.screenshot();
      const yaw0 = (await model(app)).yaw;
      // Look right in small steps, then walk forward.
      for (let i = 1; i <= 24; i++) {
        await shell.move([12, 0]);
        await page.waitForTimeout(40);
        if (frames) {
          const { mkdirSync, writeFileSync } = await import("node:fs");
          mkdirSync(frames, { recursive: true });
          writeFileSync(`${frames}/${String(n++).padStart(4, "0")}.png`, await page.screenshot());
        }
      }
      await expect.poll(async () => (await model(app)).yaw, { message: "the camera turned right" }).toBeLessThan(yaw0 - 0.1);
      await page.keyboard.down("KeyW");
      await page.waitForTimeout(500);
      await page.keyboard.up("KeyW");
      expect((await model(app)).eye.z, "walked forward").toBeGreaterThan(-5);
      expect(before.equals(await img.screenshot()), "the picture changed").toBe(false);
      expect(errors).toEqual([]);
      console.log("ok: gpu free-look");
    } finally {
      await page.close();
      await stopServer(server);
    }
  }
  {
    const { server, page, app, errors } = await open(browser, `${ROOT}examples/pointer-2d`, 1100, 900, { gpu: true });
    try {
      const view = page.getByTestId("game-view");
      const img = view.locator("img");
      await expect(img).toBeVisible({ timeout: 60_000 });
      // A frame of another aspect than the view: letterboxed, so the mapping matters.
      const box = await img.boundingBox();
      const viewBox = await view.boundingBox();
      expect(Math.abs(box.width / box.height - viewBox.width / viewBox.height), "letterboxed").toBeGreaterThan(0.05);
      await page.mouse.click(viewBox.x + 4, viewBox.y + viewBox.height / 2);
      // The targets' centres: world (5, -1) and (-1, -1) through Camera2D(20×12) at (2, -1), zoom 1.1,
      // fitted into the 4:3 surface — as fractions of the frame.
      const at = (wx) => [0.5 + ((wx - 2) * 1.1) / 20, 0.5];
      for (const [wx, index] of [[5, 1], [-1, 0]]) {
        const [fx, fy] = at(wx);
        await page.mouse.move(box.x + fx * box.width, box.y + fy * box.height);
        await page.mouse.down();
        await page.mouse.up();
        await expect.poll(async () => (await model(app)).selected, { message: `target ${index}` }).toBe(index);
      }
      expect(errors).toEqual([]);
      console.log("ok: gpu click lands where pointed");
    } finally {
      await page.close();
      await stopServer(server);
    }
  }
}

const browser = await chromium.launch();
try {
  for (const [w, h] of [[1440, 900], [700, 900]]) {
    for (const where of ["pane", "mini"]) {
      await captured(browser, w, h, where);
      await visible(browser, w, h, where);
    }
  }
  await refusedLock(browser);
  await agentBeside(browser);
  await gpu(browser);
} finally {
  await browser.close();
}
