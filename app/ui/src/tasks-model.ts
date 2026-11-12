// The Tasks pane's pure parts: which group a task is in, the groups in
// order, each row's secondary line, relative times and the chat's current
// task. No React, so `node --test` runs them.

import type { AppState, Change, Item, Task, TaskStatus } from "./generated/protocol";
import { firstLine, initialTask } from "./chat-model.ts";
import { hasChange, plural, totals } from "./diff-model.ts";

export type Group = "waiting" | "review" | "progress" | "done";

/** The groups in the order the pane lists them; Done starts collapsed. */
export const GROUPS: readonly { group: Group; label: string }[] = [
  { group: "waiting", label: "Waiting on you" },
  { group: "review", label: "Ready for review" },
  { group: "progress", label: "In progress" },
  { group: "done", label: "Done" },
];

/**
 * A task's group, from its status alone: a failed run waits on the user
 * with the ones that stopped; a kept or reverted change, or a run that
 * finished with nothing to review, is done.
 */
export function taskGroup(status: TaskStatus): Group {
  switch (status.kind) {
    case "waiting_on_you":
    case "failed":
      return "waiting";
    case "ready_for_review":
      return "review";
    case "in_progress":
      return "progress";
    case "done":
    case "kept":
    case "reverted":
      return "done";
  }
}

/** Most recently active first; tasks without a time (none yet) by id. */
function newestFirst(a: Task, b: Task): number {
  return (b.updated_at ?? 0) - (a.updated_at ?? 0) || b.id - a.id;
}

/**
 * `project`'s tasks whose title contains `query` (any case), by group in
 * `GROUPS` order, empty groups left out.
 */
export function groupTasks(tasks: readonly Task[], project: string | null, query = "") {
  const q = query.trim().toLowerCase();
  const mine = tasks.filter((t) => t.project === project && (!q || (t.title || "Untitled task").toLowerCase().includes(q)));
  return GROUPS.map(({ group, label }) => ({
    group,
    label,
    tasks: mine.filter((t) => taskGroup(t.status) === group).sort(newestFirst),
  })).filter((g) => g.tasks.length > 0);
}

/** How many of `project`'s tasks need the user: waiting on them or ready for review. */
export function needsYou(tasks: readonly Task[], project: string | null): number {
  return tasks.filter((t) => t.project === project && ["waiting", "review"].includes(taskGroup(t.status))).length;
}

/** "+3 −1 in 2 files". */
export function changeSummary(change: Change): string {
  const sum = totals(change);
  return `+${sum.added} −${sum.removed} in ${plural(sum.files, "file")}`;
}

/** The first line of the agent's last text, if its transcript is here. */
export function lastAgentLine(items: readonly Item[] | undefined): string {
  for (let i = (items?.length ?? 0) - 1; i >= 0; i--) {
    const item = items![i]!;
    if (item.kind === "agent" && item.text.trim()) return firstLine(item.text);
  }
  return "";
}

/**
 * A row's second line: a change waiting for review, else why it failed,
 * else the agent's last line, else any change left.
 */
export function secondaryLine(task: Task, items: readonly Item[] | undefined): string {
  if (task.status.kind === "ready_for_review" && hasChange(task)) return changeSummary(task.change);
  if (task.status.kind === "failed" && task.status.summary) return firstLine(task.status.summary);
  return lastAgentLine(items) || (hasChange(task) ? changeSummary(task.change) : "");
}

/** Why `task` cannot be deleted now, as the core would refuse it; `null` if it can. */
export function deleteBlock(task: Task, tracksChanges: boolean): string | null {
  if (task.run !== null || task.reverting) return "It is running: stop it first.";
  if (tracksChanges && hasChange(task)) return "It has a change to review: keep or revert it first.";
  return null;
}

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/** `at` (ms since the epoch) relative to `now`: "just now", "5 min ago", "3 h ago", "yesterday", "4 days ago", "Mar 3". */
export function relativeTime(at: number | null, now: number): string {
  if (at === null) return "";
  const ago = Math.max(0, now - at);
  if (ago < MINUTE) return "just now";
  if (ago < HOUR) return `${Math.floor(ago / MINUTE)} min ago`;
  if (ago < DAY) return `${Math.floor(ago / HOUR)} h ago`;
  if (ago < 2 * DAY) return "yesterday";
  if (ago < 7 * DAY) return `${Math.floor(ago / DAY)} days ago`;
  const d = new Date(at);
  return `${MONTHS[d.getMonth()]} ${d.getDate()}`;
}

/**
 * The chat's current task in the active project, as the core holds it:
 * a task id, `null` for a new task, `undefined` while none is chosen.
 */
export function chosenTask(state: AppState | null): number | null | undefined {
  if (!state?.active) return undefined;
  return state.current_task[state.active];
}

/** Where a page remembers the core's choice for the active project (`currentTask`). */
export function choiceKey(state: AppState): string {
  return `${state.repo_root}\n${state.active}`;
}

/**
 * The chat's current task: the core's choice; until it has one (none yet,
 * or the app restarted), the one this page last saw chosen while that task
 * is still there, else the newest open task.
 */
export function currentTask(state: AppState | null, seen: ReadonlyMap<string, number | null>): number | null {
  if (!state?.active) return null;
  const chosen = chosenTask(state);
  if (chosen !== undefined) return chosen;
  const last = seen.get(choiceKey(state));
  if (last === null || state.tasks.some((t) => t.id === last && t.project === state.active)) return last ?? null;
  return initialTask(state.tasks, state.active);
}
