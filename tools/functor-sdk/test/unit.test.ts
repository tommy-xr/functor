import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";

import {
  findRepoRoot,
  formatCrashOutput,
  FunctorClient,
  HttpClient,
  stepAll,
  waitFor,
} from "../src/index.js";

// Pure unit tests — no runtime required, always run.

test("findRepoRoot walks up to the cargo workspace root", () => {
  const root = findRepoRoot(process.cwd());
  assert.ok(root !== undefined, "should find a workspace root from the SDK dir");
  assert.ok(
    existsSync(join(root, "Cargo.toml")),
    "found root should contain Cargo.toml",
  );
});

test("findRepoRoot returns undefined when there's no workspace above", () => {
  assert.equal(findRepoRoot("/"), undefined);
});

test("formatCrashOutput keeps the panic and its context", () => {
  const lines = ["init", "loading", "ok", "panicked at foo.rs:1", "stack:", "  0: x"];
  const out = formatCrashOutput(lines);
  assert.match(out, /panicked at foo\.rs:1/);
  assert.match(out, /0: x/, "should include lines after the panic");
  assert.match(out, /ok/, "should include a little context before the panic");
});

test("formatCrashOutput falls back to the tail when there's no panic", () => {
  const lines = Array.from({ length: 50 }, (_, i) => `line ${i}`);
  const out = formatCrashOutput(lines);
  assert.match(out, /line 49/, "should include the last line");
  assert.doesNotMatch(out, /line 0\b/, "should drop the earliest lines");
});

test("stepAll advances every client by the same dt, concurrently", async () => {
  const calls: number[] = [];
  const fake = () => ({
    step: async (dt: number) => {
      calls.push(dt);
    },
  });
  const clients = [fake(), fake(), fake()];

  await stepAll(clients, 0.25);

  assert.deepEqual(calls, [0.25, 0.25, 0.25]);
});

test("isKeyDown maps bare digit input to serialized Num key names", async () => {
  const client = new FunctorClient({
    getJson: async () => ({ input: { held_keys: ["Num1"] } }),
  } as unknown as HttpClient);

  assert.equal(await client.isKeyDown("1"), true);
  assert.equal(await client.isKeyDown("2"), false);
});

test("waitFor returns once the predicate holds", async () => {
  let n = 0;
  const value = await waitFor(
    async () => ++n,
    (v) => v >= 3,
    { intervalMs: 1 },
  );
  assert.equal(value, 3);
});

test("waitFor retries when poll throws, then resolves", async () => {
  let n = 0;
  const value = await waitFor(
    async () => {
      n++;
      if (n < 3) throw new Error("transient");
      return n;
    },
    (v) => v >= 3,
    { intervalMs: 1 },
  );
  assert.equal(value, 3);
});

test("waitFor surfaces the last poll error on timeout", async () => {
  await assert.rejects(
    waitFor(
      async () => {
        throw new Error("boom");
      },
      () => true,
      { timeoutMs: 20, intervalMs: 5, description: "x" },
    ),
    /timed out after 20ms waiting for x \(last error: Error: boom\)/,
  );
});

test("waitFor throws on timeout with the description", async () => {
  await assert.rejects(
    waitFor(
      async () => false,
      (v) => v === true,
      { timeoutMs: 20, intervalMs: 5, description: "the impossible" },
    ),
    /timed out after 20ms waiting for the impossible/,
  );
});

test("stepUntil checks immediately, then advances deterministically", async () => {
  let value = 0;
  let pendingSteps = 0;
  let pollsBeforeLanding = 0;
  const calls: Array<{ path: string; body?: unknown }> = [];
  const state = () => ({
    frame: value,
    tts: value / 60,
    pending_steps: pendingSteps,
    viewport: { width: 1, height: 1 },
    views: [],
    model: { value },
    model_debug: String(value),
    input: { held_keys: [], mouse: { x: 0, y: 0 } },
  });
  const http = {
    getJson: async (path: string) => {
      calls.push({ path });
      if (pendingSteps > 0) {
        if (pollsBeforeLanding === 0) {
          pendingSteps = 0;
          value++;
        } else {
          pollsBeforeLanding--;
        }
      }
      return state();
    },
    postText: async (path: string, body: unknown) => {
      calls.push({ path, body });
      if (path === "/time") {
        const advance = body as { frames?: number };
        pendingSteps = advance.frames ?? 1;
        pollsBeforeLanding = 1;
      }
      return "ok";
    },
  } as unknown as HttpClient;
  const client = new FunctorClient(http);

  const settled = await client.stepUntil(
    (current) => (current.model as { value: number }).value >= 2,
    { maxFrames: 3, dts: 0.25, description: "value >= 2" },
  );

  assert.deepEqual(settled.model, { value: 2 });
  assert.deepEqual(
    calls.filter(({ path }) => path === "/time").map(({ body }) => body),
    [
      { type: "advance", dts: 0.25, frames: 1 },
      { type: "advance", dts: 0.25, frames: 1 },
    ],
  );
});

test("stepUntil is bounded and explains exhaustion", async () => {
  const http = {
    getJson: async () => ({
      frame: 0,
      tts: 0,
      pending_steps: 0,
      viewport: { width: 1, height: 1 },
      views: [],
      model: { ready: false },
      model_debug: "",
      input: { held_keys: [], mouse: { x: 0, y: 0 } },
    }),
    postText: async () => "ok",
  } as unknown as HttpClient;
  const client = new FunctorClient(http);

  await assert.rejects(
    client.stepUntil(() => false, {
      maxFrames: 2,
      description: "the game to become ready",
    }),
    /exhausted 2 frames waiting for the game to become ready/,
  );
  await assert.rejects(
    client.stepUntil(() => true, { maxFrames: 10_001 }),
    /between 0 and 10000/,
  );
});

test("pressKey always releases the key after a failed step", async () => {
  const calls: unknown[] = [];
  const http = {
    postText: async (path: string, body: unknown) => {
      calls.push({ path, body });
      if (path === "/time") throw new Error("step failed");
      return "ok";
    },
  } as HttpClient;
  const client = new FunctorClient(http);

  await assert.rejects(client.pressKey("space"), /step failed/);
  assert.deepEqual(calls, [
    { path: "/input", body: { type: "key", key: "space", down: true } },
    { path: "/time", body: { type: "advance", dts: 1 / 60, frames: 1 } },
    { path: "/input", body: { type: "key", key: "space", down: false } },
  ]);
});

test("pressKey waits for its queued step before releasing the key", async () => {
  let pendingSteps = 0;
  let stateReads = 0;
  let landed = false;
  const http = {
    postText: async (path: string, body: unknown) => {
      if (path === "/time") {
        pendingSteps = 1;
      }
      if (
        path === "/input" &&
        (body as { down?: boolean }).down === false
      ) {
        assert.equal(landed, true, "release must follow the landed frame");
      }
      return "ok";
    },
    getJson: async () => {
      stateReads++;
      if (stateReads >= 2) {
        pendingSteps = 0;
        landed = true;
      }
      return {
        frame: landed ? 1 : 0,
        tts: 0,
        pending_steps: pendingSteps,
        viewport: { width: 1, height: 1 },
        views: [],
        model: {},
        model_debug: "",
        input: { held_keys: [], mouse: { x: 0, y: 0 } },
      };
    },
  } as unknown as HttpClient;

  await new FunctorClient(http).pressKey("space");
  assert.equal(stateReads, 2);
});

test("xrInput returns the typed optional input domain", async () => {
  const xr = {
    head: {
      position: [0, 0, 0] as [number, number, number],
      orientation: [0, 0, 0, 1] as [number, number, number, number],
    },
    left: {
      active: true,
      grip: null,
      aim: null,
      trigger: 0.5,
      squeeze: 0,
      thumbstick: [0, 1] as [number, number],
      primary_pressed: true,
      secondary_pressed: false,
      thumbstick_pressed: false,
      menu_pressed: false,
    },
    right: {
      active: false,
      grip: null,
      aim: null,
      trigger: 0,
      squeeze: 0,
      thumbstick: [0, 0] as [number, number],
      primary_pressed: false,
      secondary_pressed: false,
      thumbstick_pressed: false,
      menu_pressed: false,
    },
  };
  const http = {
    getJson: async () => ({
      frame: 1,
      tts: 0,
      viewport: { width: 1, height: 1 },
      views: [],
      model: "{}",
      input: { held_keys: [], mouse: { x: 0, y: 0 }, xr },
    }),
  } as unknown as HttpClient;
  const client = new FunctorClient(http);

  assert.deepEqual(await client.xrInput(), xr);
});

test("xr() posts a flat, tagged sample and passes partials through", async () => {
  const calls: Array<{ path: string; body: unknown }> = [];
  const http = {
    postText: async (path: string, body: unknown) => {
      calls.push({ path, body });
      return "ok";
    },
  } as HttpClient;
  const client = new FunctorClient(http);

  // The wire shape is the sample INLINED beside the tag, not nested — and the
  // runtime defaults everything omitted, so a partial hand is legal.
  await client.xr({
    right: { active: true, grip: { position: [0, 0, 0.12] }, trigger: 1 },
  });

  assert.deepEqual(calls, [
    {
      path: "/input",
      body: {
        type: "xr",
        right: { active: true, grip: { position: [0, 0, 0.12] }, trigger: 1 },
      },
    },
  ]);
});

test("gamepadInput returns the typed optional input domain", async () => {
  const gamepad = {
    left_stick: [-0.5, 1] as [number, number],
    right_stick: [0, 0] as [number, number],
    left_trigger: 0,
    right_trigger: 0.25,
    south: true,
    east: false,
    west: false,
    north: false,
    left_bumper: false,
    right_bumper: false,
    left_stick_pressed: false,
    right_stick_pressed: false,
    dpad_up: false,
    dpad_down: false,
    dpad_left: true,
    dpad_right: false,
    start: false,
    select: false,
  };
  const http = {
    getJson: async () => ({
      frame: 1,
      tts: 0,
      viewport: { width: 1, height: 1 },
      views: [],
      model: "{}",
      input: { held_keys: [], mouse: { x: 0, y: 0 }, gamepad },
    }),
  } as unknown as HttpClient;
  const client = new FunctorClient(http);

  assert.deepEqual(await client.gamepadInput(), gamepad);
});

test("gamepad() posts a flat, tagged partial sample; gamepadClear() releases it", async () => {
  const calls: Array<{ path: string; body: unknown }> = [];
  const http = {
    postText: async (path: string, body: unknown) => {
      calls.push({ path, body });
      return "ok";
    },
  } as HttpClient;
  const client = new FunctorClient(http);

  await client.gamepad({ left_stick: [-0.5, 1], south: true });
  await client.gamepadClear();

  assert.deepEqual(calls, [
    {
      path: "/input",
      body: { type: "gamepad", left_stick: [-0.5, 1], south: true },
    },
    { path: "/input", body: { type: "gamepad_clear" } },
  ]);
});

test("reloadAssets uploads binary envelopes then finalizes the manifest", async () => {
  const calls: Array<{ path: string; body: unknown }> = [];
  const http = {
    postRawBinary: async (path: string, body: Uint8Array) => {
      calls.push({ path, body });
      return "reloaded";
    },
    postText: async (path: string, body: unknown) => {
      calls.push({ path, body });
      return "synced";
    },
  } as HttpClient;
  const client = new FunctorClient(http);

  assert.equal(
    await client.reloadAssets([["textures/grid.png", Uint8Array.of(0, 1, 255)]]),
    "synced",
  );
  assert.equal(calls[0].path, "/reload-asset");
  const envelope = calls[0].body as Uint8Array;
  const pathLength = new DataView(
    envelope.buffer,
    envelope.byteOffset,
    envelope.byteLength,
  ).getUint32(0, false);
  assert.equal(
    new TextDecoder().decode(envelope.slice(4, 4 + pathLength)),
    "textures/grid.png",
  );
  assert.deepEqual([...envelope.slice(4 + pathLength)], [0, 1, 255]);
  assert.deepEqual(calls[1], {
    path: "/sync-assets",
    body: ["textures/grid.png"],
  });
});

test("project load and reload use distinct lifecycle routes", async () => {
  const calls: Array<{ path: string; body: unknown }> = [];
  const http = {
    postText: async (path: string, body: unknown) => {
      calls.push({ path, body });
      return "ok";
    },
  } as HttpClient;
  const client = new FunctorClient(http);
  const files: [string, string][] = [["game.fun", "let init = 0"]];

  await client.loadProject(files);
  await client.reloadProject(files);

  assert.deepEqual(calls, [
    { path: "/load-project", body: files },
    { path: "/reload-project", body: files },
  ]);
});
