// How much of the user's own setup an agent run reads (D39): the project's
// level, a request's own, and what each level leaves out. No React, so
// `node --test` runs them.

import type { AppState, Harness, Scope, Task } from "./generated/protocol";

export const SCOPES: Scope[] = ["user_and_project", "project", "isolated"];

/** Each level's name, its word on the composer's button, and what it reads. */
export const SCOPE_WORDS: Record<Scope, { name: string; short: string; help: string }> = {
  user_and_project: {
    name: "Yours and the project's",
    short: "Setup",
    help: "Your own instructions, skills, memory and settings, and the project's: as in your terminal.",
  },
  project: {
    name: "The project's only",
    short: "Project only",
    help:
      "What the project has checked in. Not your instructions, skills or memory, nor the permission rules " +
      "and hooks in your settings, nor the project's own extra MCP servers.",
  },
  isolated: {
    name: "Isolated",
    short: "Isolated",
    help: "Only the app's opening instructions and tools. None of your setup or the project's: the cheapest and most repeatable.",
  },
};

/** Codex has no way to leave out the user's own instructions and skills. */
export const CODEX_NOTE = "Codex always reads your own instructions and skills: it cannot leave them out.";

/** The active project's level (the core's default when none is saved). */
export function projectScope(state: AppState | null): Scope {
  return (state?.active && state.scope_choice[state.active]) || "user_and_project";
}

/** The levels `harness` can run at. */
export function scopesFor(harness: Harness): Scope[] {
  return harness === "codex" ? ["user_and_project"] : SCOPES;
}

/** What the next request reads: the level picked for it, else the task's last, else the project's; Codex all of it. */
export function requestScope(pick: Scope | null, task: Task | null, state: AppState | null, harness: Harness): Scope {
  const scope = pick ?? task?.scope ?? projectScope(state);
  return scopesFor(harness).includes(scope) ? scope : "user_and_project";
}
