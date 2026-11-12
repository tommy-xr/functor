// The task page's pure parts: its sections assembled from the transcript —
// what the agent was given, each run's steps, the walkthrough, and the
// usage table. No React, so `node --test` runs them.

import type { Item, WalkthroughStep } from "./generated/protocol";
import { outcome, stepPhrase, type Outcome, type Phrase } from "./chat-model.ts";

/** `pending`: its run is still going; `unanswered`: the run ended first. */
export type StepState = "ok" | "failed" | "pending" | "unanswered";
export type Step = { phrase: Phrase; state: StepState };

/** One run: the steps after its user message, and how it ended (`null` while it runs). */
export type RunSteps = { n: number; steps: Step[]; outcome: Outcome | null };

export type TaskPage = {
  /** The first user message, in full. */
  given: string | null;
  /** The user's later messages ("Then you said"). */
  followUps: string[];
  runs: RunSteps[];
  /** The last walkthrough, with its transcript index (what a step click names). */
  walkthrough: { item: number; steps: WalkthroughStep[]; current: number | null } | null;
};

/** A tool's answer that reports a failure: it starts with "error". */
const FAILED = /^\s*error\b/i;

/** The page's sections from `items`; `running` is the in-flight run's first index (`Run::id`), if any. */
export function taskPage(items: readonly Item[], running: number | null): TaskPage {
  const page: TaskPage = { given: null, followUps: [], runs: [], walkthrough: null };
  let run: RunSteps | null = null;
  let live = false;
  items.forEach((item, i) => {
    switch (item.kind) {
      case "user":
        if (page.given === null) page.given = item.text;
        else page.followUps.push(item.text);
        run = { n: page.runs.length + 1, steps: [], outcome: null };
        live = i === running;
        page.runs.push(run);
        break;
      case "tool": {
        const state: StepState =
          item.result === null ? (live ? "pending" : "unanswered") : FAILED.test(item.result) ? "failed" : "ok";
        run?.steps.push({ phrase: stepPhrase(item.name, item.input), state });
        break;
      }
      case "finished":
      case "stopped":
        if (run) run.outcome = outcome(item);
        break;
      case "walkthrough":
        page.walkthrough = { item: i, steps: item.steps, current: item.current };
        break;
    }
  });
  return page;
}

export type UsageRow = { run: number; input: number | null; output: number | null };
export type UsageTable = { rows: UsageRow[]; total: { turns: number; input: number; output: number } };

/**
 * Tokens per run (each user message starts one; `null` before its usage
 * arrives) and in all: the seed for cost metrics.
 */
export function usageTable(items: readonly Item[]): UsageTable {
  const rows: UsageRow[] = [];
  const total = { turns: 0, input: 0, output: 0 };
  for (const item of items) {
    if (item.kind === "user") {
      rows.push({ run: rows.length + 1, input: null, output: null });
      total.turns++;
    } else if (item.kind === "usage") {
      const row = rows[rows.length - 1];
      if (!row) continue;
      row.input = (row.input ?? 0) + item.input;
      row.output = (row.output ?? 0) + item.output;
      total.input += item.input;
      total.output += item.output;
    }
  }
  return { rows, total };
}
