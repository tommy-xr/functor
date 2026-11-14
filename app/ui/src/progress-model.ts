// Every request shows where it is (D43): the chat's one line for a
// request's record, and which record (and task) the dock shows. Pure, so
// `node --test` runs them.

import type { Activity, AppState, Request, RequestStep } from "./generated/protocol";
import { STANCE_WORDS } from "./agent-model.ts";
import { firstLine } from "./chat-model.ts";

/** A step running longer than this says it is still going. */
export const SLOW_MS = 2000;
/** How long a request's line stays after its outcome. */
export const SHOWN_MS = 10_000;

/** Each step, once done: what it did. */
const DONE: Record<RequestStep, string> = {
  received: "Received",
  transcribing: "Transcribed",
  transcribed: "Placed",
  processing: "Started",
  agent: "Worked",
};

/** A failure names the step that failed. */
const FAILED_AT: Record<RequestStep, string> = {
  received: "Could not receive it",
  transcribing: "Could not transcribe it",
  transcribed: "Could not place it",
  processing: "The agent did not start",
  agent: "The agent failed",
};

/** `1234` → `1.2 s`. */
export function seconds(ms: number): string {
  return `${(Math.max(0, ms) / 1000).toFixed(1)} s`;
}

export type Line = {
  /** The steps done, each with how long it took. */
  done: string[];
  /** Where it is now, or how it ended. */
  now: string;
  /** The current step has run past `SLOW_MS`: how long so far. */
  slow: string | null;
  failed: boolean;
};

/** `r`'s line at wall time `now`; `activity`: its task's agent's (D34). */
export function requestLine(r: Request, activity: Activity | null, now: number): Line {
  const steps = r.steps;
  const done = steps.slice(0, -1).map((s, i) => `${DONE[s.step]} ${seconds(steps[i + 1]!.at - s.at)}`);
  const last = steps.at(-1);
  const o = r.outcome;
  if (o) {
    if (last) done.push(`${DONE[last.step]} ${seconds((r.ended ?? now) - last.at)}`);
    const headset = r.headset ?? "the headset";
    const text =
      o.kind === "answered" ? `Answered: ${o.text}`
      : o.kind === "changed" ?
        o.reload === "reloaded" ? "Changed, and the game reloaded"
        : o.reload === "when_back" ? `Changed: it loads when ${headset} is back`
        : o.reload === "waiting" ? `Changed: reloading on ${headset}…`
        : "Changed"
      : o.kind === "needs_you" ? "Changed: keep or revert it"
      : o.kind === "done" ? "Done"
      : o.kind === "cancelled" ? "Cancelled in the headset"
      : `${FAILED_AT[o.at]}: ${firstLine(o.reason, Infinity)}`;
    return { done, now: text, slow: null, failed: o.kind === "failed" };
  }
  const step = last?.step ?? "received";
  const label =
    step === "received" ? `Hearing from ${r.headset ?? "the headset"}…`
    : step === "transcribing" ? "Transcribing…"
    : step === "transcribed" ? `“${r.text ?? ""}”: deciding where it goes…`
    : step === "processing" ? `${r.line ?? "Processing"}…`
    : `${activity ? STANCE_WORDS[activity.stance] : "Working"}…`;
  const running = last ? now - last.at : 0;
  return { done, now: label, slow: running > SLOW_MS ? seconds(running) : null, failed: false };
}

/**
 * The request the dock shows for the active project: the newest one still
 * running or ended within `SHOWN_MS`, in the current task, or not in a task
 * yet. One said in a headset that went to another task shows too: the
 * dock then offers to switch to it.
 */
export function shownRequest(state: AppState | null, current: number | null, now: number): Request | null {
  if (!state?.active) return null;
  const recent = (r: Request) => r.ended === null || now - r.ended < SHOWN_MS;
  const here = (r: Request) => r.task === null || r.task === current || r.headset !== null;
  const mine = state.requests.filter((r) => r.project === state.active && recent(r) && here(r));
  return mine.at(-1) ?? null;
}

/**
 * Spoken requests that went to a task since the page last looked (ids above
 * `seen`): the task to show for the newest, unless switching would land
 * under the user (`busy`: a draft, or a task chosen moments ago); then the
 * dock leaves its line as the way there.
 */
export function spokenTask(state: AppState, seen: number, current: number | null, busy: boolean): { seen: number; show: number | null } {
  const placed = state.requests.filter((r) => r.headset !== null && r.task !== null && r.project === state.active);
  const newest = placed.at(-1);
  if (!newest || newest.id <= seen) return { seen, show: null };
  const show = !busy && newest.task !== current ? newest.task : null;
  return { seen: newest.id, show };
}
