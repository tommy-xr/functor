// Universal debug-camera integration: live/paused 3D FPS navigation, pure-2D
// pan/zoom, exact reattachment, and the sandbox's replacement chrono bar.
//
// Prerequisite:
//   wasm-pack build runtime/functor-runtime-web --target=web
// Run:
//   node e2e/detached-camera.mjs
import { spawn, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const ROOT = fileURLToPath(new URL("..", import.meta.url));
const PORT = Number(process.env.FUNCTOR_SITE_PORT ?? 8123);
const BASE = `http://127.0.0.1:${PORT}`;
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const HANDSHAKE_OWNERSHIP_SOURCE = `
let init = { keyLeaked: false, mouseLeaked: false }
let input = (model, key, isDown) => { model with keyLeaked: true }
let mouseMove = (model, x, y) => { model with mouseLeaked: true }
let tick = (model, dt, tts) => model
let draw = (model, tts) =>
  Frame.create(
    Camera3D.lookAt(Vec3.make(0.0, 1.5, -5.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.cube() |> Scene.color(Color.rgb(0.1, 0.8, 0.9)))
let webview = (model) =>
  Html.div([], [
    Html.text(
      if model.keyLeaked then "KEY LEAKED"
      else if model.mouseLeaked then "MOUSE LEAKED"
      else "CLEAN")
  ])
`;
const INPUT_OWNERSHIP_SOURCE = `
let init = { held: false, leaked: false }
let input = (model, key, isDown) =>
  match key with
  | Key.W => { model with held: isDown }
  | _ => model
let mouseMove = (model, x, y) => { model with leaked: true }
let tick = (model, dt, tts) => model
let draw = (model, tts) =>
  Frame.create(
    Camera3D.lookAt(Vec3.make(0.0, 1.5, -5.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.cube() |> Scene.color(Color.rgb(0.1, 0.8, 0.9)))
let webview = (model) =>
  Html.div([], [Html.text(if model.held then "HELD" else if model.leaked then "LEAKED" else "UP")])
`;
const TWO_D_SOURCE = `
let init = 0.0
let tick = (model, dt, tts) => model
let draw = (model, tts) =>
  Frame.create2D(
    Camera2D.create(16.0, 9.0),
    Sprite.group([
      Sprite.rectangle(Color.rgb(0.1, 0.9, 0.8), 4.0, 2.0)
        |> Sprite.move(-3.0, 0.0),
      Sprite.circle(Color.rgb(1.0, 0.25, 0.65), 1.2)
        |> Sprite.move(3.0, 1.5),
      Sprite.square(Color.rgb(0.95, 0.85, 0.2), 1.5)
        |> Sprite.move(1.0, -2.0)
    ]))
`;

const build = spawnSync("node", ["site/build.mjs"], { cwd: ROOT, stdio: "inherit" });
if (build.status !== 0) process.exit(build.status ?? 1);

const server = spawn("node", ["site/serve.mjs", "--port", String(PORT)], {
  cwd: ROOT,
  stdio: "ignore",
});

try {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    try {
      const response = await fetch(BASE);
      if (response.ok) break;
    } catch {
      await sleep(100);
    }
    if (attempt === 99) throw new Error("site server never became ready");
  }

  const executablePath = process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH;
  let browser;
  try {
    browser = await chromium.launch(executablePath ? { executablePath } : {});
  } catch {
    browser = await chromium.launch({ channel: "chrome" });
  }
  try {
    const page = await browser.newPage({ viewport: { width: 960, height: 640 } });
    const pageErrors = [];
    page.on("pageerror", (error) => pageErrors.push(error.stack ?? String(error)));

    // The 3D debug camera is universal and available during playback.
    await page.goto(`${BASE}/player.html?game=examples/orbit.fun`);
    await page.waitForFunction(() => window.__scrub?.range().length === 2);
    const directCameraButton = page.locator("#scrub-camera");
    await directCameraButton.waitFor({ state: "visible" });
    if (await directCameraButton.isDisabled()) {
      throw new Error("debug camera must be available during playback");
    }

    // Pointer capture remains an explicit user gesture. A refusal must leave
    // both the camera and the control lifecycle attached/retryable.
    await page.evaluate(() => {
      const canvas = document.querySelector("#canvas");
      Object.defineProperty(canvas, "requestPointerLock", {
        configurable: true,
        value: () => Promise.reject(new DOMException("test refusal", "NotAllowedError")),
      });
    });
    await directCameraButton.click();
    await page.waitForFunction(() => !document.querySelector("#scrub-camera")?.disabled);
    if (await page.evaluate(() => window.__scrub.detached())) {
      throw new Error("a refused pointer lock must not activate the debug camera");
    }

    // Accept capture, move and change FOV while the game keeps advancing.
    await page.evaluate(() => {
      const canvas = document.querySelector("#canvas");
      Object.defineProperty(canvas, "requestPointerLock", {
        configurable: true,
        value: () => {
          Object.defineProperty(document, "pointerLockElement", {
            configurable: true,
            value: canvas,
          });
          return Promise.resolve();
        },
      });
      window.__debugStartFrame = window.__scrub.frame();
    });
    await directCameraButton.click();
    await page.waitForFunction(() => window.__scrub.detached());
    await page.waitForFunction(() => window.__scrub.frame() > window.__debugStartFrame + 2);
    const liveBefore = await page.locator("#canvas").screenshot();
    await page.evaluate(() => {
      window.__scrub.lookDetached(180, -40);
      window.__scrub.moveDetached(1, 1, 1, 0.05);
      window.__scrub.zoomDetached(2);
    });
    await page.waitForTimeout(200);
    const liveAfter = await page.locator("#canvas").screenshot();
    if (liveBefore.equals(liveAfter)) {
      throw new Error("3D FPS look/movement/FOV input did not change rendered pixels");
    }

    // The contextual drawer drives shell-only diagnostics. Material and
    // overlays compose (normals + physics + authored frustum), and switching
    // to orbit preserves the camera before rotating around its target.
    await page.waitForFunction(
      () => getComputedStyle(document.querySelector("#scrub-debug")).display !== "none"
    );
    if (
      !(await page.evaluate(() => {
        const modes = [...document.querySelector("#scrub-debug-mode").options].map(
          (option) => option.value
        );
        const materials = [...document.querySelector("#scrub-debug-material").options].map(
          (option) => option.value
        );
        const visible = (selector) =>
          document.querySelector(selector).getClientRects().length > 0;
        return (
          modes.join(",") === "0,1" &&
          materials.includes("3") &&
          !visible("#scrub-debug-pan2d") &&
          visible("#scrub-debug-material-label") &&
          visible("#scrub-debug-physics-label") &&
          visible("#scrub-debug-frustum-label")
        );
      }))
    ) {
      throw new Error("3D drawer exposed Pan 2D or hid applicable diagnostics");
    }
    await page.evaluate(() =>
      window.__scrub.setDebugCamera({ mode: 1, authoredFrustum: true })
    );
    await page.waitForFunction(() => {
      const debug = window.__scrub.debugCamera();
      return debug.mode === 1 && debug.authoredFrustum;
    });
    const frustumOnly = await page.locator("#canvas").screenshot();
    if (liveAfter.equals(frustumOnly)) {
      throw new Error("authored-camera frustum overlay did not change rendered pixels");
    }
    await page.evaluate(() =>
      window.__scrub.setDebugCamera({ material: 3, physics: true })
    );
    await page.waitForFunction(() => {
      const debug = window.__scrub.debugCamera();
      return debug.material === 3 && debug.physics;
    });
    const transparent = await page.locator("#canvas").screenshot();
    if (frustumOnly.equals(transparent)) {
      throw new Error("transparent debug material did not change rendered pixels");
    }
    await page.evaluate(() => window.__scrub.setDebugCamera({ material: 1 }));
    await page.waitForFunction(() => {
      const debug = window.__scrub.debugCamera();
      return (
        debug.mode === 1 &&
        debug.material === 1 &&
        debug.physics &&
        debug.authoredFrustum
      );
    });
    const diagnostic = await page.locator("#canvas").screenshot();
    if (frustumOnly.equals(diagnostic)) {
      throw new Error("normals debug material did not change rendered pixels");
    }
    await page.evaluate(() => window.__scrub.lookDetached(90, -25));
    await page.waitForTimeout(120);
    const orbitAfter = await page.locator("#canvas").screenshot();
    if (diagnostic.equals(orbitAfter)) {
      throw new Error("orbit mode did not rotate the detached camera");
    }
    await page.evaluate(() => window.__scrub.setDebugCamera({ reset: true, material: 0 }));
    await page.waitForFunction(() => window.__scrub.debugCamera().material === 0);

    // Pause and resume preserve the shell-owned view. Camera motion remains
    // available while pinned and the selected game frame itself does not move.
    // A high-polling mouse coalesces independently of the ordered timeline
    // queue, so its burst cannot starve the following pause.
    await page.evaluate(() => {
      for (let sample = 0; sample < 1024; sample += 1) {
        window.__scrub.lookDetached(0.01, 0);
      }
      window.__scrub.togglePause();
    });
    await page.waitForFunction(() => window.__scrub.paused() && window.__scrub.detached());
    const pinnedFrame = await page.evaluate(() => window.__scrub.frame());
    await page.evaluate(() => window.__scrub.moveDetached(-1, 0, -1, 0.05));
    await page.waitForTimeout(150);
    if ((await page.evaluate(() => window.__scrub.frame())) !== pinnedFrame) {
      throw new Error("debug navigation advanced the paused game frame");
    }
    await page.evaluate(() => window.__scrub.togglePause());
    await page.waitForFunction(
      (frame) =>
        !window.__scrub.paused() &&
        window.__scrub.detached() &&
        window.__scrub.frame() > frame,
      pinnedFrame
    );
    await page.evaluate(() => window.__scrub.togglePause());
    await page.waitForFunction(() => window.__scrub.paused() && window.__scrub.detached());

    // At one pinned frame, exiting the debug view must reproduce the authored
    // pixels exactly after arbitrary shell-owned movement. This is the browser
    // proof that the frame/model/replay camera was never mutated.
    await page.evaluate(() => window.__scrub.toggleDetached());
    await page.waitForFunction(() => !window.__scrub.detached());
    const authored = await page.locator("#canvas").screenshot();
    await page.evaluate(() => window.__scrub.toggleDetached());
    await page.waitForFunction(() => window.__scrub.detached());
    await page.evaluate(() => {
      window.__scrub.lookDetached(-140, 55);
      window.__scrub.moveDetached(1, -1, 1, 0.05);
      window.__scrub.zoomDetached(-2);
    });
    await page.waitForTimeout(150);
    const debugView = await page.locator("#canvas").screenshot();
    if (authored.equals(debugView)) {
      throw new Error("paused 3D debug navigation did not change rendered pixels");
    }
    await page.evaluate(() => window.__scrub.toggleDetached());
    await page.waitForFunction(() => !window.__scrub.detached());
    await page.waitForTimeout(100);
    const reattached = await page.locator("#canvas").screenshot();
    if (!authored.equals(reattached)) {
      throw new Error("reattaching did not restore the unchanged authored 3D frame");
    }

    // Same-task seek + activation snapshots the selected frame, never the
    // previously displayed one.
    await page.evaluate(() => {
      window.__debugTargetFrame = window.__scrub.range()[0];
      window.__scrub.seek(window.__debugTargetFrame);
      window.__scrub.toggleDetached();
    });
    await page.waitForFunction(
      () =>
        window.__scrub.detached() &&
        window.__scrub.frame() === window.__debugTargetFrame
    );
    await page.evaluate(() => window.__scrub.toggleDetached());
    await page.waitForFunction(() => !window.__scrub.detached());

    // A batch of future steps catches up over several render frames. Activation
    // must wait for the selected final frame rather than snapshotting the first
    // intermediate camera.
    await page.evaluate(() => {
      window.__debugStartFrame = window.__scrub.frame();
      window.__debugFutureFrame = window.__debugStartFrame + 60;
      window.__debugGeneration = window.__scrub.detachedGeneration();
      for (let frame = 0; frame < 60; frame += 1) window.__scrub.step();
      window.__scrub.toggleDetached();
      window.__debugCompletedEarly = false;
      const watchCatchUp = () => {
        if (window.__scrub.frame() >= window.__debugFutureFrame) return;
        if (
          window.__scrub.detached() ||
          window.__scrub.detachedGeneration() !== window.__debugGeneration
        ) {
          window.__debugCompletedEarly = true;
        }
        requestAnimationFrame(watchCatchUp);
      };
      requestAnimationFrame(watchCatchUp);
    });
    try {
      await page.waitForFunction(
        () =>
          window.__scrub.detached() &&
          window.__scrub.frame() === window.__debugFutureFrame
      );
    } catch (error) {
      const state = await page.evaluate(() => ({
        detached: window.__scrub.detached(),
        frame: window.__scrub.frame(),
        target: window.__debugFutureFrame,
        range: window.__scrub.range(),
        model: window.__scrub.model(),
      }));
      throw new Error(`future debug camera did not settle: ${JSON.stringify(state)}`, {
        cause: error,
      });
    }
    if (await page.evaluate(() => window.__debugCompletedEarly)) {
      throw new Error("debug camera activated before future-seek catch-up settled");
    }
    await page.evaluate(() => window.__scrub.toggleDetached());
    await page.waitForFunction(() => !window.__scrub.detached());

    // Saturating the bounded scrub queue must explicitly refuse a following
    // activation request so the DOM cannot wait forever for an acknowledgement.
    const saturatedDetach = await page.evaluate(() => {
      const generation = window.__scrub.detachedGeneration();
      for (let control = 0; control < 256; control += 1) window.__scrub.step();
      window.__scrub.toggleDetached();
      return {
        active: window.__scrub.detached(),
        acknowledged: window.__scrub.detachedGeneration() !== generation,
      };
    });
    if (saturatedDetach.active || !saturatedDetach.acknowledged) {
      throw new Error(
        `saturated debug-camera request was not refused: ${JSON.stringify(saturatedDetach)}`
      );
    }

    // Per-press ownership keeps the debug camera independent of game input.
    // A W press begun by the game stays game-owned through activation and gets
    // its real release; a later W press begun while detached is shell-only.
    const handshakeOwnership =
      Buffer.from(HANDSHAKE_OWNERSHIP_SOURCE).toString("base64url");
    await page.goto(`${BASE}/player.html?src=${handshakeOwnership}`);
    await page.waitForFunction(
      () => document.querySelector("#webview")?.shadowRoot?.textContent?.includes("CLEAN")
    );

    // Pointer lock is granted before the queued runtime toggle is acknowledged.
    // Shell ownership must cover that handshake: its first key/mouse events
    // cannot reach game hooks or the recorded input log.
    await page.evaluate(() => {
      const canvas = document.querySelector("#canvas");
      Object.defineProperty(canvas, "requestPointerLock", {
        configurable: true,
        value: () => ({
          then: (accepted) => {
            Object.defineProperty(document, "pointerLockElement", {
              configurable: true,
              value: canvas,
            });
            // Probe both halves of the handshake: while requestPointerLock is
            // pending, then again after the runtime detach is queued.
            const probe = () => {
              window.dispatchEvent(
                new KeyboardEvent("keydown", { key: "w", code: "KeyW", bubbles: true })
              );
              window.dispatchEvent(
                new KeyboardEvent("keyup", { key: "w", code: "KeyW", bubbles: true })
              );
              document.dispatchEvent(new MouseEvent("mousemove", { bubbles: true }));
            };
            probe();
            accepted();
            probe();
          },
        }),
      });
    });
    await page.locator("#scrub-camera").click();
    await page.waitForFunction(() => window.__scrub.detached());
    await page.evaluate(() => window.__scrub.setDebugCamera({ gameUi: false }));
    await page.waitForFunction(
      () =>
        !window.__scrub.debugCamera().gameUi &&
        !document.querySelector("#webview")?.shadowRoot?.textContent?.includes("CLEAN")
    );
    await page.evaluate(() => window.__scrub.setDebugCamera({ gameUi: true }));
    await page.waitForFunction(
      () =>
        window.__scrub.debugCamera().gameUi &&
        document.querySelector("#webview")?.shadowRoot?.textContent?.includes("CLEAN")
    );
    if (
      !(await page.evaluate(() =>
        document.querySelector("#webview")?.shadowRoot?.textContent?.includes("CLEAN")
      ))
    ) {
      throw new Error("debug-camera activation handshake leaked input into the game model");
    }
    await page.evaluate(() => {
      delete document.pointerLockElement;
      window.__scrub.toggleDetached();
    });
    await page.waitForFunction(
      () => !window.__scrub.detached() && !window.__scrub.ownsDetachedInput()
    );

    const inputOwnership = Buffer.from(INPUT_OWNERSHIP_SOURCE).toString("base64url");
    await page.goto(`${BASE}/player.html?src=${inputOwnership}`);
    await page.waitForFunction(
      () => document.querySelector("#webview")?.shadowRoot?.textContent?.includes("UP")
    );
    await page.keyboard.down("w");
    await page.waitForFunction(
      () => document.querySelector("#webview")?.shadowRoot?.textContent?.includes("HELD")
    );
    await page.evaluate(() => {
      const canvas = document.querySelector("#canvas");
      Object.defineProperty(canvas, "requestPointerLock", {
        configurable: true,
        value: () => Promise.resolve(),
      });
    });
    await page.locator("#scrub-camera").click();
    await page.waitForFunction(() => window.__scrub.detached());
    await page.evaluate(() => {
      Object.defineProperty(document, "pointerLockElement", {
        configurable: true,
        value: document.querySelector("#canvas"),
      });
    });
    if (
      !(await page.evaluate(() =>
        document.querySelector("#webview")?.shadowRoot?.textContent?.includes("HELD")
      ))
    ) {
      throw new Error("activating the debug camera stole a game-owned key press");
    }
    await page.keyboard.up("w");
    await page.waitForFunction(
      () => document.querySelector("#webview")?.shadowRoot?.textContent?.includes("UP")
    );
    await page.keyboard.down("w");
    await page.waitForTimeout(100);
    const debugKeyState = await page.evaluate(() => ({
      modelUp: document
        .querySelector("#webview")
        ?.shadowRoot?.textContent?.includes("UP"),
      detached: window.__scrub.detached(),
      ownsDetachedInput: window.__scrub.ownsDetachedInput(),
      pointerLocked:
        document.pointerLockElement === document.querySelector("#canvas"),
    }));
    if (!debugKeyState.modelUp) {
      throw new Error(
        `a debug-owned key press reached the game model: ${JSON.stringify(debugKeyState)}`
      );
    }
    await page.keyboard.up("w");
    await page.evaluate(() => window.__scrub.toggleDetached());
    await page.waitForFunction(() => !window.__scrub.detached());

    // A text field focused before detaching must not retain WASD/QE ownership
    // after the debug camera takes pointer lock. Keep a real shadow-DOM input
    // focused while activating through the seam and verify W moves the view
    // instead of typing into the field.
    await page.evaluate(() => {
      const canvas = document.querySelector("#canvas");
      const input = document.createElement("input");
      input.id = "debug-focus-probe";
      input.setAttribute("data-fn-input", "0");
      document.querySelector("#webview").shadowRoot.append(input);
      input.focus();
      Object.defineProperty(document, "pointerLockElement", {
        configurable: true,
        value: canvas,
      });
      window.__scrub.toggleDetached();
    });
    await page.waitForFunction(
      () =>
        window.__scrub.detached() &&
        document.querySelector("#webview")?.shadowRoot?.activeElement?.id ===
          "debug-focus-probe"
    );
    const focusedBefore = await page.locator("#canvas").screenshot();
    await page.keyboard.down("w");
    await page.waitForTimeout(180);
    await page.keyboard.up("w");
    const focusedAfter = await page.locator("#canvas").screenshot();
    if (focusedBefore.equals(focusedAfter)) {
      throw new Error("focused text input blocked debug-camera navigation");
    }
    if (
      await page.evaluate(
        () =>
          document
            .querySelector("#webview")
            ?.shadowRoot?.getElementById("debug-focus-probe")?.value !== ""
      )
    ) {
      throw new Error("debug-camera navigation typed into a focused text input");
    }
    // Pointer unlock preserves the detached view but hands navigation letters
    // back to a focused field until the viewport is recaptured.
    await page.evaluate(() => {
      Object.defineProperty(document, "pointerLockElement", {
        configurable: true,
        value: null,
      });
      document.dispatchEvent(new Event("pointerlockchange"));
    });
    if (
      await page.evaluate(
        () => document.pointerLockElement === document.querySelector("#canvas")
      )
    ) {
      throw new Error("focused-field unlock probe still had pointer capture");
    }
    await page.keyboard.type("wq");
    if (
      await page.evaluate(
        () =>
          document
            .querySelector("#webview")
            ?.shadowRoot?.getElementById("debug-focus-probe")?.value !== "wq"
      )
    ) {
      throw new Error("unlocked debug camera still swallowed focused-field text");
    }
    if (
      !(await page.evaluate(() =>
        document.querySelector("#webview")?.shadowRoot?.textContent?.includes("UP")
      ))
    ) {
      throw new Error("unlocked focused-field input leaked into the game model");
    }
    await page.evaluate(() => {
      document
        .querySelector("#webview")
        ?.shadowRoot?.getElementById("debug-focus-probe")
        ?.remove();
      window.__scrub.toggleDetached();
    });
    await page.waitForFunction(() => !window.__scrub.detached());

    // Pure Frame.create2D content selects pan/zoom. As in 3D, exiting must
    // reproduce the untouched authored frame exactly.
    const twoD = Buffer.from(TWO_D_SOURCE).toString("base64url");
    await page.goto(`${BASE}/player.html?src=${twoD}`);
    // Tooltips are page pixels: a cursor parked on a bar button (left there
    // by an earlier scenario's click) would leak hover chrome into the
    // byte-equality captures below. Park it over inert canvas instead.
    await page.mouse.move(480, 420);
    await page.waitForFunction(() => window.__scrub?.range().length === 2);
    await page.evaluate(() => window.__scrub.togglePause());
    await page.waitForFunction(() => window.__scrub.paused());
    const authored2d = await page.locator("#canvas").screenshot();
    await page.evaluate(() => window.__scrub.toggleDetached());
    await page.waitForFunction(() => window.__scrub.detached());
    await page.waitForFunction(() => {
      const modes = [...document.querySelector("#scrub-debug-mode").options].map(
        (option) => option.value
      );
      const visible = (selector) =>
        document.querySelector(selector).getClientRects().length > 0;
      return (
        modes.join(",") === "0,1" &&
        !visible("#scrub-debug-mode") &&
        visible("#scrub-debug-pan2d") &&
        !visible("#scrub-debug-material-label") &&
        !visible("#scrub-debug-physics-label") &&
        !visible("#scrub-debug-frustum-label")
      );
    });
    if (
      !(await page.evaluate(() => {
        const debug = window.__scrub.debugCamera();
        return debug.mode === 2 && debug.fov < 0 && debug.zoom2d > 0;
      }))
    ) {
      throw new Error("pure 2D drawer did not isolate Pan 2D/zoom controls");
    }
    await page.evaluate(() => {
      window.__scrub.lookDetached(120, -55);
      window.__scrub.moveDetached(1, 1, 0, 0.05);
      window.__scrub.zoomDetached(3);
    });
    await page.waitForTimeout(150);
    const debug2d = await page.locator("#canvas").screenshot();
    if (authored2d.equals(debug2d)) {
      throw new Error("2D pan/zoom did not change rendered pixels");
    }
    await page.evaluate(() => window.__scrub.toggleDetached());
    await page.waitForFunction(() => !window.__scrub.detached());
    await page.waitForTimeout(100);
    const reattached2d = await page.locator("#canvas").screenshot();
    if (!authored2d.equals(reattached2d)) {
      throw new Error("reattaching did not restore the unchanged authored 2D frame");
    }

    // Physics wireframes use the same shared line pass on web as native.
    await page.goto(`${BASE}/player.html?game=examples/bounce.fun`);
    await page.waitForFunction(() => window.__scrub?.range().length === 2);
    await page.evaluate(() => {
      window.__scrub.togglePause();
      window.__scrub.toggleDetached();
    });
    await page.waitForFunction(
      () => window.__scrub.paused() && window.__scrub.detached()
    );
    await page.evaluate(() => {
      window.__scrub.lookDetached(120, -35);
      window.__scrub.moveDetached(-1, 1, 1, 0.05);
    });
    await page.waitForTimeout(120);
    const physicsOff = await page.locator("#canvas").screenshot();
    await page.evaluate(() => window.__scrub.setDebugCamera({ physics: true }));
    await page.waitForFunction(() => window.__scrub.debugCamera().physics);
    await page.waitForTimeout(120);
    const physicsOn = await page.locator("#canvas").screenshot();
    if (physicsOff.equals(physicsOn)) {
      throw new Error("web physics debug overlay did not change rendered pixels");
    }
    await page.evaluate(() => window.__scrub.setDebugCamera({ material: 3 }));
    await page.waitForFunction(() => window.__scrub.debugCamera().material === 3);
    await page.waitForTimeout(120);
    const transparentPhysics = await page.locator("#canvas").screenshot();
    if (physicsOn.equals(transparentPhysics)) {
      throw new Error("transparent material did not reveal a distinct physics debug view");
    }

    // The sandbox's replacement control is also universal and live-capable.
    await page.goto(`${BASE}/sandbox.html?example=orbit`);
    await page.waitForFunction(() => {
      const iframe = document.querySelector("#player");
      return iframe?.contentWindow?.__scrub?.range().length === 2;
    });
    const cameraButton = page.locator("#mp-camera");
    await cameraButton.waitFor({ state: "visible" });
    if (await cameraButton.isDisabled()) {
      throw new Error("sandbox debug camera must be enabled during playback");
    }
    await page.evaluate(() => {
      const canvas = document.querySelector("#player")?.contentDocument?.querySelector("#canvas");
      Object.defineProperty(canvas, "requestPointerLock", {
        configurable: true,
        value: () => Promise.reject(new DOMException("test refusal", "NotAllowedError")),
      });
    });
    await cameraButton.click();
    await page.waitForFunction(() => !document.querySelector("#mp-camera")?.disabled);
    if (
      await page.evaluate(
        () => document.querySelector("#player")?.contentWindow?.__scrub?.detached()
      )
    ) {
      throw new Error("sandbox must not activate when pointer lock is refused");
    }
    await page.evaluate(
      () => delete document.querySelector("#player")?.contentDocument?.querySelector("#canvas")
        ?.requestPointerLock
    );
    const sandboxStart = await page.evaluate(
      () => document.querySelector("#player")?.contentWindow?.__scrub?.frame()
    );
    await page.waitForFunction(
      (frame) => document.querySelector("#player")?.contentWindow?.__scrub?.frame() > frame,
      sandboxStart
    );
    await page.locator("#mp-pause").click();
    await page.waitForFunction(
      () => document.querySelector("#player")?.contentWindow?.__scrub?.paused()
    );
    await cameraButton.click();
    await page.waitForFunction(
      () => document.querySelector("#player")?.contentWindow?.__scrub?.detached()
    );
    await page.evaluate(() =>
      document
        .querySelector("#player")
        ?.contentWindow?.__scrub?.setDebugCamera({ mode: 1, authoredFrustum: true })
    );
    await page.waitForFunction(() => {
      const iframe = document.querySelector("#player");
      const debug = iframe?.contentWindow?.__scrub?.debugCamera();
      return (
        !document.querySelector(".mp-debug")?.hidden &&
        debug?.mode === 1 &&
        debug?.authoredFrustum
      );
    });
    if (
      !(await page.evaluate(() => {
        const modes = [...document.querySelector("#mp-debug-mode").options].map(
          (option) => option.value
        );
        const materials = [...document.querySelector("#mp-debug-material").options].map(
          (option) => option.value
        );
        const visible = (selector) =>
          document.querySelector(selector).getClientRects().length > 0;
        return (
          modes.join(",") === "0,1" &&
          materials.includes("3") &&
          !visible("#mp-debug-pan2d")
        );
      }))
    ) {
      throw new Error("sandbox drawer exposed Pan 2D in 3D or omitted transparency");
    }
    if (
      !(await page.evaluate(() => {
        const iframe = document.querySelector("#player");
        return (
          document.activeElement === iframe &&
          iframe?.contentDocument?.activeElement?.id === "canvas"
        );
      }))
    ) {
      throw new Error("sandbox debug-camera activation did not focus the player canvas");
    }
    if (
      !(await page.evaluate(() =>
        document
          .querySelector("#player")
          ?.contentDocument?.querySelector("#capture-hint")
          ?.classList.contains("visible")
      ))
    ) {
      throw new Error("sandbox pointer capture did not show the Escape release hint");
    }
    // The camera button belongs to the sandbox host, not the player iframe.
    // Pointer lock alone does not transfer keyboard focus across browsing
    // contexts: activation must focus the pane so real WASD/QE events reach
    // its runtime-owned movement sampler.
    const sandboxCanvas = page.frameLocator("#player").locator("#canvas");
    const sandboxBeforeKeys = await sandboxCanvas.screenshot();
    await page.keyboard.down("w");
    await page.waitForTimeout(180);
    await page.keyboard.up("w");
    const sandboxAfterForward = await sandboxCanvas.screenshot();
    if (sandboxBeforeKeys.equals(sandboxAfterForward)) {
      throw new Error("sandbox debug camera did not receive real W keyboard movement");
    }
    await page.keyboard.down("e");
    await page.waitForTimeout(180);
    await page.keyboard.up("e");
    const sandboxAfterVertical = await sandboxCanvas.screenshot();
    if (sandboxAfterForward.equals(sandboxAfterVertical)) {
      throw new Error("sandbox debug camera did not receive real Q/E-axis keyboard movement");
    }
    await page.evaluate(() =>
      document.querySelector("#player")?.contentDocument?.exitPointerLock()
    );
    await cameraButton.click();
    await page.waitForFunction(
      () => !document.querySelector("#player")?.contentWindow?.__scrub?.detached()
    );

    // The IDE's camera control lives inside its player iframe, so its click
    // already focuses the correct browsing context. Pin that parity with the
    // same real-key movement check rather than relying on direct seam calls.
    await page.goto(`${BASE}/ide.html`);
    await page.waitForFunction(() => {
      const iframe = document.querySelector("#player");
      return iframe?.contentWindow?.__scrub?.range().length === 2;
    });
    const ideFrame = page.frameLocator("#player");
    const ideCameraButton = ideFrame.locator("#scrub-camera");
    await ideCameraButton.waitFor({ state: "visible" });
    await ideFrame.locator("#scrub-pause").click();
    await page.waitForFunction(
      () => document.querySelector("#player")?.contentWindow?.__scrub?.paused()
    );
    await ideCameraButton.click();
    await page.waitForFunction(
      () => document.querySelector("#player")?.contentWindow?.__scrub?.detached()
    );
    const ideCanvas = ideFrame.locator("#canvas");
    const ideBeforeKeys = await ideCanvas.screenshot();
    await page.keyboard.down("w");
    await page.waitForTimeout(180);
    await page.keyboard.up("w");
    const ideAfterKeys = await ideCanvas.screenshot();
    if (ideBeforeKeys.equals(ideAfterKeys)) {
      throw new Error("IDE debug camera did not receive real W keyboard movement");
    }

    // A pointer-lock promise belongs to the pane that was clicked. Switching
    // focus before it resolves must still retire that pane's generation ack.
    await page.goto(`${BASE}/sandbox.html?example=orbit&clients=2`);
    await page.waitForFunction(() => {
      const iframes = [...document.querySelectorAll(".mp-pane iframe")];
      return (
        iframes.length === 2 &&
        iframes.every((iframe) => iframe.contentWindow?.__scrub?.range().length === 2)
      );
    });
    await page.evaluate(() => {
      const iframe = document.querySelectorAll(".mp-pane iframe")[0];
      const canvas = iframe.contentDocument?.querySelector("#canvas");
      Object.defineProperty(canvas, "requestPointerLock", {
        configurable: true,
        value: () =>
          new Promise((resolve) => {
            window.__resolveDebugPointerLock = resolve;
          }),
      });
    });
    await page.locator("#mp-camera").click();
    await page.evaluate(() =>
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "2" }))
    );
    await page.waitForFunction(() => {
      const first = document.querySelectorAll(".mp-pane iframe")[0];
      return !first.contentWindow?.__scrub?.detached() &&
        !document.querySelector("#mp-camera")?.disabled;
    });
    await page.evaluate(() => window.__resolveDebugPointerLock());
    await page.waitForTimeout(50);
    if (
      await page.evaluate(
        () => document.querySelectorAll(".mp-pane iframe")[0]
          ?.contentWindow?.__scrub?.detached()
      )
    ) {
      throw new Error("stale pointer-lock acceptance activated an unfocused pane");
    }

    // Once pointer lock has been accepted and the toggle queued, a later focus
    // change still reconciles pane 1's acknowledgement and releases capture.
    await page.evaluate(() => window.dispatchEvent(new KeyboardEvent("keydown", { key: "1" })));
    await page.evaluate(() => {
      const iframe = document.querySelectorAll(".mp-pane iframe")[0];
      const canvas = iframe.contentDocument?.querySelector("#canvas");
      Object.defineProperty(canvas, "requestPointerLock", {
        configurable: true,
        value: () => ({
          then: (accepted) => {
            accepted();
            window.dispatchEvent(new KeyboardEvent("keydown", { key: "2" }));
          },
        }),
      });
    });
    await page.locator("#mp-camera").click();
    await page.waitForFunction(() => {
      const first = document.querySelectorAll(".mp-pane iframe")[0];
      return first.contentWindow?.__scrub?.detached() &&
        !document.querySelector("#mp-camera")?.disabled;
    });

    // If the pane reloads after acceptance but before its WASM frame applies
    // the toggle, the host must abandon the old, unacknowledgeable seam.
    await page.evaluate(() => window.dispatchEvent(new KeyboardEvent("keydown", { key: "1" })));
    await page.locator("#mp-camera").click();
    await page.waitForFunction(() => {
      const first = document.querySelectorAll(".mp-pane iframe")[0];
      return !first.contentWindow?.__scrub?.detached();
    });
    await page.evaluate(() => {
      const iframe = document.querySelectorAll(".mp-pane iframe")[0];
      window.__debugReloadDocument = iframe.contentDocument;
      const canvas = iframe.contentDocument?.querySelector("#canvas");
      Object.defineProperty(canvas, "requestPointerLock", {
        configurable: true,
        value: () => ({
          then: (accepted) => {
            accepted();
            iframe.src = iframe.src;
          },
        }),
      });
    });
    await page.locator("#mp-camera").click();
    await page.waitForFunction(() => {
      const first = document.querySelectorAll(".mp-pane iframe")[0];
      return (
        first.contentDocument !== window.__debugReloadDocument &&
        first.contentWindow?.__scrub?.range().length === 2
      );
    });
    await page.waitForFunction(() => !document.querySelector("#mp-camera")?.disabled);

    if (pageErrors.length) throw new Error(`page errors: ${pageErrors.join("; ")}`);
    console.log(
      "PASS: universal debug cameras navigate live/paused 3D and paused 2D without mutating authored frames"
    );
  } finally {
    await browser.close();
  }
} finally {
  server.kill("SIGTERM");
}
