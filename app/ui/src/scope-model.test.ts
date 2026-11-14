import { test } from "node:test";
import assert from "node:assert/strict";
import type { AppState, Task } from "./generated/protocol";
import { projectScope, requestScope, scopesFor } from "./scope-model.ts";

const state = (scope_choice: AppState["scope_choice"]) => ({ active: "a", scope_choice }) as unknown as AppState;
const task = (scope: Task["scope"]) => ({ scope }) as Task;

test("the project's level, all of the user's setup when none is saved", () => {
  assert.equal(projectScope(null), "user_and_project");
  assert.equal(projectScope(state({})), "user_and_project");
  assert.equal(projectScope(state({ a: "isolated" })), "isolated");
});

test("a request reads its pick, else its task's level, else the project's", () => {
  const s = state({ a: "project" });
  assert.equal(requestScope(null, null, s, "claude"), "project");
  assert.equal(requestScope(null, task("isolated"), s, "claude"), "isolated");
  assert.equal(requestScope("user_and_project", task("isolated"), s, "claude"), "user_and_project");
});

test("Codex is offered, and runs, only all of it", () => {
  assert.deepEqual(scopesFor("codex"), ["user_and_project"]);
  assert.equal(requestScope("isolated", null, state({ a: "project" }), "codex"), "user_and_project");
});
