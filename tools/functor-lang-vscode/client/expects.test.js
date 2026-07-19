const test = require("node:test");
const assert = require("node:assert");
const expects = require("./expects");

test("mergeStatuses replaces per-uri, keeps absent uris, drops unknown states", () => {
  const byUri = new Map([["file:///old.fun", [{ line: 1, state: "pass" }]]]);
  const touched = expects.mergeStatuses(byUri, {
    generation: 3,
    files: {
      "file:///a.fun": [
        { line: 2, state: "pass" },
        { line: 4, state: "sparkly" }, // future state: dropped
        { line: 5, state: "fail", detail: "left == right — left: 1, right: 2" },
      ],
      "file:///b.fun": [], // authoritative clear
    },
  });
  assert.deepStrictEqual(touched.sort(), ["file:///a.fun", "file:///b.fun"]);
  assert.strictEqual(byUri.get("file:///a.fun").length, 2);
  assert.deepStrictEqual(byUri.get("file:///b.fun"), []);
  // Absent uri untouched.
  assert.strictEqual(byUri.get("file:///old.fun").length, 1);
});

test("mergeStatuses rejects malformed pushes", () => {
  assert.strictEqual(expects.mergeStatuses(new Map(), null), null);
  assert.strictEqual(expects.mergeStatuses(new Map(), { files: 3 }), null);
});

test("groupByState buckets lines per state", () => {
  const groups = expects.groupByState([
    { line: 1, state: "pass" },
    { line: 2, state: "fail" },
    { line: 3, state: "pass" },
    { line: 4, state: "running" },
    { line: 5, state: "unrunnable" },
    { line: 6, state: "error" },
  ]);
  assert.deepStrictEqual(groups.pass, [1, 3]);
  assert.deepStrictEqual(groups.fail, [2]);
  assert.deepStrictEqual(groups.running, [4]);
  assert.deepStrictEqual(groups.unrunnable, [5]);
  assert.deepStrictEqual(groups.error, [6]);
});

test("problemRows carries fail/error details only", () => {
  const rows = expects.problemRows([
    { line: 1, state: "pass" },
    { line: 2, state: "fail", detail: "left == right — left: 1, right: 2" },
    { line: 3, state: "error", detail: "game.fun:3:1: no pattern matched 1" },
    { line: 4, state: "unrunnable", detail: "unknown external `Scene.cube`" },
  ]);
  assert.strictEqual(rows.length, 2);
  assert.strictEqual(rows[0].line, 2);
  assert.match(rows[0].message, /left == right/);
  assert.match(rows[1].message, /no pattern matched/);
});
