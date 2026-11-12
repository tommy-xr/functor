import { test } from "node:test";
import assert from "node:assert/strict";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { Change, DiffLine, FileChange, Task } from "./generated/protocol.ts";
import {
  collapse,
  fileFor,
  hasChange,
  omittedText,
  renderLines,
  totals,
  COLLAPSE_LINES,
} from "./diff-model.ts";

const file = (path: string, added = 1, removed = 0): FileChange => ({
  path,
  from: null,
  added,
  removed,
  binary: false,
  large: false,
});

const change: Change = {
  files: [file("examples/counter/game.fun", 2, 1), file("README.md", 3, 0)],
  more: 0,
  tree: "t",
};

test("a tool call's file finds its entry, absolute or relative to the project", () => {
  const game = change.files[0];
  const at = (path: string, project = "examples/counter") => fileFor(change, "/repo", project, path);
  assert.equal(at("/repo/examples/counter/game.fun"), game);
  assert.equal(at("game.fun"), game);
  assert.equal(at("README.md", "."), change.files[1]);
  // Another checkout of the repo.
  assert.equal(at("/elsewhere/examples/counter/game.fun"), game);
  // Same name, another project: not this change's file.
  assert.equal(at("game.fun", "examples/other"), null);
  assert.equal(at("/repo/examples/counter/other.fun"), null);
  assert.equal(fileFor(null, "/repo", ".", "/repo/README.md"), null);
});

test("totals and whether there is a change", () => {
  assert.deepEqual(totals({ ...change, more: 2 }), { files: 4, added: 5, removed: 1 });
  const task = { baseline: "t", change } as Task;
  assert.ok(hasChange(task));
  assert.ok(!hasChange({ ...task, change: { files: [], more: 0 } }));
  assert.ok(!hasChange({ ...task, baseline: null }));
  assert.ok(!hasChange(null));
});

test("long files collapse until expanded", () => {
  const lines: DiffLine[] = Array.from({ length: COLLAPSE_LINES + 5 }, (_, i) => ({ kind: "context", text: `${i}` }));
  const { shown, hidden } = collapse(lines, false);
  assert.equal(shown.length, COLLAPSE_LINES);
  assert.equal(hidden, 5);
  assert.deepEqual(collapse(lines, true), { shown: lines, hidden: 0 });
  assert.deepEqual(collapse(lines.slice(0, 3), false), { shown: lines.slice(0, 3), hidden: 0 });
});

test("lines are classed by kind and their text is never markup", () => {
  const html = renderToStaticMarkup(
    createElement(
      "div",
      null,
      renderLines([
        { kind: "hunk", text: "@@ -1 +1 @@" },
        { kind: "removed", text: "let a = 1" },
        { kind: "added", text: '<script>alert(1)</script><img src=x onerror="alert(1)">' },
        { kind: "context", text: "" },
        { kind: "note", text: "No newline at end of file" },
      ], "notes.txt"),
    ),
  );
  assert.ok(!/<script|<img/i.test(html), html);
  // Highlighted, the text is still only text.
  const fun = renderToStaticMarkup(createElement("div", null, renderLines([{ kind: "added", text: '"<script>alert(1)</script>"' }], "game.fun")));
  assert.ok(!/<script/i.test(fun) && fun.includes('<span class="tok-string">'), fun);
  assert.ok(html.includes("&lt;script&gt;alert(1)&lt;/script&gt;"), html);
  assert.deepEqual(
    [...html.matchAll(/class="dl dl-(\w+)"/g)].map((m) => m[1]),
    ["hunk", "removed", "added", "context", "note"],
  );
  assert.ok(html.includes('<span class="dl-mark" aria-hidden="true">-</span><span class="dl-text">let a = 1</span>'), html);
});

test("omitted files say why in one line", () => {
  assert.equal(omittedText("binary", file("a.png")), "Binary file: not shown.");
  assert.equal(omittedText("large", file("a.json", 2500, 1)), "2501 changed lines: too large to show.");
});
