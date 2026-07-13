import test from "node:test";
import assert from "node:assert/strict";
import {
  createTimelineState,
  deriveTimelineView,
  normalizePreviewConfig,
  reduceTimeline,
} from "./timeline-model.js";

const publish = (state, frame, hi, paused, lo = 0) =>
  reduceTimeline(state, {
    type: "runtime-published",
    snapshot: { frame, lo, hi, paused },
  });

test("live playback keeps the selected frame at the recorded endpoint", () => {
  let state = publish(createTimelineState(), 10, 10, false);
  state = publish(state, 11, 11, false);
  const view = deriveTimelineView(state);
  assert.equal(view.selectedFrame, 11);
  assert.equal(view.viewport.hi, 11);
  assert.equal(view.playheadUnit, 1);
});

test("pausing captures a viewport that preview changes cannot resize", () => {
  let state = publish(createTimelineState(), 300, 300, false);
  state = publish(state, 300, 300, true);
  state = reduceTimeline(state, {
    type: "preview-changed",
    preview: { enabled: true, seconds: 5 },
  });
  state = publish(state, 301, 301, true);
  assert.deepEqual(deriveTimelineView(state).viewport, { lo: 0, hi: 300 });
});

test("seeking changes selection without changing the paused viewport", () => {
  let state = publish(createTimelineState({ enabled: true, seconds: 2 }), 300, 300, true);
  state = reduceTimeline(state, { type: "seek-requested", frame: 250 });
  const view = deriveTimelineView(state);
  assert.equal(view.selectedFrame, 250);
  assert.deepEqual(view.viewport, { lo: 0, hi: 300 });
});

test("the logical preview remains full length while its rendering clips", () => {
  let state = publish(createTimelineState({ enabled: true, seconds: 2 }), 300, 300, true);
  state = reduceTimeline(state, { type: "seek-requested", frame: 250 });
  const view = deriveTimelineView(state);
  assert.equal(view.previewFrames, 120);
  assert.equal(view.previewEndFrame, 370);
  assert.equal(view.visiblePreviewEndFrame, 300);
  assert.equal(view.previewClippedFrames, 70);
});

test("an optimistic seek is held until the runtime acknowledges it", () => {
  let state = publish(createTimelineState(), 100, 200, true);
  state = reduceTimeline(state, { type: "seek-requested", frame: 150 });
  state = publish(state, 100, 200, true);
  assert.equal(deriveTimelineView(state).selectedFrame, 150);
  state = publish(state, 150, 200, true);
  assert.equal(state.requestedFrame, null);
  assert.equal(deriveTimelineView(state).selectedFrame, 150);
});

test("invalid preview edits retain the previous canonical configuration", () => {
  const previous = { enabled: true, seconds: 2, rate: 5 };
  assert.deepEqual(normalizePreviewConfig({ seconds: NaN, rate: NaN }, previous), previous);
  assert.deepEqual(normalizePreviewConfig({ seconds: 99, rate: -4 }, previous), {
    enabled: true,
    seconds: 5,
    rate: 1,
  });
});

test("resuming returns the viewport to the live recorded extent", () => {
  let state = publish(createTimelineState(), 200, 200, true);
  state = publish(state, 200, 240, true);
  assert.equal(deriveTimelineView(state).viewport.hi, 200);
  state = publish(state, 241, 241, false);
  assert.equal(deriveTimelineView(state).viewport.hi, 241);
});
