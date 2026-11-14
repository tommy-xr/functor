import { test } from "node:test";
import assert from "node:assert/strict";
import type { AppState, Queue, Task } from "./generated/protocol";
import { destinations, placeLine, waitLine } from "./queue-model.ts";
import { groupTasks } from "./tasks-model.ts";

const task = (id: number, over: Partial<Task> = {}): Task => ({
  id,
  project: "a",
  title: `task ${id}`,
  status: { kind: "done" },
  harness: "claude",
  model: null,
  session_id: null,
  run: null,
  queued: null,
  baseline: null,
  change: null,
  reverting: false,
  created_at: null,
  updated_at: null,
  ...over,
});

const running = { run: { id: 0, usage: null, views: 0 }, status: { kind: "in_progress" } } as const;
const queued = (place: number) => ({ queued: place, status: { kind: "queued" } }) as const;

const state = (tasks: Task[], queue?: Queue): AppState =>
  ({ active: "a", tasks, queues: queue ? { a: queue } : {} }) as unknown as AppState;

test("the composer steers a running task, adds to a queued one and replies in another", () => {
  const r = task(1, running);
  const q = task(2, queued(1));
  const s = state([r, q, task(3)], { tasks: [2], wait: { kind: "running", task: 1 }, holds: true });
  const [steer, fresh] = destinations(s, r)!;
  assert.deepEqual(steer, { kind: "task", task: 1, line: "Steers “task 1”", switchTo: "Steer “task 1” instead" });
  assert.deepEqual(fresh, { kind: "new", line: "Queues a new task behind “task 1”", switchTo: "New task instead" });
  assert.equal(destinations(s, q)![0].line, "Adds to “task 2”");
  assert.equal(destinations(s, task(3))![0].line, "Replies in “task 3”");
});

test("a new task says whether it starts or queues, and can steer the running task instead", () => {
  assert.deepEqual(destinations(state([task(3)]), null), [
    { kind: "new", line: "Starts a new task", switchTo: "New task instead" },
    null,
  ]);
  const s = state([task(1, running)], { tasks: [], wait: { kind: "running", task: 1 }, holds: true });
  const [fresh, steer] = destinations(s, null)!;
  assert.equal(fresh.line, "Queues a new task behind “task 1”");
  assert.deepEqual(steer, { kind: "task", task: 1, line: "Steers “task 1”", switchTo: "Steer “task 1” instead" });
  // A change waiting for a decision holds only the queue: with none in line a new task starts.
  const decision = { kind: "decision", task: 4 } as const;
  const waiting = [task(4, { title: "Spin" })];
  assert.equal(destinations(state(waiting, { tasks: [], wait: decision, holds: false }), null)![0].line, "Starts a new task");
  const line = destinations(state([...waiting, task(5, queued(1))], { tasks: [5], wait: decision, holds: true }), null)![0].line;
  assert.equal(line, "Queues a new task: waiting for your decision on “Spin”");
  assert.equal(destinations(null, null), null);
});

test("the queue says what it waits for, and each row its place", () => {
  const s = state([task(1, { title: "Spin" })]);
  assert.equal(waitLine(s, { kind: "running", task: 1 }), "Starts when “Spin” ends.");
  assert.equal(
    waitLine(s, { kind: "decision", task: 1 }),
    "Waiting for your decision on “Spin”: its change and the next one’s would mix.",
  );
  assert.equal(waitLine(s, { kind: "measuring", task: 9 }), "Waiting for task 9’s change to be measured: open its changes.");
  assert.deepEqual(
    [0, 1, 2, 3, 10, 11, 12, 19, 20, 21].map(placeLine),
    ["Next", "2nd in line", "3rd in line", "4th in line", "11th in line", "12th in line", "13th in line", "20th in line", "21st in line", "22nd in line"],
  );
});

test("the Queued group lists tasks in line, after those in progress", () => {
  const tasks = [task(1, running), task(2, { ...queued(5), updated_at: 9 }), task(3, queued(2)), task(4)];
  const groups = groupTasks(tasks, "a").map((g) => [g.group, g.tasks.map((t) => t.id)]);
  assert.deepEqual(groups, [
    ["progress", [1]],
    ["queued", [3, 2]],
    ["done", [4]],
  ]);
});
