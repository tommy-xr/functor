// The Diff tab's pure parts: matching a tool call's file to the change,
// totals, collapsing, and the lines as React elements. Every diff text is
// untrusted (the agent wrote it): it only ever becomes a text node. No JSX,
// so `node --test` runs them.

import { createElement as h, type ReactNode } from "react";
import type { Change, DiffLine, FileChange, LineKind, Omitted, Task } from "./generated/protocol";

/** A file's lines shown before "Show more". */
export const COLLAPSE_LINES = 80;

/** A non-empty change to keep or revert. */
export function hasChange(task: Task | null | undefined): task is Task & { change: Change } {
  return !!task?.baseline && !!task.change && (task.change.files.length > 0 || task.change.more > 0);
}

/** The change's entry for a tool call's `file_path` (absolute, or relative to the repo). */
export function fileFor(change: Change | null | undefined, repoRoot: string, filePath: string): FileChange | null {
  if (!change) return null;
  const root = repoRoot.endsWith("/") ? repoRoot : `${repoRoot}/`;
  const rel = filePath.startsWith(root) ? filePath.slice(root.length) : filePath;
  return (
    change.files.find((f) => f.path === rel) ??
    // A path from another checkout of the same repo, or relative to the project.
    change.files.find((f) => filePath.endsWith(`/${f.path}`) || f.path.endsWith(`/${rel}`)) ??
    null
  );
}

export type Totals = { files: number; added: number; removed: number };

export function totals(change: Change): Totals {
  return {
    files: change.files.length + change.more,
    added: change.files.reduce((n, f) => n + f.added, 0),
    removed: change.files.reduce((n, f) => n + f.removed, 0),
  };
}

export function plural(n: number, word: string): string {
  return `${n} ${word}${n === 1 ? "" : "s"}`;
}

/** The lines shown, and how many are held back until "Show more". */
export function collapse(lines: DiffLine[], expanded: boolean, limit = COLLAPSE_LINES) {
  if (expanded || lines.length <= limit) return { shown: lines, hidden: 0 };
  return { shown: lines.slice(0, limit), hidden: lines.length - limit };
}

const MARKS: Record<LineKind, string> = { hunk: "", context: " ", added: "+", removed: "-", note: "\\" };

/** One diff line: its marker and text, both text nodes. */
export function renderLine(line: DiffLine, key: number): ReactNode {
  return h(
    "div",
    { key, className: `dl dl-${line.kind}` },
    h("span", { className: "dl-mark", "aria-hidden": true }, MARKS[line.kind]),
    h("span", { className: "dl-text" }, line.text),
  );
}

export function renderLines(lines: DiffLine[]): ReactNode {
  return lines.map(renderLine);
}

/** Why a file's lines are not shown, in one line. */
export function omittedText(why: Omitted, file: FileChange): string {
  switch (why) {
    case "binary":
      return "Binary file: not shown.";
    case "large":
      return `${plural(file.added + file.removed, "changed line")}: too large to show.`;
    case "budget":
      return "Not shown: the change is too large to show in full.";
    case "unreadable":
      return "Not shown: git's diff for it could not be read.";
  }
}
