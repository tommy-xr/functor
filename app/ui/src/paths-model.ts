// The words around a task's predicted paths (D38): which moment and which
// inputs both paths share, and what to say when there are none.
import type { Task } from "./generated/protocol";

/** "frame 212, replaying the same 340 recorded input steps": what both paths share. */
export function momentPhrase(task: Task): string | null {
  const m = task.moment;
  if (m?.kind !== "taken") return null;
  const inputs =
    m.steps === 0 ? "with no recorded input" : `replaying the same ${m.steps} recorded input ${m.steps === 1 ? "step" : "steps"}`;
  return `frame ${m.frame}, ${inputs}`;
}

/**
 * The task page's line when no pair is shown; `null` when the pair shows
 * or there is nothing to say (no code change, no moment).
 */
export function pathsNote(task: Task): string | null {
  const state = task.evidence?.state;
  const from = momentPhrase(task);
  switch (state?.kind) {
    case undefined:
    case "shown":
      return null;
    case "taking":
      return "Predicting the path before and after the change…";
    case "same":
      return `The change moves nothing on the predicted path${from ? ` from ${from}` : ""}.`;
    case "withheld":
      return `No predicted path: ${state.reason}.`;
  }
}
