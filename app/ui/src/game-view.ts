// The game view's pure parts: which keys reach the game, where the frame sits
// in the view, and where a pointer lands in the game.

export type Size = { w: number; h: number };
export type Rect = { x: number; y: number; w: number; h: number };

/** `KeyboardEvent.code` → the `/input` wire's key name. Escape is absent: it releases input. */
const KEYS: Record<string, string> = {
  ArrowUp: "up",
  ArrowDown: "down",
  ArrowLeft: "left",
  ArrowRight: "right",
  Space: "space",
  Enter: "enter",
};
for (let i = 0; i < 26; i++) {
  const letter = String.fromCharCode(97 + i);
  KEYS[`Key${letter.toUpperCase()}`] = letter;
}
// The native window folds the numpad into the digit row too.
for (let d = 0; d <= 9; d++) {
  KEYS[`Digit${d}`] = String(d);
  KEYS[`Numpad${d}`] = String(d);
}

/** The wire name for `code`, or null for a key the wire does not know (it is ignored). */
export function wireKey(code: string): string | null {
  return Object.hasOwn(KEYS, code) ? KEYS[code]! : null;
}

/**
 * `frame`'s rect centred in `box`, letterboxed to its aspect ratio and no
 * larger than `max` (the frame's own pixels in CSS pixels), so it is never
 * scaled up.
 */
export function fitRect(box: Size, frame: Size, max: Size): Rect {
  const scale = Math.min(box.w / frame.w, box.h / frame.h, max.w / frame.w, max.h / frame.h);
  const w = frame.w * scale;
  const h = frame.h * scale;
  return { x: (box.w - w) / 2, y: (box.h - h) / 2, w, h };
}

/** A point in the view → the game's pointer surface (`0..w-1`), clamped to the frame. */
export function toSurface(x: number, y: number, rect: Rect, surface: Size): { x: number; y: number } {
  const to = (v: number, size: number) => Math.min(Math.max(Math.round(v * size), 0), size - 1);
  return { x: to((x - rect.x) / rect.w, surface.w), y: to((y - rect.y) / rect.h, surface.h) };
}

const LOOPBACK = new Set(["127.0.0.1", "localhost", "[::1]"]);

/** The session's `/stream` for frames that fit `size` device pixels; null off loopback. */
export function streamUrl(debugUrl: string, size: Size): string | null {
  const url = new URL("/stream", debugUrl);
  if (url.protocol !== "http:" || !LOOPBACK.has(url.hostname)) return null;
  url.search = `w=${Math.max(1, Math.round(size.w))}&h=${Math.max(1, Math.round(size.h))}`;
  return url.href;
}
