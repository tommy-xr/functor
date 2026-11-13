import { test } from "node:test";
import assert from "node:assert/strict";
import type { Evidence, Report, Task } from "./generated/protocol.ts";
import { evidenceLines, firstOf, sizeLine } from "./decision-model.ts";

test("the size line names the files and their kinds", () => {
  assert.equal(sizeLine({ added: 2, removed: 2, files: 1, kinds: ["game_logic"] }), "1 file · game logic");
  assert.equal(sizeLine({ added: 9, removed: 0, files: 3, kinds: ["game_logic", "assets", "other"] }), "3 files · game logic, assets, other files");
  assert.equal(sizeLine({ added: 0, removed: 0, files: 201, kinds: [] }), "201 files");
});

const task = (report: Report | null, state: Evidence["state"] | null) =>
  ({ report, moment: { kind: "taken", frame: 5, steps: 0 }, evidence: state && { tree: "t", state } }) as Task;

test("evidence is the agent's checks, then the predicted path", () => {
  const report: Report = {
    summary: null,
    notes: [],
    evidence: [
      { ran: "functor test", result: "4 expects pass", ok: true },
      { ran: "functor build", result: "1 error", ok: false },
    ],
  };
  assert.deepEqual(evidenceLines(task(report, { kind: "shown", before: "a", after: "b" })), [
    { code: "functor test", text: "4 expects pass", ok: true },
    { code: "functor build", text: "1 error", ok: false },
    { text: "Predicted path before and after", ok: true, tab: "captures" },
  ]);
  // The same line while it is taken, without its tick.
  assert.deepEqual(evidenceLines(task(null, { kind: "taking" })), [{ text: "Predicted path before and after", ok: null, tab: "captures" }]);
  assert.deepEqual(evidenceLines(task(null, { kind: "withheld", reason: "the old code did not load" })), [
    { text: "No predicted path: the old code did not load.", ok: null },
  ]);
  assert.deepEqual(evidenceLines(task(null, { kind: "same" })), [
    { text: "The change moves nothing on the predicted path from frame 5, with no recorded input.", ok: null },
  ]);
  assert.deepEqual(evidenceLines(task(null, null)), []);
});

test("lists show their first lines and count the rest", () => {
  assert.deepEqual(firstOf([1, 2, 3, 4, 5], 3), { shown: [1, 2, 3], more: 2 });
  assert.deepEqual(firstOf([1], 3), { shown: [1], more: 0 });
});
