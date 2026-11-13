// The separate review (D31): its setting per project, and what the decision
// page says about it. No React, so `node --test` runs them.

import type { AppState, Harness, ReviewSetting, Task } from "./generated/protocol";

/** A project's setting when none is saved (the core's `ReviewSetting::default`). */
export const DEFAULT_REVIEW: ReviewSetting = { on: true, harness: null, model: null, instructions: null };

/** The model a review runs when the setting names none (the core's `review::default_model`). */
export const DEFAULT_MODEL: Record<Harness, string | null> = { claude: "sonnet", codex: null };

/** The active project's setting. */
export function reviewSetting(state: AppState | null): ReviewSetting {
  return (state?.active && state.review_settings[state.active]) || DEFAULT_REVIEW;
}

/** The model field's hint: what runs when it is left empty. */
export function modelHint(setting: ReviewSetting, taskHarness: Harness): string {
  const harness = setting.harness ?? taskHarness;
  const model = DEFAULT_MODEL[harness];
  return model ? `${model} (default for ${harness})` : `${harness}'s default`;
}

/** "41k" from 41_234. */
const kilo = (n: number) => (n < 1000 ? String(n) : `${Math.round(n / 1000)}k`);

/** What the first page says about the review: who prepared the report, that one is being prepared, or why it was not. */
export type ReviewLine = { kind: "by" | "preparing" | "failed"; text: string };

export function reviewLines(task: Task): ReviewLine[] {
  const review = task.review;
  const lines: ReviewLine[] = [];
  if (task.report?.reviewed && review) {
    const who = [review.harness, review.model].filter(Boolean).join(" · ");
    const tokens = review.usage ? ` · ${kilo(review.usage.input)} tokens in` : "";
    lines.push({ kind: "by", text: `Prepared by a separate review (${who}${tokens})` });
  }
  if (review?.state.kind === "preparing") lines.push({ kind: "preparing", text: "Preparing the summary…" });
  if (review?.state.kind === "failed") {
    lines.push({ kind: "failed", text: `The separate review did not finish: ${review.state.reason}.` });
  }
  return lines;
}
