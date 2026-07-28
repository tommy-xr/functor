import assert from "node:assert/strict";
import { test } from "node:test";

import {
  automation,
  runAutomation,
  type AutomationClient,
  type RuntimeState,
} from "../src/index.js";

function state(model: RuntimeState["model"]): RuntimeState {
  return {
    frame: 3,
    tts: 0.032,
    pending_steps: 0,
    viewport: { width: 800, height: 600 },
    views: [],
    input: { held_keys: [], mouse: { x: 0, y: 0 } },
    model,
    model_debug: "",
  };
}

test("the fluent vocabulary produces the same serializable plan as MCP", () => {
  const plan = automation("jam proof")
    .pause()
    .pressKey("3")
    .mouseMove(600, 200)
    .step({ frames: 2, dts: 0.02 })
    .expectModel("stats.kills", 8)
    .inspect("settled")
    .capture("proof")
    .toPlan();

  assert.deepEqual(plan, {
    version: 1,
    name: "jam proof",
    steps: [
      { type: "pause" },
      { type: "press_key", key: "3" },
      { type: "mouse_move", x: 600, y: 200 },
      { type: "step", frames: 2, dts: 0.02 },
      { type: "expect_model", path: "stats.kills", equals: 8 },
      { type: "inspect", label: "settled" },
      { type: "capture", label: "proof" },
    ],
  });
  assert.match(
    automation("jam proof").pause().pressKey("3").toCode(),
    /^automation\("jam proof"\)\n  \.pause\(\)\n  \.pressKey\("3"\);\n$/,
  );
});

test("standalone execution drives typed operations, asserts, inspects, and captures", async () => {
  const calls: string[] = [];
  const png = Buffer.from([0x89, 0x50, 0x4e, 0x47]);
  const client = {
    pause: async () => {
      calls.push("pause");
    },
    keyDown: async (key: string) => {
      calls.push(`down:${key}`);
    },
    keyUp: async (key: string) => {
      calls.push(`up:${key}`);
    },
    mouseMove: async (x: number, y: number) => {
      calls.push(`move:${x},${y}`);
    },
    mouseDown: async () => {},
    mouseUp: async () => {},
    mouseWheel: async () => {},
    input: async () => {},
    stepFrames: async (frames: number, dts: number) => {
      calls.push(`step:${frames}@${dts}`);
    },
    state: async () => state({ stats: { kills: 8 } }),
    capture: async () => png,
  } as AutomationClient;

  const result = await runAutomation(
    client,
    automation("standalone")
      .pause()
      .pressKey("3")
      .mouseMove(600, 200)
      .step({ frames: 2, dts: 0.02 })
      .expectModel("stats.kills", 8)
      .inspect("settled")
      .capture("proof"),
  );

  assert.deepEqual(calls, [
    "pause",
    "down:3",
    "step:1@0.016",
    "up:3",
    "move:600,200",
    "step:2@0.02",
  ]);
  assert.equal(result.assertions[0].passed, true);
  assert.equal(result.observations[0].label, "settled");
  assert.equal(result.captures[0].png, png);
  assert.deepEqual(result.finalState.model, { stats: { kills: 8 } });
});

test("pressKey releases the key when its deterministic step fails", async () => {
  const calls: string[] = [];
  const client = {
    keyDown: async (key: string) => {
      calls.push(`down:${key}`);
    },
    keyUp: async (key: string) => {
      calls.push(`up:${key}`);
    },
    stepFrames: async () => {
      throw new Error("step failed");
    },
  } as unknown as AutomationClient;

  await assert.rejects(
    runAutomation(client, automation().pressKey("space")),
    /step failed/,
  );
  assert.deepEqual(calls, ["down:space", "up:space"]);
});

test("standalone plan validation matches MCP's literal and wire bounds", async () => {
  const neverUsed = {} as AutomationClient;
  await assert.rejects(
    runAutomation(neverUsed, automation().mouseMove(1_000_001, 0)),
    /±1,000,000/,
  );
  await assert.rejects(
    runAutomation(neverUsed, automation().uiClick(0x1_0000_0000)),
    /unsigned 32-bit/,
  );
  await assert.rejects(
    runAutomation(neverUsed, automation().expectModel("a..b", true)),
    /dotted model paths/,
  );
  await assert.rejects(
    runAutomation(neverUsed, automation().expectModel("x", Number.NaN)),
    /finite numbers/,
  );
  await assert.rejects(
    runAutomation(
      neverUsed,
      automation().expectModel("x", "x".repeat(17 * 1024)),
    ),
    /canonical automation source.*maximum/,
  );
});
