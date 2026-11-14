import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { Held, KEY_VARIANTS, keyAction, keyDown, LockedCursor, wireKey, wireName } from "./game-input.ts";

/** The built-in `Key` module's constructors, read from its source (the runtime's `Key` enum is tested against it). */
function keyModule(): string[] {
  const src = readFileSync(new URL("../../../functor-lang/stdlib/key.fun", import.meta.url), "utf8");
  const body = src.slice(src.indexOf("type t ="));
  return [...body.matchAll(/\|\s*([A-Z]\w*)/g)].map((m) => m[1]!);
}

test("the table covers exactly the Key module's variants", () => {
  const module = keyModule();
  assert.ok(module.length > 40, `read ${module.length} variants`);
  assert.deepEqual(new Set(Object.values(KEY_VARIANTS)), new Set(module));
});

test("keys map to the wire's names", () => {
  const cases: [string, string][] = [
    ["KeyA", "a"],
    ["KeyZ", "z"],
    ["Digit0", "0"],
    ["Numpad7", "7"],
    ["ArrowUp", "up"],
    ["Space", "space"],
    ["Enter", "enter"],
    ["Escape", "escape"],
    ["Tab", "tab"],
    ["ShiftLeft", "shift"],
    ["ShiftRight", "shift"],
    ["ControlRight", "control"],
    ["AltLeft", "alt"],
    ["F1", "f1"],
    ["F12", "f12"],
    ["Minus", "minus"],
    ["BracketLeft", "leftbracket"],
    ["Quote", "quote"],
  ];
  for (const [code, name] of cases) assert.equal(wireKey(code), name, code);
  assert.equal(wireName("Num9"), "9");
  assert.equal(wireName("LeftBracket"), "leftbracket");
});

test("keys the game does not hear are not forwarded", () => {
  for (const code of ["MetaLeft", "Backquote", "Backspace", "F13", "NumpadEnter", "", "toString", "constructor"]) {
    assert.equal(wireKey(code), null, code);
  }
});

const press = (code: string, mods: { meta?: boolean; repeat?: boolean } = {}) => ({
  code,
  metaKey: mods.meta ?? false,
  repeat: mods.repeat ?? false,
});

test("⌘Esc releases, ⌘ chords are the app's, Escape and Tab are the game's", () => {
  assert.equal(keyAction(press("Escape", { meta: true })), "release");
  assert.equal(keyAction(press("KeyW", { meta: true })), "app");
  assert.equal(keyAction(press("MetaLeft", { meta: true })), "app");
  assert.deepEqual(keyAction(press("Escape")), { game: "escape" });
  assert.deepEqual(keyAction(press("Tab")), { game: "tab" });
  assert.deepEqual(keyAction(press("F5")), { game: "f5" });
  assert.equal(keyAction(press("Backquote")), null);
});

test("each press gets one release; repeats and stray releases send nothing", () => {
  const held = new Held();
  assert.deepEqual(keyDown(held, press("KeyW")).inputs, [{ type: "key", key: "w", down: true }]);
  // Auto-repeat is still the game's (kept from the page) but sends no edge.
  assert.deepEqual(keyDown(held, press("KeyW", { repeat: true })), { inputs: [], release: false, handled: true });
  // The other Shift while one is held is the same key.
  keyDown(held, press("ShiftLeft"));
  assert.deepEqual(keyDown(held, press("ShiftRight")).inputs, []);
  assert.deepEqual(held.key("w", false), { type: "key", key: "w", down: false });
  assert.equal(held.key("w", false), null);
  assert.equal(held.key("q", false), null);
  assert.deepEqual(held.button("left", true), { type: "mouse_button", button: "left", down: true });
  assert.equal(held.button("left", true), null);
});

test("⌘ going down lets go of held keys, which macOS would never release", () => {
  const held = new Held();
  keyDown(held, press("KeyW"));
  held.button("right", true);
  assert.deepEqual(keyDown(held, press("MetaLeft", { meta: true })), {
    inputs: [{ type: "key", key: "w", down: false }],
    release: false,
    handled: false,
  });
  // A ⌘ chord is not forwarded, and its key-up sends nothing.
  assert.deepEqual(keyDown(held, press("KeyR", { meta: true })).inputs, []);
  assert.equal(held.key("r", false), null);
  assert.deepEqual([...held.buttons], ["right"]);
});

test("releasing lets go of every held key and button", () => {
  const held = new Held();
  keyDown(held, press("Tab"));
  keyDown(held, press("F1"));
  keyDown(held, press("ControlLeft"));
  held.button("left", true);
  const out = keyDown(held, press("Escape", { meta: true }));
  assert.equal(out.release, true);
  assert.deepEqual(out.inputs, [
    { type: "key", key: "tab", down: false },
    { type: "key", key: "f1", down: false },
    { type: "key", key: "control", down: false },
    { type: "mouse_button", button: "left", down: false },
  ]);
  assert.deepEqual(held.releaseAll(), []);
});

test("a locked cursor moves by deltas from where it was locked, unbounded", () => {
  const cursor = new LockedCursor({ x: 400, y: 300 });
  assert.deepEqual(cursor.move(10, -5), { x: 410, y: 295 });
  // Fractions add up rather than rounding away.
  cursor.move(0.4, 0);
  assert.deepEqual(cursor.move(0.4, 0), { x: 411, y: 295 });
  assert.deepEqual(cursor.move(-1000, 2000), { x: -589, y: 2295 });
});
