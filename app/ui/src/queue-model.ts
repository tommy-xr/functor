// The queue's pure parts (D30): why a project's queue waits, and where the
// composer's message goes ("Steers …" / "Starts a new task"). No React, so
// `node --test` runs them.

import type { AppState, Task, Wait } from "./generated/protocol";
import { untitled } from "./tasks-model.ts";

/** Task `id`'s title in quotes, or "task <id>" when it is not here. */
function named(state: AppState, id: number): string {
  const task = state.tasks.find((t) => t.id === id);
  return task ? `“${untitled(task)}”` : `task ${id}`;
}

/** Why the queue waits, as the Tasks pane says it: "Starts when “X” ends." */
export function waitLine(state: AppState, wait: Wait | null): string {
  if (!wait) return "Starting.";
  const who = named(state, wait.task);
  switch (wait.kind) {
    case "running":
      return `Starts when ${who} ends.`;
    case "reverting":
      return `Starts once ${who}’s revert is done.`;
    case "decision":
      return `Waiting for your decision on ${who}: its change and the next one’s would mix.`;
    case "measuring":
      return `Waiting for ${who}’s change to be measured: open its changes.`;
  }
}

/** A queued task's place shown in its row: "Next", "2nd in line", …. */
export function placeLine(index: number): string {
  if (index === 0) return "Next";
  const n = index + 1;
  const suffix = n % 10 === 2 && n % 100 !== 12 ? "nd" : n % 10 === 3 && n % 100 !== 13 ? "rd" : "th";
  return `${n}${suffix} in line`;
}

/** Whether a task the user starts now in the active project waits (the core's `queue::holds`). */
function holds(state: AppState): Wait | null {
  const q = state.active ? state.queues[state.active] : undefined;
  if (!q?.wait) return null;
  return q.tasks.length > 0 || q.wait.kind === "running" || q.wait.kind === "reverting" ? q.wait : null;
}

/** Where a typed message goes: into a task (`send_message`) or a new one (`new_task`). */
export type Destination = { kind: "task"; task: number; line: string; switchTo: string } | { kind: "new"; line: string; switchTo: string };

/** "Starts a new task", or why and behind what it queues. */
function newLine(state: AppState): string {
  const wait = holds(state);
  if (!wait) return "Starts a new task";
  const who = named(state, wait.task);
  switch (wait.kind) {
    case "running":
      return `Queues a new task behind ${who}`;
    case "reverting":
      return `Queues a new task until ${who}’s revert is done`;
    case "decision":
      return `Queues a new task: waiting for your decision on ${who}`;
    case "measuring":
      return `Queues a new task: waiting for ${who}’s change to be measured`;
  }
}

/**
 * The composer's two destinations for the chat's `current` task, the first
 * the default: a running task is steered, a queued one added to, another
 * replied in, else a new task; the second is the one-click switch (a new
 * task, or steering the running task from a new one). `null`: no switch.
 */
export function destinations(state: AppState | null, current: Task | null): [Destination, Destination | null] | null {
  if (!state?.active) return null;
  const fresh: Destination = { kind: "new", line: newLine(state), switchTo: "New task instead" };
  if (current) {
    const title = `“${untitled(current)}”`;
    const verb = current.run !== null ? "Steers" : current.queued !== null ? "Adds to" : "Replies in";
    const word = current.run !== null ? "Steer" : current.queued !== null ? "Add to" : "Reply in";
    return [{ kind: "task", task: current.id, line: `${verb} ${title}`, switchTo: `${word} ${title} instead` }, fresh];
  }
  const running = state.tasks.find((t) => t.project === state.active && t.run !== null);
  if (!running) return [fresh, null];
  const title = `“${untitled(running)}”`;
  return [fresh, { kind: "task", task: running.id, line: `Steers ${title}`, switchTo: `Steer ${title} instead` }];
}
