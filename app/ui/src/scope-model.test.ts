import { test } from "node:test";
import assert from "node:assert/strict";
import type { AppState, Task } from "./generated/protocol";
import { projectScope, taskScope, scopesFor } from "./scope-model.ts";

const state = (scope_choice: AppState["scope_choice"]) => ({ active: "a", scope_choice }) as unknown as AppState;
const task = (scope: Task["scope"]) => ({ scope }) as Task;

test("the project's level, all of the user's setup when none is saved", () => {
  assert.equal(projectScope(null), "user_and_project");
  assert.equal(projectScope(state({})), "user_and_project");
  assert.equal(projectScope(state({ a: "isolated" })), "isolated");
});

test("a new task reads its pick, else the project's; a task keeps its own", () => {
  const s = state({ a: "project" });
  assert.equal(taskScope(null, null, s, "claude"), "project");
  assert.equal(taskScope("isolated", null, s, "claude"), "isolated");
  assert.equal(taskScope("user_and_project", task("isolated"), s, "claude"), "isolated");
});

test("Codex is offered, and runs, only all of it", () => {
  assert.deepEqual(scopesFor("codex"), ["user_and_project"]);
  assert.equal(taskScope("isolated", null, state({ a: "project" }), "codex"), "user_and_project");
});
