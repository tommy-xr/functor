// The separate review (D31): its setting per project, and what the decision
// page says about it. No React, so `node --test` runs them.

import type { AppState, ReviewSetting, Task } from "./generated/protocol";
import { tokens } from "./chat-model.ts";

/** A project's setting when none is saved (the core's `ReviewSetting::default`). */
export const DEFAULT_REVIEW: ReviewSetting = { on: true, harness: null, model: null, instructions: null };

/** The model a review runs when the setting names none (the core's `review::default_model`; reviews run on Claude). */
export const DEFAULT_MODEL = "sonnet";

/** The active project's setting. */
export function reviewSetting(state: AppState | null): ReviewSetting {
  return (state?.active && state.review_settings[state.active]) || DEFAULT_REVIEW;
}

/** The model field's hint: what runs when it is left empty. */
export const modelHint = () => `${DEFAULT_MODEL} (default)`;

/** What the first page says about the review: who prepared the report, that one is being prepared, or why it was not. */
export type ReviewLine = { kind: "by" | "preparing" | "failed"; text: string };

export function reviewLines(task: Task): ReviewLine[] {
  const review = task.review;
  const lines: ReviewLine[] = [];
  if (task.report?.reviewed && review) {
    const who = [review.harness, review.model].filter(Boolean).join(" · ");
    const used = review.usage ? ` · ${tokens(review.usage.input)} tokens in` : "";
    lines.push({ kind: "by", text: `Prepared by a separate review (${who}${used})` });
  }
  if (review?.state.kind === "preparing") lines.push({ kind: "preparing", text: "Preparing the summary…" });
  if (review?.state.kind === "failed") {
    lines.push({ kind: "failed", text: `The separate review did not finish: ${review.state.reason}.` });
  }
  return lines;
}
