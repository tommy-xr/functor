import { test } from "node:test";
import assert from "node:assert/strict";
import { requestLine, shownRequest, spokenTask } from "./progress-model.ts";
import type { AppState, Request } from "./generated/protocol";

const spoken = (over: Partial<Request> = {}): Request => ({
  id: 1,
  project: "p",
  headset: "Quest 3",
  text: null,
  task: null,
  line: null,
  steps: [{ step: "received", at: 1000 }],
  outcome: null,
  ended: null,
  ...over,
});

test("a request's line moves through its steps, the done ones timed", () => {
  assert.deepEqual(requestLine(spoken(), null, 1500), { done: [], now: "Hearing from Quest 3…", slow: null, failed: false });
  const transcribing = spoken({ steps: [{ step: "received", at: 1000 }, { step: "transcribing", at: 1800 }] });
  const line = requestLine(transcribing, null, 4900);
  assert.deepEqual(line.done, ["Received 0.8 s"]);
  assert.equal(line.now, "Transcribing…");
  assert.equal(line.slow, "3.1 s", "past 2 s it says it is still going");
  const agent = spoken({
    task: 4,
    text: "make it red",
    line: "Started a new task",
    steps: [
      { step: "received", at: 0 },
      { step: "transcribing", at: 500 },
      { step: "transcribed", at: 1500 },
      { step: "processing", at: 1600 },
      { step: "agent", at: 2600 },
    ],
  });
  const activity = { stance: "thinking", calls: [], error: null, subagents: [], context: null } as const;
  assert.equal(requestLine(agent, activity, 3000).now, "Thinking…");
  const reloaded = { ...agent, outcome: { kind: "changed", reload: "reloaded" } as const, ended: 9600 };
  const done = requestLine(reloaded, null, 9700);
  assert.equal(done.now, "Changed, and the game reloaded");
  assert.deepEqual(done.done, ["Received 0.5 s", "Transcribed 1.0 s", "Placed 0.1 s", "Started 1.0 s", "Worked 7.0 s"]);
  const away = { ...agent, outcome: { kind: "changed", reload: "when_back" } as const, ended: 9600 };
  assert.equal(requestLine(away, null, 9700).now, "Changed: it loads when Quest 3 is back");
});

test("a failure names its step", () => {
  const failed = spoken({
    steps: [{ step: "received", at: 0 }, { step: "transcribing", at: 10 }],
    outcome: { kind: "failed", at: "transcribing", reason: "heard no words" },
    ended: 20,
  });
  const line = requestLine(failed, null, 30);
  assert.equal(line.now, "Could not transcribe it: heard no words");
  assert.ok(line.failed);
  const cancelled = spoken({ outcome: { kind: "cancelled" }, ended: 1100 });
  assert.equal(requestLine(cancelled, null, 1200).now, "Cancelled in the headset");
});

const state = (requests: Request[]): AppState => ({ active: "p", requests }) as unknown as AppState;

test("the dock shows the newest request here, a while after it ends", () => {
  const typed = spoken({ id: 2, headset: null, task: 7, steps: [{ step: "processing", at: 0 }] });
  assert.equal(shownRequest(state([spoken(), typed]), 7, 0)?.id, 2);
  // Another task's typed request is not this dock's.
  assert.equal(shownRequest(state([typed]), 8, 0), null);
  const ended = { ...typed, outcome: { kind: "done" } as const, ended: 1000 };
  assert.equal(shownRequest(state([ended]), 7, 5000)?.id, 2);
  assert.equal(shownRequest(state([ended]), 7, 20_000), null);
});

test("a spoken request's task shows unless that would switch under the user", () => {
  const placed = spoken({ id: 3, task: 9 });
  const s = state([placed]);
  assert.deepEqual(spokenTask(s, 0, 4, false), { seen: 3, show: 9 });
  assert.deepEqual(spokenTask(s, 0, 4, true), { seen: 3, show: null }, "busy: its line is the way there");
  assert.deepEqual(spokenTask(s, 3, 4, false), { seen: 3, show: null }, "seen already");
  assert.deepEqual(spokenTask(state([spoken({ id: 4 })]), 3, 4, false), { seen: 3, show: null }, "not placed yet");
});
