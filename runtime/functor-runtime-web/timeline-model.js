// Pure state and geometry for the web time-travel timeline. The DOM component
// dispatches semantic actions and renders the derived view; it never owns the
// timeline policy itself. Keeping this file browser-free makes the pause/seek /
// extrapolation invariants cheap to test under Node.

export const TIMELINE_FPS = 60;
export const PREVIEW_SECONDS_MIN = 0.5;
export const PREVIEW_SECONDS_MAX = 5;
export const PREVIEW_RATE_MIN = 1;
export const PREVIEW_RATE_MAX = 30;

const clamp = (value, lo, hi) => Math.min(hi, Math.max(lo, value));
const finiteOr = (value, fallback) =>
  typeof value === "number" && Number.isFinite(value) ? value : fallback;

export function normalizePreviewConfig(next = {}, previous = { seconds: 2, rate: 5 }) {
  return {
    enabled: next.enabled ?? previous.enabled ?? false,
    seconds: clamp(
      finiteOr(next.seconds, previous.seconds),
      PREVIEW_SECONDS_MIN,
      PREVIEW_SECONDS_MAX
    ),
    rate: Math.round(
      clamp(finiteOr(next.rate, previous.rate), PREVIEW_RATE_MIN, PREVIEW_RATE_MAX)
    ),
  };
}

export function createTimelineState(preview = {}) {
  return {
    runtime: null,
    viewport: null,
    selectedFrame: null,
    requestedFrame: null,
    followLive: true,
    preview: normalizePreviewConfig(preview),
    events: [],
  };
}

const validSnapshot = (snapshot) =>
  snapshot &&
  Number.isFinite(snapshot.frame) &&
  Number.isFinite(snapshot.lo) &&
  Number.isFinite(snapshot.hi) &&
  snapshot.hi >= snapshot.lo;

const rangesOverlap = (a, b) => a && b && a.hi >= b.lo && b.hi >= a.lo;

export function reduceTimeline(state, action) {
  switch (action.type) {
    case "runtime-published": {
      const snapshot = action.snapshot;
      if (!validSnapshot(snapshot)) return state;

      const previousPaused = state.runtime?.paused ?? false;
      const recorded = { lo: snapshot.lo, hi: snapshot.hi };
      let viewport = state.viewport;

      // A reload resets the seekable model-history window around a new monotonic
      // frame. If the old viewport no longer intersects it, the new recording is
      // the only honest domain.
      if (!rangesOverlap(viewport, recorded)) viewport = recorded;

      if (snapshot.paused) {
        // Pause captures the viewport exactly once. Subsequent history/config /
        // selection updates cannot resize it.
        if (!previousPaused || !viewport) viewport = recorded;
      } else if (state.followLive || !viewport) {
        viewport = recorded;
      }

      const acknowledged =
        state.requestedFrame !== null && snapshot.frame === state.requestedFrame;
      const requestedFrame = acknowledged ? null : state.requestedFrame;
      const selectedFrame = requestedFrame ?? snapshot.frame;

      return {
        ...state,
        runtime: snapshot,
        viewport,
        requestedFrame,
        selectedFrame,
      };
    }

    case "seek-requested": {
      if (!state.viewport || !Number.isFinite(action.frame)) return state;
      const frame = Math.round(clamp(action.frame, state.viewport.lo, state.viewport.hi));
      return { ...state, selectedFrame: frame, requestedFrame: frame };
    }

    case "preview-changed":
      return {
        ...state,
        preview: normalizePreviewConfig(action.preview, state.preview),
      };

    case "events-published":
      return {
        ...state,
        events: Array.isArray(action.events) ? action.events : state.events,
      };

    case "preview-end-requested": {
      if (state.selectedFrame === null || !Number.isFinite(action.frame)) return state;
      const seconds = (action.frame - state.selectedFrame) / TIMELINE_FPS;
      return {
        ...state,
        preview: normalizePreviewConfig({ seconds }, state.preview),
      };
    }

    case "preview-delta-requested": {
      if (!Number.isFinite(action.frames)) return state;
      const seconds = state.preview.seconds + action.frames / TIMELINE_FPS;
      return {
        ...state,
        preview: normalizePreviewConfig({ seconds }, state.preview),
      };
    }

    case "follow-live-changed":
      return { ...state, followLive: Boolean(action.enabled) };

    case "viewport-changed": {
      if (!validSnapshot({ frame: action.lo, lo: action.lo, hi: action.hi })) return state;
      return {
        ...state,
        viewport: { lo: action.lo, hi: action.hi },
        followLive: false,
      };
    }

    default:
      return state;
  }
}

export function frameToUnit(frame, viewport) {
  if (!viewport || viewport.hi <= viewport.lo) return 0;
  return clamp((frame - viewport.lo) / (viewport.hi - viewport.lo), 0, 1);
}

export function unitToFrame(unit, viewport) {
  if (!viewport) return 0;
  return Math.round(viewport.lo + clamp(unit, 0, 1) * (viewport.hi - viewport.lo));
}

export function deriveTimelineView(state) {
  if (!state.runtime || !state.viewport || state.selectedFrame === null) return null;

  const selectedFrame = Math.round(
    clamp(state.selectedFrame, state.viewport.lo, state.viewport.hi)
  );
  const previewFrames = state.preview.enabled
    ? Math.round(state.preview.seconds * TIMELINE_FPS)
    : 0;
  const previewEndFrame = selectedFrame + previewFrames;
  const visiblePreviewEndFrame = Math.min(previewEndFrame, state.viewport.hi);
  const eventMarkers = clusterTimelineEvents(state.events, state.viewport);

  return {
    viewport: state.viewport,
    recorded: { lo: state.runtime.lo, hi: state.runtime.hi },
    paused: state.runtime.paused,
    selectedFrame,
    requestedFrame: state.requestedFrame,
    playheadUnit: frameToUnit(selectedFrame, state.viewport),
    previewFrames,
    previewEndFrame,
    visiblePreviewEndFrame,
    previewEndUnit: frameToUnit(visiblePreviewEndFrame, state.viewport),
    previewClippedFrames: Math.max(previewEndFrame - state.viewport.hi, 0),
    eventMarkers,
  };
}

export function clusterTimelineEvents(events, viewport, bucketCount = 250) {
  if (!viewport || viewport.hi < viewport.lo) return [];
  const buckets = new Map();
  for (const event of events) {
    if (!event || !Number.isFinite(event.frame)) continue;
    if (event.frame < viewport.lo || event.frame > viewport.hi) continue;
    const category = String(event.kind).startsWith("reload") ? "reload" : "input";
    const unit = frameToUnit(event.frame, viewport);
    const bucket = Math.round(unit * bucketCount);
    const key = `${category}:${bucket}`;
    const existing = buckets.get(key);
    if (existing) {
      existing.count += 1;
      existing.lastFrame = event.frame;
      existing.labels.push(event.label);
    } else {
      buckets.set(key, {
        id: event.id,
        frame: event.frame,
        lastFrame: event.frame,
        kind: event.kind,
        category,
        count: 1,
        labels: [event.label],
        unit,
      });
    }
  }
  return [...buckets.values()].sort((a, b) => a.frame - b.frame);
}
