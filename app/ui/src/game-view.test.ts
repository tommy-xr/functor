import { test } from "node:test";
import assert from "node:assert/strict";
import { fitRect, streamUrl, toSurface, wireKey } from "./game-view.ts";

test("keys map to the wire's names", () => {
  assert.equal(wireKey("KeyA"), "a");
  assert.equal(wireKey("KeyZ"), "z");
  assert.equal(wireKey("Digit0"), "0");
  assert.equal(wireKey("Numpad7"), "7");
  assert.equal(wireKey("ArrowUp"), "up");
  assert.equal(wireKey("ArrowRight"), "right");
  assert.equal(wireKey("Space"), "space");
  assert.equal(wireKey("Enter"), "enter");
  assert.equal(wireKey("NumpadEnter"), "enter");
});

test("keys the wire does not know, and Escape, are not forwarded", () => {
  for (const code of ["Escape", "Tab", "ShiftLeft", "F1", "Minus", "", "toString", "constructor"]) {
    assert.equal(wireKey(code), null, code);
  }
});

test("a wider view letterboxes the frame left and right", () => {
  // A 4:3 frame in a 16:9 box, frame larger than the box.
  const rect = fitRect({ w: 1600, h: 900 }, { w: 1600, h: 1200 }, { w: 1600, h: 1200 });
  assert.deepEqual(rect, { x: 200, y: 0, w: 1200, h: 900 });
});

test("a taller view letterboxes the frame top and bottom", () => {
  const rect = fitRect({ w: 800, h: 1000 }, { w: 800, h: 600 }, { w: 800, h: 600 });
  assert.deepEqual(rect, { x: 0, y: 200, w: 800, h: 600 });
});

test("a frame smaller than the view is centred, not scaled up", () => {
  // 1600×1200 device pixels at 2× is 800×600 CSS pixels.
  const rect = fitRect({ w: 1000, h: 800 }, { w: 1600, h: 1200 }, { w: 800, h: 600 });
  assert.deepEqual(rect, { x: 100, y: 100, w: 800, h: 600 });
});

test("pointer positions map into the game's surface through the letterbox", () => {
  const rect = { x: 200, y: 0, w: 1200, h: 900 };
  const surface = { w: 800, h: 600 };
  assert.deepEqual(toSurface(200, 0, rect, surface), { x: 0, y: 0 });
  assert.deepEqual(toSurface(800, 450, rect, surface), { x: 400, y: 300 });
  assert.deepEqual(toSurface(1400, 900, rect, surface), { x: 800, y: 600 });
  assert.deepEqual(toSurface(500, 300, rect, surface), { x: 200, y: 200 });
});

test("pointer positions in the bars clamp to the frame's edge", () => {
  const rect = { x: 200, y: 0, w: 1200, h: 900 };
  const surface = { w: 800, h: 600 };
  assert.deepEqual(toSurface(50, 450, rect, surface), { x: 0, y: 300 });
  assert.deepEqual(toSurface(1550, -10, rect, surface), { x: 800, y: 0 });
});

test("the stream is requested at the view's size, on loopback only", () => {
  assert.equal(
    streamUrl("http://127.0.0.1:52011", { w: 1599.6, h: 900 }, 3),
    "http://127.0.0.1:52011/stream?w=1600&h=900&t=3",
  );
  assert.equal(streamUrl("http://localhost:8077", { w: 0, h: 0 }, 0), "http://localhost:8077/stream?w=1&h=1&t=0");
  assert.equal(streamUrl("http://192.168.1.5:8077", { w: 10, h: 10 }, 0), null);
  assert.equal(streamUrl("https://127.0.0.1:8077", { w: 10, h: 10 }, 0), null);
});
