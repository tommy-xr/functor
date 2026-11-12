import { test } from "node:test";
import assert from "node:assert/strict";
import type { Template } from "./generated/protocol";
import { ALL, filterTemplates, stepTemplate, tagChips } from "./new-project-model.ts";

const t = (name: string, description: string, tags: string[], kind: Template["kind"] = "starter"): Template => ({
  name,
  description,
  tags,
  kind,
});
const LIST = [
  t("3d", "A small lit 3D scene", ["3d"]),
  t("2d", "Shapes on a 2D camera", ["2d", "keyboard"]),
  t("fps", "First person: WASD and mouse look", ["3d", "first-person", "keyboard", "mouse"]),
  t("tetris", "A complete falling-block puzzle", ["2d", "puzzle", "keyboard"], "example"),
];
const names = (list: Template[]) => list.map((x) => x.name);

test("chips are All, then the tags most used first", () => {
  assert.deepEqual(tagChips(LIST), [ALL, "keyboard", "2d", "3d", "first-person", "mouse", "puzzle"]);
  assert.deepEqual(tagChips([]), [ALL]);
});

test("search matches every word in the name, description or tags", () => {
  assert.deepEqual(names(filterTemplates(LIST, "", ALL)), ["3d", "2d", "fps", "tetris"]);
  assert.deepEqual(names(filterTemplates(LIST, "PUZZLE", ALL)), ["tetris"]);
  assert.deepEqual(names(filterTemplates(LIST, "mouse look", ALL)), ["fps"]);
  assert.deepEqual(names(filterTemplates(LIST, "camera  2d", ALL)), ["2d"]);
  assert.deepEqual(names(filterTemplates(LIST, "nothing", ALL)), []);
});

test("a chip keeps the templates with that tag, and search narrows it", () => {
  assert.deepEqual(names(filterTemplates(LIST, "", "keyboard")), ["2d", "fps", "tetris"]);
  assert.deepEqual(names(filterTemplates(LIST, "complete", "keyboard")), ["tetris"]);
  assert.deepEqual(names(filterTemplates(LIST, "", "vr")), []);
});

test("arrow keys move through the shown templates, wrapping; Home and End jump", () => {
  assert.equal(stepTemplate(LIST, "3d", "ArrowRight"), "2d");
  assert.equal(stepTemplate(LIST, "3d", "ArrowDown"), "2d");
  assert.equal(stepTemplate(LIST, "tetris", "ArrowRight"), "3d");
  assert.equal(stepTemplate(LIST, "3d", "ArrowLeft"), "tetris");
  assert.equal(stepTemplate(LIST, "2d", "End"), "tetris");
  assert.equal(stepTemplate(LIST, "fps", "Home"), "3d");
  // A selection filtered out: the first shown.
  assert.equal(stepTemplate(LIST.slice(1), "3d", "ArrowRight"), "2d");
  assert.equal(stepTemplate(LIST, "3d", "a"), null);
  assert.equal(stepTemplate([], "3d", "ArrowRight"), null);
});
