import test from "node:test";
import assert from "node:assert/strict";
import {
  clusterTimelineEvents,
  createTimelineState,
  describeRecordedAvailability,
  deriveTimelineView,
  normalizePreviewConfig,
  reduceTimeline,
} from "./timeline-model.js";

const publish = (state, frame, hi, paused, lo = 0, generation = 0) =>
  reduceTimeline(state, {
    type: "runtime-published",
    snapshot: { frame, lo, hi, paused, generation },
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
  state = reduceTimeline(state, { type: "seek-requested", id: 1, frame: 150 });
  state = publish(state, 100, 200, true);
  assert.equal(deriveTimelineView(state).selectedFrame, 150);
  state = publish(state, 150, 200, true);
  assert.equal(state.requestedFrame, null);
  assert.equal(state.requestedSeekId, null);
  assert.equal(deriveTimelineView(state).selectedFrame, 150);
});

test("a resolved seek reconciles a clamped or refused optimistic target", () => {
  let state = publish(createTimelineState(), 100, 200, true);
  state = reduceTimeline(state, { type: "seek-requested", id: 7, frame: 150 });
  state = reduceTimeline(state, { type: "seek-resolved", id: 6, frame: 100 });
  assert.equal(deriveTimelineView(state).selectedFrame, 150, "ignore an older result");
  state = reduceTimeline(state, { type: "seek-resolved", id: 7, frame: 100 });
  assert.equal(state.requestedFrame, null);
  assert.equal(deriveTimelineView(state).selectedFrame, 100);
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

test("timeline events are filtered and clustered within the viewport", () => {
  const markers = clusterTimelineEvents(
    [
      { id: 1, frame: 9, kind: "key-down", label: "Space down" },
      { id: 2, frame: 20, kind: "key-down", label: "W down" },
      { id: 3, frame: 20, kind: "key-up", label: "W up" },
      { id: 4, frame: 80, kind: "reload-ok", label: "hot reload" },
    ],
    { lo: 10, hi: 100 }
  );
  assert.equal(markers.length, 2);
  assert.equal(markers[0].category, "input");
  assert.equal(markers[0].count, 2);
  assert.equal(markers[1].category, "reload");
});

test("stepping past a frozen paused endpoint preserves the logical selected frame", () => {
  let state = publish(createTimelineState(), 300, 300, false);
  state = publish(state, 300, 300, true);
  state = publish(state, 301, 301, true);
  const view = deriveTimelineView(state);
  assert.deepEqual(view.viewport, { lo: 0, hi: 300 });
  assert.equal(view.selectedFrame, 301);
  assert.equal(view.playheadUnit, 1);
  assert.equal(view.playheadClippedAfter, true);
});

test("a shortened branch paints only the still-recorded part of a frozen viewport", () => {
  let state = publish(createTimelineState(), 300, 300, true);
  state = reduceTimeline(state, { type: "seek-requested", id: 1, frame: 150 });
  state = reduceTimeline(state, { type: "seek-resolved", id: 1, frame: 150 });
  state = publish(state, 151, 151, true);
  const view = deriveTimelineView(state);
  assert.deepEqual(view.viewport, { lo: 0, hi: 300 });
  assert.equal(view.recordedStartUnit, 0);
  assert.equal(view.recordedEndUnit, 151 / 300);
});

test("clearing recording keeps the frozen transport view but disables seeking", () => {
  let state = publish(createTimelineState(), 300, 300, true);
  state = reduceTimeline(state, { type: "recording-cleared" });
  state = reduceTimeline(state, { type: "seek-requested", id: 1, frame: 100 });
  const view = deriveTimelineView(state);
  assert.equal(view.recordingAvailable, false);
  assert.equal(view.recordedStartUnit, 0);
  assert.equal(view.recordedEndUnit, 0);
  assert.equal(view.unavailableEndUnit, 1);
  assert.equal(view.hasUnavailableHistory, true);
  assert.equal(state.requestedFrame, null);
  assert.equal(
    describeRecordedAvailability(view),
    "no recorded history is currently available"
  );
});

test("a paused reload keeps its frame and viewport while marking old history unavailable", () => {
  let state = publish(createTimelineState({ enabled: true }), 300, 300, true);
  state = publish(state, 300, 300, true, 300, 1);
  const view = deriveTimelineView(state);
  assert.equal(view.selectedFrame, 300);
  assert.deepEqual(view.viewport, { lo: 0, hi: 300 });
  assert.equal(view.recordedStartUnit, 1);
  assert.equal(view.recordedEndUnit, 1);
  assert.equal(view.unavailableEndUnit, 1);
  assert.equal(view.unavailableAfterStartUnit, 1);
  assert.equal(view.hasUnavailableHistory, true);
  assert.equal(view.previewFrames, 120);
});

test("an unsafe reload in the middle stripes both discarded sides", () => {
  let state = publish(createTimelineState(), 300, 300, true);
  state = reduceTimeline(state, { type: "seek-requested", id: 1, frame: 120 });
  state = reduceTimeline(state, { type: "seek-resolved", id: 1, frame: 120 });
  state = publish(state, 120, 120, true, 120, 1);

  const view = deriveTimelineView(state);
  assert.deepEqual(view.viewport, { lo: 0, hi: 300 });
  assert.equal(view.recordedStartUnit, 0.4);
  assert.equal(view.recordedEndUnit, 0.4);
  assert.equal(view.unavailableEndUnit, 0.4);
  assert.equal(view.unavailableAfterStartUnit, 0.4);
  assert.equal(view.hasUnavailableHistory, true);
  assert.equal(
    describeRecordedAvailability(view),
    "recorded frames 120 to 120; striped history outside that range is unavailable"
  );
});

test("a resumed middle branch keeps the old total until it is refilled", () => {
  let state = publish(createTimelineState(), 75, 200, true);
  state = publish(state, 76, 76, false, 0, 1);

  let view = deriveTimelineView(state);
  assert.deepEqual(view.viewport, { lo: 0, hi: 200 });
  assert.equal(view.recordedEndUnit, 76 / 200);
  assert.equal(view.unavailableAfterStartUnit, 76 / 200);
  assert.equal(view.hasUnavailableHistory, true);

  state = publish(state, 200, 200, false, 0, 1);
  view = deriveTimelineView(state);
  assert.deepEqual(view.viewport, { lo: 0, hi: 200 });
  assert.equal(view.hasUnavailableHistory, false);
  assert.equal(state.continuity, null);
});

test("live frames replace a reload stripe without collapsing the visual scale", () => {
  let state = publish(createTimelineState(), 300, 300, false);
  state = publish(state, 300, 300, false, 300, 1);
  assert.deepEqual(deriveTimelineView(state).viewport, { lo: 0, hi: 300 });

  state = publish(state, 301, 301, false, 300, 1);
  let view = deriveTimelineView(state);
  assert.deepEqual(view.viewport, { lo: 1, hi: 301 });
  assert.equal(view.unavailableEndUnit, 299 / 300);
  assert.equal(view.unavailableAfterStartUnit, 1);

  state = publish(state, 600, 600, false, 300, 1);
  view = deriveTimelineView(state);
  assert.deepEqual(view.viewport, { lo: 300, hi: 600 });
  assert.equal(view.hasUnavailableHistory, false);
  assert.equal(state.continuity, null);
});

test("a safe reload generation leaves the full seekable history unchanged", () => {
  let state = publish(createTimelineState(), 300, 300, true, 0, 4);
  state = publish(state, 300, 300, true, 0, 4);
  const view = deriveTimelineView(state);
  assert.deepEqual(view.viewport, { lo: 0, hi: 300 });
  assert.equal(view.recordedStartUnit, 0);
  assert.equal(view.recordedEndUnit, 1);
  assert.equal(view.hasUnavailableHistory, false);
});

test("hover takes precedence over persistent marker selection", () => {
  let state = publish(createTimelineState(), 100, 100, true);
  state = reduceTimeline(state, {
    type: "events-published",
    events: [
      { id: 1, frame: 20, kind: "key-down", label: "Space down" },
      { id: 2, frame: 80, kind: "reload-ok", label: "hot reload" },
    ],
  });
  state = reduceTimeline(state, { type: "event-selected", id: 1 });
  assert.equal(deriveTimelineView(state).activeEvent.id, 1);
  state = reduceTimeline(state, { type: "event-hovered", id: 2 });
  assert.equal(deriveTimelineView(state).activeEvent.id, 2);
  state = reduceTimeline(state, { type: "event-hovered", id: null });
  assert.equal(deriveTimelineView(state).activeEvent.id, 1);
});
