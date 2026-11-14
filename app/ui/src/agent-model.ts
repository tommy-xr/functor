// The agent cards' pure parts (D34): which tasks have a card, and what each
// card's bubble and stance say. No React, so `node --test` runs them.

import type { Activity, AppState, CallKind, ContextFill, Stance, Subagent, Task } from "./generated/protocol";
import { tokens } from "./chat-model.ts";
import { newestFirst, untitled } from "./tasks-model.ts";

export const STANCE_WORDS: Record<Stance, string> = {
  thinking: "Thinking",
  working: "Working",
  waiting: "Waiting for you",
  done: "Done",
  failed: "Failed",
};

export const ENGINE_WORDS: Record<Task["harness"], string> = { claude: "Claude", codex: "Codex" };

/** What the bubble shows: the running question, else the first call (and how many run), else the error, else the stance. */
export type Bubble = { kind: CallKind | null; text: string; count: number; error: boolean };

export function bubble(task: Task, activity: Activity): Bubble {
  const error = activity.error !== null;
  // A question is what the user is waiting on.
  const call = activity.calls.find((c) => c.kind === "ask") ?? activity.calls[0];
  if (call) return { kind: call.kind, text: call.label, count: activity.calls.length, error };
  if (activity.error) return { kind: null, text: activity.error, count: 0, error };
  const text =
    activity.stance === "waiting" ? (task.status.kind === "ready_for_review" ? "change ready for review" : "your turn")
    : activity.stance === "thinking" ? "thinking"
    : STANCE_WORDS[activity.stance].toLowerCase();
  return { kind: activity.stance === "thinking" ? "thinking" : null, text, count: 0, error };
}

/** Stances in the order the strip lists them: what needs the user first, as the task list groups them. */
const ORDER: Stance[] = ["waiting", "failed", "working", "thinking", "done"];

/** The Tasks pane's cards: the active project's tasks not done, the user's turn first, then newest. */
export function cardTasks(state: AppState | null): Task[] {
  if (!state?.active) return [];
  const stance = (t: Task) => ORDER.indexOf(state.activity[t.id]?.stance ?? "done");
  return state.tasks
    .filter((t) => t.project === state.active && stance(t) < ORDER.indexOf("done"))
    .sort((a, b) => stance(a) - stance(b) || newestFirst(a, b));
}

/** The chat dock's card: the active project's running task, else the chat's current one. */
export function dockTask(state: AppState | null, current: number | null): Task | null {
  const mine = (state?.tasks ?? []).filter((t) => t.project === state?.active && state.activity[t.id]);
  return mine.find((t) => t.run !== null) ?? mine.find((t) => t.id === current) ?? null;
}

/** How full the context is: amber from 70%, red from 90%, like a health bar running out. */
export type Room = { percent: number; level: "ok" | "low" | "out"; title: string };

export function room(fill: ContextFill | null): Room | null {
  if (!fill || fill.window <= 0) return null;
  const share = fill.used / fill.window;
  const level = share >= 0.9 ? "out" : share >= 0.7 ? "low" : "ok";
  const percent = Math.min(100, Math.floor(share * 100));
  return { percent, level, title: `Context ${percent}% full: ${tokens(fill.used)} of ${tokens(fill.window)} tokens` };
}

/** A subagent's figure: working while a call of its own runs, its line that call or "thinking". */
export function subBubble(sub: Subagent): { stance: Stance; kind: CallKind; text: string } {
  const call = sub.calls[0];
  return call ? { stance: "working", kind: call.kind, text: call.label } : { stance: "thinking", kind: "thinking", text: "thinking" };
}

/** The card's accessible name: who, on what, its stance and bubble, its subagents and context. */
export function cardLabel(task: Task, activity: Activity): string {
  const b = bubble(task, activity);
  const more = b.count > 1 ? `, ${b.count} calls` : "";
  const error = b.error && activity.calls.length > 0 ? `, error: ${activity.error}` : "";
  const subs = activity.subagents.map((s) => `; subagent “${s.label}”: ${subBubble(s).text}`).join("");
  const r = room(activity.context);
  const context = r ? `; context ${r.percent}% full` : "";
  return `${ENGINE_WORDS[task.harness]} on “${untitled(task)}”: ${STANCE_WORDS[activity.stance]}. ${b.text}${more}${error}${subs}${context}`;
}
