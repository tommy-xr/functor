// The mini game view's pure parts: where it sits in the canvas, how a corner
// drag resizes it, and which corner a drag or an arrow key snaps it to. Its
// corner and width are the core's (`Layout.game.mini`); the page only fits
// them to the canvas it has.

import type { Corner, Mini } from "./generated/protocol";
import type { Rect, Size } from "./game-view";

/** The core's bounds on the width (`MINI_MIN`, `MINI_MAX`). */
export const MINI_MIN = 280;
export const MINI_MAX = 1200;
/** Space kept between the mini view and the canvas's edges. */
export const MINI_MARGIN = 12;
/** One keyboard resize step, in CSS pixels. */
export const MINI_STEP = 40;
/** The game's aspect ratio (w/h) until the session reports its surface. */
export const DEFAULT_ASPECT = 16 / 9;

export const isLeft = (c: Corner) => c === "top_left" || c === "bottom_left";
export const isTop = (c: Corner) => c === "top_left" || c === "top_right";
export const cornerOf = (left: boolean, top: boolean): Corner =>
  `${top ? "top" : "bottom"}_${left ? "left" : "right"}` as Corner;

/**
 * The mini view's box in a canvas of `canvas`: `mini.width` wide, a `strip`
 * above the game at `aspect`, shrunk (keeping the aspect) to fit inside the
 * canvas's margins, in `mini.corner`.
 */
export function miniRect(mini: Mini, aspect: number, canvas: Size, strip: number): Rect {
  const room = { w: Math.max(0, canvas.w - 2 * MINI_MARGIN), h: Math.max(0, canvas.h - 2 * MINI_MARGIN) };
  const w = Math.max(0, Math.min(mini.width, room.w, (room.h - strip) * aspect));
  const h = w / aspect + strip;
  const x = isLeft(mini.corner) ? MINI_MARGIN : canvas.w - MINI_MARGIN - w;
  const y = isTop(mini.corner) ? MINI_MARGIN : canvas.h - MINI_MARGIN - h;
  return { x, y, w, h };
}

/** The canvas corner nearest a box centred at (`x`, `y`). */
export function nearestCorner(x: number, y: number, canvas: Size): Corner {
  return cornerOf(x < canvas.w / 2, y < canvas.h / 2);
}

/**
 * The width after dragging the resize handle (the corner facing the
 * canvas's middle) by (`dx`, `dy`) from `width`: whichever axis moved
 * further sets it, keeping `aspect`; within the core's bounds.
 */
export function resizedWidth(corner: Corner, width: number, dx: number, dy: number, aspect: number): number {
  const byX = isLeft(corner) ? dx : -dx;
  const byY = (isTop(corner) ? dy : -dy) * aspect;
  const grow = Math.abs(byX) >= Math.abs(byY) ? byX : byY;
  return clampWidth(width + grow);
}

export function clampWidth(width: number): number {
  return Math.round(Math.min(Math.max(width, MINI_MIN), MINI_MAX));
}

/**
 * What a key on the mini view's move handle does: an arrow moves it to the
 * corner that way, + and − resize by a step. Null for any other key or no
 * change.
 */
export function miniKey(mini: Mini, key: string): Mini | null {
  const left = isLeft(mini.corner);
  const top = isTop(mini.corner);
  let next: Mini;
  switch (key) {
    case "ArrowLeft":
    case "ArrowRight":
      next = { ...mini, corner: cornerOf(key === "ArrowLeft", top) };
      break;
    case "ArrowUp":
    case "ArrowDown":
      next = { ...mini, corner: cornerOf(left, key === "ArrowUp") };
      break;
    case "+":
    case "=":
      next = { ...mini, width: clampWidth(mini.width + MINI_STEP) };
      break;
    case "-":
    case "_":
      next = { ...mini, width: clampWidth(mini.width - MINI_STEP) };
      break;
    default:
      return null;
  }
  return next.corner === mini.corner && next.width === mini.width ? null : next;
}
