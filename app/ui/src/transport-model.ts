// The transport bar's pure parts. The timeline itself (viewport, playhead,
// optimistic seeks, mark clustering, keys, ticks, labels) is the web
// scrubber's own model, imported as is; this file only feeds it the core's
// transport.

import {
  deriveTimelineView,
  reduceTimeline,
  reloadPeeks,
  seekKeyTarget,
} from "../../../runtime/functor-runtime-web/timeline-model.js";
import type { Mark, Transport } from "./generated/protocol";

export {
  EXTRAPOLATE_BUTTON,
  PEEK_HOLD_MS,
  STEP_BUTTON,
  TIMELINE_COLORS,
  createTimelineState,
  deriveTimelineView,
  frameToUnit,
  markerDetail,
  markerLabel,
  pauseButton,
  reduceTimeline,
  timelineTicks,
  unitToFrame,
} from "../../../runtime/functor-runtime-web/timeline-model.js";

export type TimelineState = ReturnType<typeof reduceTimeline>;
export type TimelineView = NonNullable<ReturnType<typeof deriveTimelineView>>;

/**
 * The timeline after the core's `transport`: its clock as the runtime
 * snapshot the model takes, and this page's seek resolved once the core
 * echoes its id (it may have clamped it). The Extrapolate switch is the
 * model's preview, so the rail shows its window.
 */
export function follow(state: TimelineState, transport: Transport): TimelineState {
  let next = state;
  if (next.preview.enabled !== transport.extrapolate) {
    next = reduceTimeline(next, { type: "preview-changed", preview: { enabled: transport.extrapolate } });
  }
  const [lo, hi] = transport.recorded ?? [null, null];
  if (lo !== null && hi !== null) {
    const snapshot = { frame: transport.frame, lo, hi, paused: transport.paused, generation: transport.generation };
    // A poll sees a pause a few frames after the last one it saw running:
    // the window freezes where the runtime stopped, not at that older poll.
    if (snapshot.paused && next.runtime && !next.runtime.paused) {
      next = reduceTimeline(next, { type: "runtime-published", snapshot: { ...snapshot, paused: false } });
    }
    next = reduceTimeline(next, { type: "runtime-published", snapshot });
  } else if (next.runtime) {
    next = reduceTimeline(next, { type: "recording-cleared" });
  }
  if (next.requestedSeekId !== null && transport.seek === next.requestedSeekId) {
    next = reduceTimeline(next, { type: "seek-resolved", id: transport.seek, frame: transport.frame });
  }
  return next;
}

/**
 * Whether `transport` lands an edit the page should peek at (the web
 * scrubber's rule): a hot reload since revision `seen`, paused, with the
 * switch off. A revision counts only successful reloads.
 */
export const editPeeks = (seen: number, transport: Transport): boolean =>
  transport.revision > seen && reloadPeeks("reload-ok", transport.paused, transport.extrapolate);

/** The marks as the model's events: the same `{id, frame, kind, label}`. */
export function followMarks(state: TimelineState, marks: Mark[]): TimelineState {
  return reduceTimeline(state, { type: "events-published", events: marks });
}

export type KeyAction = { seek: number } | { past: number } | { step: true } | null;

/**
 * What a key on the playhead does: the web scrubber's seek, except forward
 * from the window's end. A pause freezes the window, so steps after it are
 * recorded past it: forward walks them (`past`, not clamped to the window),
 * and from the newest recorded frame steps the game.
 */
export function keyAction(view: TimelineView | null, key: string, shiftKey: boolean): KeyAction {
  if (!view) return null;
  if ((key === "ArrowRight" || key === "ArrowUp") && view.selectedFrame >= view.viewport.hi) {
    return view.selectedFrame >= view.recorded.hi ? { step: true } : { past: view.selectedFrame + 1 };
  }
  const frame = seekKeyTarget(view, key, shiftKey);
  return frame === null ? null : { seek: frame };
}
