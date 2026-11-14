// The game view's input rules, pure: which keys reach the game while it has
// input focus, what it holds down (released together when focus leaves), and
// the cursor a pointer lock moves.

import type { GameInput, MouseButton } from "./generated/protocol";

/**
 * `KeyboardEvent.code` (the key's position, whatever the layout) → the
 * runtime `Key` variant it is. Left and right modifiers are one key, and the
 * numpad's digits the digit row's, as in a game's own window.
 */
const VARIANTS: Record<string, string> = {
  ArrowUp: "Up",
  ArrowDown: "Down",
  ArrowLeft: "Left",
  ArrowRight: "Right",
  Space: "Space",
  Enter: "Enter",
  Escape: "Escape",
  Tab: "Tab",
  ShiftLeft: "Shift",
  ShiftRight: "Shift",
  ControlLeft: "Control",
  ControlRight: "Control",
  AltLeft: "Alt",
  AltRight: "Alt",
  Minus: "Minus",
  Equal: "Equal",
  BracketLeft: "LeftBracket",
  BracketRight: "RightBracket",
  Backslash: "Backslash",
  Semicolon: "Semicolon",
  Quote: "Quote",
  Comma: "Comma",
  Period: "Period",
  Slash: "Slash",
};
for (let i = 0; i < 26; i++) {
  const letter = String.fromCharCode(65 + i);
  VARIANTS[`Key${letter}`] = letter;
}
for (let d = 0; d <= 9; d++) {
  VARIANTS[`Digit${d}`] = `Num${d}`;
  VARIANTS[`Numpad${d}`] = `Num${d}`;
}
for (let f = 1; f <= 12; f++) VARIANTS[`F${f}`] = `F${f}`;

/** Every `KeyboardEvent.code` the game hears, with the variant it is (for the drift test). */
export const KEY_VARIANTS: Readonly<Record<string, string>> = VARIANTS;

/** The `/input` wire's name for `variant`: its name in lower case, digits bare (`Num7` → `7`). */
export function wireName(variant: string): string {
  return /^Num\d$/.test(variant) ? variant.slice(3) : variant.toLowerCase();
}

/** The wire name for `code`, or null for a key the game does not hear. */
export function wireKey(code: string): string | null {
  return Object.hasOwn(VARIANTS, code) ? wireName(VARIANTS[code]!) : null;
}

export type KeyLike = { code: string; metaKey: boolean; repeat: boolean };

/**
 * What a key press does while the game has input focus. Every key the game
 * hears goes to it, Tab, Escape and the function keys included. ⌘ is the
 * app's and the OS's: a ⌘ chord never reaches the game, and ⌘Esc releases
 * it — the one way out that always works.
 */
export type KeyAction = "release" | "app" | { game: string } | null;

export function keyAction(e: KeyLike): KeyAction {
  if (e.metaKey && e.code === "Escape") return "release";
  if (e.metaKey || e.code === "MetaLeft" || e.code === "MetaRight") return "app";
  const key = wireKey(e.code);
  return key ? { game: key } : null;
}

/**
 * The keys and buttons the game holds down, so each press gets exactly one
 * release: a repeat or a second press of a held key sends nothing, a release
 * of one never pressed sends nothing, and letting go of focus releases all.
 */
export class Held {
  readonly keys = new Set<string>();
  readonly buttons = new Set<MouseButton>();

  key(key: string, down: boolean): GameInput | null {
    if (down === this.keys.has(key)) return null;
    if (down) this.keys.add(key);
    else this.keys.delete(key);
    return { type: "key", key, down };
  }

  button(button: MouseButton, down: boolean): GameInput | null {
    if (down === this.buttons.has(button)) return null;
    if (down) this.buttons.add(button);
    else this.buttons.delete(button);
    return { type: "mouse_button", button, down };
  }

  /**
   * Releases of every held key. ⌘ going down calls this: macOS sends no
   * key-up for a key let go while ⌘ is held, so it would stick.
   */
  releaseKeys(): GameInput[] {
    const ups: GameInput[] = [...this.keys].map((key) => ({ type: "key", key, down: false }));
    this.keys.clear();
    return ups;
  }

  releaseAll(): GameInput[] {
    const ups = this.releaseKeys();
    for (const button of this.buttons) ups.push({ type: "mouse_button", button, down: false });
    this.buttons.clear();
    return ups;
  }
}

/** What a key-down sends, and whether the page should keep it from the app and browser. */
export function keyDown(held: Held, e: KeyLike): { inputs: GameInput[]; release: boolean; handled: boolean } {
  const action = keyAction(e);
  if (action === "release") return { inputs: held.releaseAll(), release: true, handled: true };
  if (action === "app") {
    const meta = e.code === "MetaLeft" || e.code === "MetaRight";
    return { inputs: meta ? held.releaseKeys() : [], release: false, handled: false };
  }
  if (!action) return { inputs: [], release: false, handled: false };
  // Auto-repeat is not a new press: the game wants edges.
  const press = e.repeat ? null : held.key(action.game, true);
  return { inputs: press ? [press] : [], release: false, handled: true };
}

/**
 * The cursor a pointer lock moves: from where the game view was clicked, by
 * the mouse's deltas in points, unbounded — as the game's own window moves
 * its captured cursor, so a free-look game turns at the same rate.
 */
export class LockedCursor {
  private x: number;
  private y: number;

  constructor(at: { x: number; y: number }) {
    this.x = at.x;
    this.y = at.y;
  }

  move(dx: number, dy: number): { x: number; y: number } {
    this.x += dx;
    this.y += dy;
    return { x: Math.round(this.x), y: Math.round(this.y) };
  }
}
