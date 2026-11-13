import { test } from "node:test";
import assert from "node:assert/strict";
import type { AppState, Report, Review, Task } from "./generated/protocol.ts";
import { DEFAULT_REVIEW, modelHint, reviewLines, reviewSetting } from "./review-model.ts";

const review = (state: Review["state"], usage: Review["usage"] = null): Review => ({
  run: 0,
  tree: "t",
  harness: "claude",
  model: "sonnet",
  state,
  usage,
});
const report = (reviewed: boolean): Report => ({ summary: "The cube spins.", notes: [], evidence: [], reviewed });
const task = (r: Review | null, rep: Report | null) => ({ review: r, report: rep }) as Task;

test("the first page says who prepared the report, or that it is being prepared, or why not", () => {
  assert.deepEqual(reviewLines(task(null, report(false))), []);
  assert.deepEqual(reviewLines(task(review({ kind: "preparing" }), report(false))), [
    { kind: "preparing", text: "Preparing the summary…" },
  ]);
  assert.deepEqual(reviewLines(task(review({ kind: "prepared" }, { input: 41_234, output: 900 }), report(true))), [
    { kind: "by", text: "Prepared by a separate review (claude · sonnet · 41k tokens in)" },
  ]);
  // A review that failed leaves the agent's report, and says why.
  assert.deepEqual(reviewLines(task(review({ kind: "failed", reason: "cut: over 180 s" }), report(false))), [
    { kind: "failed", text: "The separate review did not finish: cut: over 180 s." },
  ]);
});

test("the setting is the active project's, else the default", () => {
  const state = { active: "a", review_settings: { a: { on: false, harness: null, model: null, instructions: null } } } as unknown as AppState;
  assert.equal(reviewSetting(state).on, false);
  assert.deepEqual(reviewSetting({ ...state, active: "b" }), DEFAULT_REVIEW);
  assert.deepEqual(reviewSetting(null), DEFAULT_REVIEW);
  assert.equal(modelHint(DEFAULT_REVIEW, "claude"), "sonnet (default for claude)");
  assert.equal(modelHint({ ...DEFAULT_REVIEW, harness: "codex" }, "claude"), "codex's default");
});
