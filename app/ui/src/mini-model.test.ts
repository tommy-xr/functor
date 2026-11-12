import { test } from "node:test";
import assert from "node:assert/strict";
import { MINI_MARGIN, MINI_MAX, MINI_MIN, miniKey, miniRect, nearestCorner, resizedWidth } from "./mini-model.ts";

const canvas = { w: 1000, h: 700 };

test("the mini view sits in its corner at its width, the game's aspect below the strip", () => {
  const r = miniRect({ corner: "bottom_right", width: 320 }, 16 / 9, canvas, 40);
  assert.deepEqual(r, { x: 1000 - MINI_MARGIN - 320, y: 700 - MINI_MARGIN - 220, w: 320, h: 220 });
  const tl = miniRect({ corner: "top_left", width: 320 }, 16 / 9, canvas, 40);
  assert.deepEqual([tl.x, tl.y], [MINI_MARGIN, MINI_MARGIN]);
  const bl = miniRect({ corner: "bottom_left", width: 320 }, 4 / 3, canvas, 40);
  assert.deepEqual(bl, { x: MINI_MARGIN, y: 700 - MINI_MARGIN - 280, w: 320, h: 280 });
});

test("the mini view never outgrows the canvas, keeping the aspect", () => {
  // Too wide for a narrow canvas.
  const narrow = miniRect({ corner: "top_right", width: 1200 }, 2, { w: 500, h: 2000 }, 40);
  assert.equal(narrow.w, 500 - 2 * MINI_MARGIN);
  assert.equal(narrow.x, MINI_MARGIN);
  // Too tall for a short one: the height bounds the width.
  const short = miniRect({ corner: "bottom_left", width: 1200 }, 2, { w: 2000, h: 300 }, 40);
  assert.equal(short.h, 300 - 2 * MINI_MARGIN);
  assert.equal(short.w, (300 - 2 * MINI_MARGIN - 40) * 2);
  assert.equal(short.y, MINI_MARGIN);
  // No room at all: nothing, never negative.
  assert.equal(miniRect({ corner: "top_left", width: 300 }, 2, { w: 10, h: 10 }, 40).w, 0);
});

test("a drag snaps to the nearest corner", () => {
  assert.equal(nearestCorner(100, 100, canvas), "top_left");
  assert.equal(nearestCorner(900, 100, canvas), "top_right");
  assert.equal(nearestCorner(100, 600, canvas), "bottom_left");
  assert.equal(nearestCorner(900, 600, canvas), "bottom_right");
});

test("dragging the handle away from the corner grows it, keeping the aspect", () => {
  // Bottom-right: the handle is top-left, so up and left grow it.
  assert.equal(resizedWidth("bottom_right", 300, -50, 0, 2), 350);
  assert.equal(resizedWidth("bottom_right", 300, 0, -50, 2), 400);
  assert.equal(resizedWidth("bottom_right", 400, 30, 0, 2), 370);
  // Top-left: down and right grow it; the larger move wins.
  assert.equal(resizedWidth("top_left", 300, 10, 40, 2), 380);
  assert.equal(resizedWidth("top_left", 300, 100, 10, 2), 400);
  // Within the core's bounds.
  assert.equal(resizedWidth("top_left", 300, -500, 0, 2), MINI_MIN);
  assert.equal(resizedWidth("top_left", 300, 5000, 0, 2), MINI_MAX);
});

test("arrow keys move between corners and + and − resize", () => {
  const mini = { corner: "bottom_right", width: 360 } as const;
  assert.deepEqual(miniKey(mini, "ArrowLeft"), { corner: "bottom_left", width: 360 });
  assert.deepEqual(miniKey(mini, "ArrowUp"), { corner: "top_right", width: 360 });
  assert.equal(miniKey(mini, "ArrowRight"), null);
  assert.equal(miniKey(mini, "ArrowDown"), null);
  assert.deepEqual(miniKey(mini, "+"), { corner: "bottom_right", width: 400 });
  assert.deepEqual(miniKey(mini, "-"), { corner: "bottom_right", width: 320 });
  assert.equal(miniKey({ ...mini, width: MINI_MIN }, "-"), null);
  assert.equal(miniKey(mini, "a"), null);
});
