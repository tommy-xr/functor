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
    .expectModelClose("camera.yaw", -0.6, 0.001)
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
      {
        type: "expect_model_close",
        path: "camera.yaw",
        expected: -0.6,
        abs_tolerance: 0.001,
      },
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
    state: async () => state({ stats: { kills: 8 }, camera: { yaw: -0.60001 } }),
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
      .expectModelClose("camera.yaw", -0.6, 0.001)
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
  assert.deepEqual(result.assertions[1], {
    path: "camera.yaw",
    expected: -0.6,
    actual: -0.60001,
    absTolerance: 0.001,
    passed: true,
  });
  assert.equal(result.observations[0].label, "settled");
  assert.equal(result.captures[0].png, png);
  assert.deepEqual(result.finalState.model, {
    stats: { kills: 8 },
    camera: { yaw: -0.60001 },
  });
});

test("standalone array paths reject non-canonical indices like MCP", async () => {
  const client = {
    state: async () => state({ players: [{ score: 3 }] }),
  } as AutomationClient;

  await assert.rejects(
    runAutomation(client, automation().expectModel("players.00.score", 3)),
    /does not exist/,
  );
  await assert.rejects(
    runAutomation(client, automation().expectModel("players.+0.score", 3)),
    /does not exist/,
  );
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

test("pressKey attempts release when keyDown itself reports failure", async () => {
  const calls: string[] = [];
  const client = {
    keyDown: async (key: string) => {
      calls.push(`down:${key}`);
      throw new Error("down timed out");
    },
    keyUp: async (key: string) => {
      calls.push(`up:${key}`);
    },
    stepFrames: async () => {
      calls.push("unexpected step");
    },
  } as unknown as AutomationClient;

  await assert.rejects(
    runAutomation(client, automation().pressKey("space")),
    /down timed out/,
  );
  assert.deepEqual(calls, ["down:space", "up:space"]);
});

test("standalone plan validation matches MCP's literal and wire bounds", async () => {
  const neverUsed = {} as AutomationClient;
  await assert.rejects(
    runAutomation(neverUsed, automation().mouseMove(1.5, 0)),
    /signed 32-bit integer/,
  );
  await assert.rejects(
    runAutomation(neverUsed, automation().mouseMove(0x8000_0000, 0)),
    /signed 32-bit integer/,
  );
  await assert.rejects(
    runAutomation(neverUsed, automation().mouseWheel(-0x8000_0001)),
    /signed 32-bit integer/,
  );
  await assert.rejects(
    runAutomation(neverUsed, automation().uiClick(0x1_0000_0000)),
    /unsigned 32-bit/,
  );
  await assert.rejects(
    runAutomation(neverUsed, automation().uiClick(0.5)),
    /unsigned 32-bit/,
  );
  await assert.rejects(
    runAutomation(neverUsed, automation().keyDown("Shift")),
    /unknown key "Shift"/,
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
      automation().expectModelClose("x", 1, Number.NaN),
    ),
    /absolute tolerance must be finite/,
  );
  await assert.rejects(
    runAutomation(neverUsed, automation().expectModelClose("x", 1, -0.1)),
    /absolute tolerance must be non-negative/,
  );
  await assert.rejects(
    runAutomation(
      neverUsed,
      automation().expectModel("x", "x".repeat(17 * 1024)),
    ),
    /canonical automation source.*maximum/,
  );
});
