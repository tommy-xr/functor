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
      "The project's own instructions, skills and settings, and your local settings for it " +
      "(.claude/settings.local.json). Not your own instructions, skills, memory, MCP servers or plugins, " +
      "nor the permission rules and hooks in your user settings, nor the project's extra MCP servers (.mcp.json).",
  },
  isolated: {
    name: "Isolated",
    short: "Isolated",
    help: "Only the app's opening instructions and tools: none of your setup or the project's. The fewest tokens a turn.",
  },
};

/** A task keeps its level: its session already holds what it read. */
export const KEPT_NOTE = "A task keeps the level it started with: its conversation already holds what it read.";

/** The app's limits stay; the allow rules and hooks in a setting go with it. */
export const LIMITS_NOTE =
  "The app's own limits apply at every level: edits stay inside the open folder. Allow rules and hooks come " +
  "only with the settings that hold them, so a shell command your settings allow is denied at a level that leaves them out.";

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

/** What a task reads: its own level once it exists (its follow-ups resume a session that already holds it), else the one picked for it, else the project's; Codex all of it. */
export function taskScope(pick: Scope | null, task: Task | null, state: AppState | null, harness: Harness): Scope {
  const scope = task ? task.scope : (pick ?? projectScope(state));
  return scopesFor(harness).includes(scope) ? scope : "user_and_project";
}
