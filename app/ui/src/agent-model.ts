// The agent cards' pure parts (D34): which tasks have a card, and what each
// card's bubble and stance say. No React, so `node --test` runs them.

import type { Activity, AppState, CallKind, Stance, Task } from "./generated/protocol";

export const STANCE_WORDS: Record<Stance, string> = {
  thinking: "Thinking",
  working: "Working",
  waiting: "Waiting for you",
  done: "Done",
  failed: "Failed",
};

export const ENGINE_WORDS: Record<Task["harness"], string> = { claude: "Claude", codex: "Codex" };

/** What the bubble shows: the first running call (and how many run), else the error, else the stance. */
export type Bubble = { kind: CallKind | null; text: string; count: number; error: boolean };

export function bubble(task: Task, activity: Activity): Bubble {
  const error = activity.error !== null;
  const call = activity.calls[0];
  if (call) return { kind: call.kind, text: call.label, count: activity.calls.length, error };
  if (activity.error) return { kind: null, text: activity.error, count: 0, error };
  const text =
    activity.stance === "waiting" ? (task.status.kind === "ready_for_review" ? "change ready for review" : "your turn")
    : activity.stance === "thinking" ? "thinking"
    : STANCE_WORDS[activity.stance].toLowerCase();
  return { kind: activity.stance === "thinking" ? "thinking" : null, text, count: 0, error };
}

/** Stances in the order the strip lists them: the user's turn first. */
const ORDER: Stance[] = ["waiting", "working", "thinking", "failed", "done"];

/** The Tasks pane's cards: the active project's tasks not done, the user's turn first, then newest. */
export function cardTasks(state: AppState | null): Task[] {
  if (!state?.active) return [];
  const stance = (t: Task) => ORDER.indexOf(state.activity[t.id]?.stance ?? "done");
  return state.tasks
    .filter((t) => t.project === state.active && stance(t) < ORDER.indexOf("done"))
    .sort((a, b) => stance(a) - stance(b) || (b.updated_at ?? 0) - (a.updated_at ?? 0) || b.id - a.id);
}

/** The chat dock's card: the active project's running task, else the chat's current one. */
export function dockTask(state: AppState | null, current: number | null): Task | null {
  const mine = (state?.tasks ?? []).filter((t) => t.project === state?.active && state.activity[t.id]);
  return mine.find((t) => t.run !== null) ?? mine.find((t) => t.id === current) ?? null;
}

/** The card's accessible name: who, on what, its stance and bubble. */
export function cardLabel(task: Task, activity: Activity): string {
  const b = bubble(task, activity);
  const more = b.count > 1 ? `, ${b.count} calls` : "";
  const error = b.error && activity.calls.length > 0 ? `, error: ${activity.error}` : "";
  return `${ENGINE_WORDS[task.harness]} on “${task.title || "Untitled task"}”: ${STANCE_WORDS[activity.stance]}. ${b.text}${more}${error}`;
}
