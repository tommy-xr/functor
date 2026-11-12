import { test } from "node:test";
import assert from "node:assert/strict";
import type { Layout, Task, View } from "./generated/protocol";
import { isTyping, keysFor, panesOf, shortcutFor, shortcutHelp, tabTitles, viewKey } from "./layout-model.ts";

const file = (path: string): View => ({ kind: "file", path });

test("tab titles: names, and the folder when two open files share one", () => {
  const views = [{ kind: "game" } as View, file("game.fun"), file("src/game.fun"), file("lib/util.fun"), file("a/b/x.fun"), file("c/x.fun")];
  const titles = tabTitles(views, []);
  assert.deepEqual(
    views.map((v) => titles.get(viewKey(v))),
    // A top-level file has no folder to add.
    ["Game", "game.fun", "game.fun · src", "util.fun", "x.fun · a/b", "x.fun · c"],
  );
});

test("tab titles: a task's changes and page by its title", () => {
  const task = { id: 3, title: "Make the cube spin" } as Task;
  const titles = tabTitles([{ kind: "changes", task: 3 }, { kind: "task", task: 3 }, { kind: "changes", task: 9 }], [task]);
  assert.deepEqual([...titles.values()], ["Changes · Make the cube spin", "Task · Make the cube spin", "Changes · task 9"]);
});

test("panes in order", () => {
  const layout: Layout = {
    root: {
      kind: "split",
      id: 4,
      direction: "right",
      ratio: 500,
      first: { kind: "pane", id: 1, tabs: [{ kind: "game" }], active: 0 },
      second: {
        kind: "split",
        id: 5,
        direction: "down",
        ratio: 500,
        first: { kind: "pane", id: 2, tabs: [file("a.fun"), file("b.fun")], active: 1 },
        second: { kind: "pane", id: 3, tabs: [], active: 0 },
      },
    },
    focus: 2,
  };
  assert.deepEqual(panesOf(layout).map((p) => p.id), [1, 2, 3]);
  assert.deepEqual(panesOf(null), []);
});

const keys = (code: string, mods: Partial<Record<"altKey" | "ctrlKey" | "metaKey" | "shiftKey", boolean>> = {}) => ({
  code,
  altKey: false,
  ctrlKey: false,
  metaKey: false,
  shiftKey: false,
  ...mods,
});
const free = { typing: false, captured: false };

test("shortcuts are Option/Alt plus a key", () => {
  assert.equal(shortcutFor(keys("KeyW", { altKey: true }), free), "close_tab");
  assert.equal(shortcutFor(keys("BracketRight", { altKey: true }), free), "next_tab");
  assert.equal(shortcutFor(keys("BracketLeft", { altKey: true }), free), "previous_tab");
  assert.equal(shortcutFor(keys("Backslash", { altKey: true }), free), "split_right");
  assert.equal(shortcutFor(keys("KeyO", { altKey: true }), free), "next_pane");
  // Without Alt, with another modifier too, or another key: none.
  for (const e of [
    keys("KeyW"),
    keys("KeyW", { altKey: true, metaKey: true }),
    keys("KeyW", { altKey: true, ctrlKey: true }),
    keys("KeyW", { altKey: true, shiftKey: true }),
    keys("KeyQ", { altKey: true }),
  ]) {
    assert.equal(shortcutFor(e, free), null, JSON.stringify(e));
  }
});

test("shortcuts are inert while typing or while the game has input", () => {
  const e = keys("KeyW", { altKey: true });
  assert.equal(shortcutFor(e, { typing: true, captured: false }), null);
  assert.equal(shortcutFor(e, { typing: false, captured: true }), null);
  assert.equal(isTyping({ tagName: "TEXTAREA" }), true);
  assert.equal(isTyping({ tagName: "INPUT" }), true);
  assert.equal(isTyping({ tagName: "SELECT" }), true);
  assert.equal(isTyping({ tagName: "DIV", isContentEditable: true }), true);
  assert.equal(isTyping({ tagName: "BUTTON", isContentEditable: false }), false);
  assert.equal(isTyping(null), false);
});

test("shortcut labels follow the platform", () => {
  assert.equal(keysFor("split_right", "MacIntel"), "⌥\\");
  assert.equal(keysFor("close_tab", "Linux x86_64"), "Alt+W");
  assert.match(shortcutHelp("MacIntel"), /^⌥W close tab · ⌥\] next tab/);
});
