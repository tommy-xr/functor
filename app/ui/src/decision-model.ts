// The decision page's (D35) pure parts: the size line, the verdict's words,
// and the evidence list from the agent's checks and the app's own predicted
// path. No React, so `node --test` runs them.

import type { Depth, Kind, Size, Task } from "./generated/protocol";
import { plural } from "./diff-model.ts";

/** The task page's tabs, in order: the first page, then depth (D31). */
export const TABS = ["decision", "highlights", "captures", "changes"] as const;
export type Tab = (typeof TABS)[number];

export const TAB_WORDS: Record<Tab, string> = {
  decision: "Decision",
  highlights: "Highlights",
  captures: "Captures",
  changes: "Changes",
};

export const DEPTH_WORDS: Record<Depth, string> = {
  glance: "Approve at a glance",
  look: "Look into this",
};

const KIND_WORDS: Record<Kind, string> = {
  game_logic: "game logic",
  assets: "assets",
  config: "config",
  other: "other files",
};

/** "game logic", "game logic, assets". */
export const kindsLine = (size: Size) => size.kinds.map((k) => KIND_WORDS[k]).join(", ");

/** "1 file · game logic", "3 files · game logic, assets". */
export function sizeLine(size: Size): string {
  const kinds = kindsLine(size);
  return kinds ? `${plural(size.files, "file")} · ${kinds}` : plural(size.files, "file");
}

/** `ok`: passed; `false`: failed; `null`: neither (a fact, or still being taken). */
export type EvidenceLine = { text: string; code?: string; ok: boolean | null; tab?: Tab | undefined };

/** The agent's checks, then what the app compared itself: the predicted path. */
export function evidenceLines(task: Task): EvidenceLine[] {
  const lines: EvidenceLine[] = (task.report?.evidence ?? []).map((c) => ({ code: c.ran, text: c.result, ok: c.ok }));
  const state = task.evidence?.state;
  switch (state?.kind) {
    // The same words while it is taken: the line only gains its tick.
    case "shown":
    case "taking":
      lines.push({ text: "Predicted path before and after", ok: state.kind === "shown" || null, tab: "captures" });
      break;
    case "same":
      lines.push({ text: "The predicted path is the same before and after", ok: null });
      break;
    case "withheld":
      lines.push({ text: `No predicted path: ${state.reason}`, ok: null });
      break;
  }
  return lines;
}

/** The first `max` items, and how many more there are. */
export function firstOf<T>(items: readonly T[], max: number): { shown: T[]; more: number } {
  return { shown: items.slice(0, max), more: Math.max(0, items.length - max) };
}

/** Lines visible before "Show N more": at most three per list. */
export const VISIBLE = 3;
