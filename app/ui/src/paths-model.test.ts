import { test } from "node:test";
import assert from "node:assert/strict";
import type { Evidence, Moment, Task } from "./generated/protocol.ts";
import { momentPhrase, pathsNote } from "./paths-model.ts";

const task = (moment: Moment | null, state: Evidence["state"] | null) =>
  ({ moment, evidence: state && { tree: "t", state } }) as Task;
const taken: Moment = { kind: "taken", frame: 212, steps: 340 };

test("the moment names its frame and the inputs both paths replay", () => {
  assert.equal(momentPhrase(task(taken, null)), "frame 212, replaying the same 340 recorded input steps");
  assert.equal(momentPhrase(task({ kind: "taken", frame: 0, steps: 0 }, null)), "frame 0, with no recorded input");
  assert.equal(momentPhrase(task({ kind: "taken", frame: 1, steps: 1 }, null)), "frame 1, replaying the same 1 recorded input step");
  assert.equal(momentPhrase(task({ kind: "missing", reason: "x" }, null)), null);
  assert.equal(momentPhrase(task(null, null)), null);
});

test("the page says why there is no pair, and nothing when there is one or no code changed", () => {
  assert.equal(pathsNote(task(taken, null)), null);
  assert.equal(pathsNote(task(taken, { kind: "shown", before: "a", after: "b" })), null);
  assert.equal(pathsNote(task(taken, { kind: "taking" })), "Predicting the path before and after the change…");
  assert.equal(
    pathsNote(task(taken, { kind: "same" })),
    "The change moves nothing on the predicted path from frame 212, replaying the same 340 recorded input steps.",
  );
  assert.equal(
    pathsNote(task(taken, { kind: "withheld", reason: "the old code did not load" })),
    "No predicted path: the old code did not load.",
  );
});
