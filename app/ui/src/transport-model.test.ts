import { test } from "node:test";
import assert from "node:assert/strict";
import { createTimelineState, deriveTimelineView, follow, followMarks, keyAction, reduceTimeline } from "./transport-model.ts";
import type { Transport } from "./generated/protocol";

const at = (frame: number, over: Partial<Transport> = {}): Transport => ({
  clock: true,
  paused: false,
  frame,
  recorded: [1, frame],
  generation: 0,
  seek: 0,
  ...over,
});

test("the core's clock drives the web timeline's playhead and window", () => {
  let state = follow(createTimelineState(), at(30));
  state = follow(state, at(31));
  const view = deriveTimelineView(state)!;
  assert.equal(view.selectedFrame, 31);
  assert.deepEqual(view.viewport, { lo: 1, hi: 31 });
  assert.equal(view.paused, false);
  // Paused, the window freezes while a step moves the playhead past it.
  state = follow(state, at(31, { paused: true }));
  state = follow(state, at(32, { paused: true }));
  assert.deepEqual(deriveTimelineView(state)!.viewport, { lo: 1, hi: 31 });
  assert.equal(deriveTimelineView(state)!.selectedFrame, 32);
});

test("a pause seen a few frames late freezes the window where the runtime stopped", () => {
  let state = follow(createTimelineState(), at(90));
  state = follow(state, at(91, { paused: true }));
  const view = deriveTimelineView(state)!;
  assert.deepEqual(view.viewport, { lo: 1, hi: 91 });
  assert.equal(view.playheadClippedAfter, false);
});

test("a seek shows at once and resolves when the core echoes its id", () => {
  let state = follow(createTimelineState(), at(100, { paused: true }));
  state = reduceTimeline(state, { type: "seek-requested", id: 4, frame: 40 });
  // An older clock does not move the playhead back.
  state = follow(state, at(100, { paused: true }));
  assert.equal(deriveTimelineView(state)!.selectedFrame, 40);
  // The core clamped it to 41 and says so with the page's id.
  state = follow(state, at(100, { paused: true, frame: 41, recorded: [41, 100], seek: 4 }));
  assert.equal(state.requestedSeekId, null);
  assert.equal(deriveTimelineView(state)!.selectedFrame, 41);
});

test("nothing recorded leaves no seekable rail", () => {
  let state = follow(createTimelineState(), at(0, { recorded: null }));
  assert.equal(deriveTimelineView(state), null);
  state = follow(state, at(5));
  state = follow(state, at(5, { recorded: null, paused: true }));
  assert.equal(state.recordingAvailable, false);
});

test("marks are the timeline's events, clustered in its window", () => {
  let state = follow(createTimelineState(), at(100));
  state = followMarks(state, [
    { id: 1, frame: 10, kind: "key-down", label: "W down" },
    { id: 2, frame: 10, kind: "key-up", label: "W up" },
    { id: 3, frame: 60, kind: "reload-ok", label: "hot reload" },
  ]);
  const markers = deriveTimelineView(state)!.eventMarkers;
  assert.deepEqual(
    markers.map((m) => [m.frame, m.category, m.count]),
    [
      [10, "input", 2],
      [60, "reload", 1],
    ],
  );
});

test("playhead keys seek as the web's do, and forward from the newest frame steps", () => {
  const view = deriveTimelineView(follow(createTimelineState(), at(50, { paused: true })))!;
  assert.deepEqual(keyAction(view, "ArrowLeft", false), { seek: 49 });
  assert.deepEqual(keyAction(view, "Home", false), { seek: 1 });
  assert.deepEqual(keyAction(view, "ArrowRight", false), { step: true });
  assert.equal(keyAction(view, "x", false), null);
  // A pause froze the window at 50 and steps recorded 51–52: forward walks
  // them without branching, then steps on from the newest.
  let stepped = follow(createTimelineState(), at(50));
  stepped = follow(stepped, at(50, { paused: true }));
  stepped = follow(stepped, at(50, { paused: true, recorded: [1, 52] }));
  assert.deepEqual(keyAction(deriveTimelineView(stepped), "ArrowRight", false), { past: 51 });
  stepped = follow(stepped, at(52, { paused: true, recorded: [1, 52] }));
  assert.deepEqual(keyAction(deriveTimelineView(stepped), "ArrowRight", false), { step: true });
  const earlier = deriveTimelineView(reduceTimeline(follow(createTimelineState(), at(50, { paused: true })), { type: "seek-requested", id: 1, frame: 20 }))!;
  assert.deepEqual(keyAction(earlier, "ArrowRight", true), { seek: 30 });
});
