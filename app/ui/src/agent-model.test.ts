import { test } from "node:test";
import assert from "node:assert/strict";
import type { Activity, AppState, Task, TaskStatus } from "./generated/protocol";
import { bubble, cardLabel, cardTasks, dockTask, room, subBubble } from "./agent-model.ts";

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

const act = (stance: Activity["stance"], over: Partial<Activity> = {}): Activity => ({ stance, calls: [], error: null, subagents: [], context: null, ...over });

const running = { id: 0, usage: null, views: 0 };

test("the bubble shows the first running call and how many run", () => {
  const t = task(1, { kind: "in_progress" }, { run: running });
  const calls = [
    { kind: "files" as const, label: "reading weapons.fun" },
    { kind: "network" as const, label: "running curl x.dev" },
  ];
  assert.deepEqual(bubble(t, act("working", { calls })), { kind: "files", text: "reading weapons.fun", count: 2, error: false });
  // An error keeps the bubble red under a running call, and shows when none runs.
  const error = "Claude requested permissions to use Bas…";
  assert.equal(bubble(t, act("working", { calls, error })).error, true);
  assert.deepEqual(bubble(t, act("thinking", { error })), { kind: null, text: error, count: 0, error: true });
  assert.deepEqual(bubble(t, act("thinking")), { kind: "thinking", text: "thinking", count: 0, error: false });
  // A question shows over an earlier call still running.
  const ask = { kind: "ask" as const, label: "asking: Ten or twenty?" };
  assert.equal(bubble(t, act("waiting", { calls: [calls[0]!, ask] })).text, "asking: Ten or twenty?");
});

test("with nothing running, the bubble says whose turn it is", () => {
  assert.equal(bubble(task(1, { kind: "ready_for_review" }), act("waiting")).text, "change ready for review");
  assert.equal(bubble(task(1, { kind: "waiting_on_you" }), act("waiting")).text, "your turn");
  assert.equal(bubble(task(1, { kind: "done" }), act("done")).text, "done");
  assert.equal(bubble(task(1, { kind: "failed", summary: null }), act("failed")).text, "failed");
});

const state = (tasks: Task[], activity: Record<number, Activity>): AppState =>
  ({ active: "a", tasks, activity }) as unknown as AppState;

test("the strip leaves done tasks out and lists what needs the user first", () => {
  const tasks = [
    task(1, { kind: "in_progress" }, { run: running }),
    task(2, { kind: "waiting_on_you" }),
    task(3, { kind: "kept" }),
    task(4, { kind: "failed", summary: null }),
    task(5, { kind: "ready_for_review" }),
    task(6, { kind: "waiting_on_you" }, { project: "b" }),
  ];
  const s = state(tasks, {
    1: act("working"),
    2: act("waiting"),
    3: act("done"),
    4: act("failed"),
    5: act("waiting"),
    6: act("waiting"),
  });
  assert.deepEqual(cardTasks(s).map((t) => t.id), [5, 2, 4, 1]);
  assert.deepEqual(cardTasks(null), []);
});

test("the dock shows the running task, else the current one", () => {
  const tasks = [task(1, { kind: "done" }), task(2, { kind: "in_progress" }, { run: running })];
  const s = state(tasks, { 1: act("done"), 2: act("thinking") });
  assert.equal(dockTask(s, 1)?.id, 2);
  assert.equal(dockTask(state([tasks[0]!], { 1: act("done") }), 1)?.id, 1);
  assert.equal(dockTask(state([tasks[0]!], { 1: act("done") }), null), null);
  // A task the core sent no activity for has no card.
  assert.equal(dockTask(state([tasks[0]!], {}), 1), null);
});

test("a card's name says who, on what, and what it is doing", () => {
  const t = task(1, { kind: "in_progress" }, { harness: "codex", title: "Faster guns", run: running });
  const calls = [
    { kind: "shell" as const, label: "running cargo test" },
    { kind: "files" as const, label: "reading a.fun" },
  ];
  assert.equal(cardLabel(t, act("working", { calls })), "Codex on “Faster guns”: Working. running cargo test, 2 calls");
  assert.equal(
    cardLabel(t, act("working", { calls: calls.slice(0, 1), error: "refused" })),
    "Codex on “Faster guns”: Working. running cargo test, error: refused",
  );
});

test("the context bar goes amber at 70% and red at 90%, and is absent when unknown", () => {
  assert.equal(room(null), null);
  assert.deepEqual(room({ used: 412_300, window: 1_000_000 }), {
    percent: 41,
    level: "ok",
    title: "Context 41% full: 412.3k of 1M tokens",
  });
  assert.equal(room({ used: 699, window: 1000 })!.level, "ok");
  assert.equal(room({ used: 700, window: 1000 })!.level, "low");
  assert.equal(room({ used: 899, window: 1000 })!.level, "low");
  assert.equal(room({ used: 900, window: 1000 })!.level, "out");
  // Past the window stays a full bar.
  assert.equal(room({ used: 1200, window: 1000 })!.percent, 100);
});

test("a subagent's figure shows its own call, and the card names it and the room left", () => {
  const sub = { label: "Find the spawn code", calls: [{ kind: "files" as const, label: "reading spawn.fun" }] };
  assert.deepEqual(subBubble(sub), { stance: "working", kind: "files", text: "reading spawn.fun" });
  assert.deepEqual(subBubble({ ...sub, calls: [] }), { stance: "thinking", kind: "thinking", text: "thinking" });
  const t = task(1, { kind: "in_progress" }, { title: "Spawn", run: running });
  const calls = [{ kind: "agents" as const, label: "subagent: Find the spawn code" }];
  assert.equal(
    cardLabel(t, act("working", { calls, subagents: [sub], context: { used: 75, window: 100 } })),
    "Claude on “Spawn”: Working. subagent: Find the spawn code; subagent “Find the spawn code”: reading spawn.fun; context 75% full",
  );
});
