import { test } from "node:test";
import assert from "node:assert/strict";
import type { AppState, Change, Item, Task, TaskStatus } from "./generated/protocol";
import {
  changeSummary,
  choiceKey,
  chosenTask,
  currentTask,
  deleteBlock,
  groupTasks,
  needsYou,
  relativeTime,
  secondaryLine,
  taskGroup,
} from "./tasks-model.ts";
import { taskPage, usageTable } from "./task-page-model.ts";

const change: Change = {
  files: [
    { path: "a/game.fun", from: null, added: 2, removed: 1, binary: false, large: false },
    { path: "a/other.fun", from: null, added: 1, removed: 0, binary: false, large: false },
  ],
  more: 0,
  tree: "t",
};

const task = (id: number, status: TaskStatus, over: Partial<Task> = {}): Task => ({
  id,
  project: "a",
  title: `task ${id}`,
  status,
  harness: "claude",
  model: null,
  session_id: null,
  run: null,
  baseline: null,
  change: null,
  reverting: false,
  created_at: null,
  updated_at: id * 1000,
  ...over,
});

test("every status has its group: failed waits with the stopped, an answer is done", () => {
  const all: [TaskStatus, string][] = [
    [{ kind: "waiting_on_you" }, "waiting"],
    [{ kind: "failed", summary: "x" }, "waiting"],
    [{ kind: "failed", summary: null }, "waiting"],
    [{ kind: "ready_for_review" }, "review"],
    [{ kind: "in_progress" }, "progress"],
    [{ kind: "done" }, "done"],
    [{ kind: "kept" }, "done"],
    [{ kind: "reverted" }, "done"],
  ];
  for (const [status, group] of all) assert.equal(taskGroup(status), group, status.kind);
});

test("groups come in order, newest first, empty ones left out, filtered by title", () => {
  const tasks = [
    task(1, { kind: "kept" }),
    task(2, { kind: "in_progress" }),
    task(3, { kind: "failed", summary: null }, { title: "Fix the ladder" }),
    task(4, { kind: "waiting_on_you" }),
    task(5, { kind: "ready_for_review" }, { title: "Spin the LADDER" }),
    task(6, { kind: "done" }, { updated_at: null }),
    task(7, { kind: "waiting_on_you" }, { project: "b" }),
  ];
  const shape = (q?: string) => groupTasks(tasks, "a", q).map((g) => [g.label, g.tasks.map((t) => t.id)]);
  assert.deepEqual(shape(), [
    ["Waiting on you", [4, 3]],
    ["Ready for review", [5]],
    ["In progress", [2]],
    ["Done", [1, 6]],
  ]);
  assert.deepEqual(shape(" ladder "), [
    ["Waiting on you", [3]],
    ["Ready for review", [5]],
  ]);
  assert.deepEqual(shape("nothing like it"), []);
  // An untitled task matches what it shows as.
  assert.deepEqual(groupTasks([task(8, { kind: "done" }, { title: "" })], "a", "untitled")[0]!.tasks.length, 1);
  // Waiting (failed included) and ready for review need the user.
  assert.equal(needsYou(tasks, "a"), 3);
  assert.equal(needsYou(tasks, "b"), 1);
});

test("a row's second line: the change to review, why it failed, the agent's last line", () => {
  const items: Item[] = [
    { kind: "user", text: "hi" },
    { kind: "agent", text: "\nChecked the ladder.\nMore.", partial: false },
    { kind: "tool", name: "Read", input: {}, result: "x" },
  ];
  assert.equal(changeSummary(change), "+3 −1 in 2 files");
  const ready = task(1, { kind: "ready_for_review" }, { baseline: "b", change });
  assert.equal(secondaryLine(ready, items), "+3 −1 in 2 files");
  const failed = task(2, { kind: "failed", summary: "No game.\nStart one." });
  assert.equal(secondaryLine(failed, items), "No game.");
  assert.equal(secondaryLine(task(3, { kind: "done" }), items), "Checked the ladder.");
  assert.equal(secondaryLine(task(3, { kind: "done" }), undefined), "");
});

test("delete is blocked while running or with a change to review", () => {
  const running = task(1, { kind: "in_progress" }, { run: { id: 0, usage: null, views: 0 } });
  assert.equal(deleteBlock(running, true), "It is running: stop it first.");
  const ready = task(2, { kind: "ready_for_review" }, { baseline: "b", change });
  assert.equal(deleteBlock(ready, true), "It has a change to review: keep or revert it first.");
  // Outside git nothing can be reviewed, so nothing holds it.
  assert.equal(deleteBlock(ready, false), null);
  assert.equal(deleteBlock(task(3, { kind: "kept" }), true), null);
});

test("relative times against a fixed clock", () => {
  const now = Date.UTC(2026, 10, 12, 12, 0, 0);
  const ago = (ms: number) => relativeTime(now - ms, now);
  assert.equal(relativeTime(null, now), "");
  assert.equal(ago(0), "just now");
  assert.equal(ago(-5000), "just now", "a clock skewed ahead");
  assert.equal(ago(59_000), "just now");
  assert.equal(ago(60_000), "1 min ago");
  assert.equal(ago(59 * 60_000), "59 min ago");
  assert.equal(ago(3 * 3600_000), "3 h ago");
  assert.equal(ago(30 * 3600_000), "yesterday");
  assert.equal(ago(4 * 86400_000), "4 days ago");
  const old = now - 30 * 86400_000;
  const d = new Date(old);
  const month = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"][d.getMonth()];
  assert.equal(relativeTime(old, now), `${month} ${d.getDate()}`);
});

test("the current task is the core's, per active project", () => {
  const state = (current: AppState["current_task"], active: string | null = "a") => ({ active, current_task: current }) as AppState;
  assert.equal(chosenTask(state({ a: 3 })), 3);
  assert.equal(chosenTask(state({ a: null })), null);
  assert.equal(chosenTask(state({ b: 3 })), undefined);
  assert.equal(chosenTask(state({ a: 3 }, null)), undefined);
  assert.equal(chosenTask(null), undefined);
});

test("until the core has one, the page's last choice, else the newest open task", () => {
  const tasks = [task(1, { kind: "done" }), task(2, { kind: "in_progress" }), task(3, { kind: "kept" })];
  const state = (current: AppState["current_task"]) => ({ repo_root: "/r", active: "a", tasks, current_task: current }) as AppState;
  const none = new Map<string, number | null>();
  assert.equal(currentTask(state({ a: 3 }), none), 3);
  assert.equal(currentTask(state({ a: null }), none), null);
  assert.equal(currentTask(state({}), none), 2);
  // A restarted app forgot: what this page last saw, while that task is there.
  const seen = (v: number | null) => new Map([[choiceKey(state({})), v]]);
  assert.equal(currentTask(state({}), seen(1)), 1);
  assert.equal(currentTask(state({}), seen(null)), null);
  assert.equal(currentTask(state({}), seen(9)), 2);
  assert.equal(currentTask(null, none), null);
});

const tool = (name: string, result: string | null, input: object = {}): Item => ({ kind: "tool", name, input: input as never, result });

test("the page: the first message in full, follow-ups, steps per run with failures", () => {
  const items: Item[] = [
    { kind: "user", text: "Make the ladder faster.\nAnd the top rung easier." },
    tool("Read", "1\tlet x", { file_path: "ladder.fun" }),
    tool("mcp__mis__get_state", "error: no game session is running"),
    { kind: "usage", input: 100, output: 10 },
    { kind: "finished", ok: false, summary: "No game.\nStart one." },
    { kind: "user", text: "Try again" },
    tool("Edit", "ok", { file_path: "ladder.fun" }),
    { kind: "agent", text: "Done.", partial: false },
    { kind: "walkthrough", steps: [{ title: "The climb", note: null, view: { kind: "game" } }], current: 0 },
    { kind: "usage", input: 200, output: 20 },
    { kind: "finished", ok: true, summary: "Done." },
    { kind: "kept" },
    { kind: "user", text: "One more" },
    tool("Bash", null, { command: "ls" }),
  ];
  const page = taskPage(items, 12);
  assert.equal(page.given, "Make the ladder faster.\nAnd the top rung easier.");
  assert.deepEqual(page.followUps, ["Try again", "One more"]);
  const runs = page.runs.map((r) => ({
    n: r.n,
    steps: r.steps.map((s) => `${s.phrase.text}${s.phrase.code ? ` ${s.phrase.code}` : ""}:${s.state}`),
    outcome: r.outcome?.label ?? null,
  }));
  assert.deepEqual(runs, [
    { n: 1, steps: ["Read ladder.fun:ok", "Read the game state:failed"], outcome: "Failed: No game." },
    { n: 2, steps: ["Edited ladder.fun:ok"], outcome: "Done" },
    { n: 3, steps: ["Ran ls:pending"], outcome: null },
  ]);
  assert.deepEqual(page.walkthrough, { item: 8, steps: (items[8] as { steps: unknown }).steps, current: 0 });
  // The same run ended (the app stopped): its open call never answered.
  assert.equal(taskPage(items, null).runs[2]!.steps[0]!.state, "unanswered");
  // No walkthrough, no messages.
  assert.equal(taskPage(items.slice(0, 5), null).walkthrough, null);
  assert.deepEqual(taskPage([], null), { given: null, followUps: [], runs: [], walkthrough: null });
});

test("usage per run and in all, a run without usage yet counted as a turn", () => {
  const items: Item[] = [
    { kind: "user", text: "a" },
    { kind: "usage", input: 2400, output: 75 },
    { kind: "finished", ok: true, summary: null },
    { kind: "user", text: "b" },
    { kind: "usage", input: 3000, output: 40 },
    { kind: "stopped" },
    { kind: "user", text: "c" },
  ];
  assert.deepEqual(usageTable(items), {
    rows: [
      { run: 1, input: 2400, output: 75 },
      { run: 2, input: 3000, output: 40 },
      { run: 3, input: null, output: null },
    ],
    total: { turns: 3, input: 5400, output: 115 },
  });
  assert.deepEqual(usageTable([]), { rows: [], total: { turns: 0, input: 0, output: 0 } });
});
