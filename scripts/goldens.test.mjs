// Guards the per-example golden manifests (docs/goldens.md): the repo's set
// loads cleanly, and the loader rejects the retired root file, duplicates, and
// malformed entries.
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test, { after } from "node:test";

import { loadGoldenScenarios } from "./goldens.mjs";

const timed = (name) => ({ name, fixedTime: 2.0, debugRender: null, targets: ["native"] });

const roots = [];
after(() => roots.forEach((root) => rmSync(root, { recursive: true, force: true })));

/** A throwaway repo root with `examples/<sample>/goldens.json` per entry. */
function fixture(samples, rootFiles = {}) {
  const root = mkdtempSync(join(tmpdir(), "goldens-"));
  roots.push(root);
  for (const [sample, list] of Object.entries(samples)) {
    mkdirSync(join(root, "examples", sample), { recursive: true });
    writeFileSync(join(root, "examples", sample, "goldens.json"), JSON.stringify(list));
  }
  for (const [file, body] of Object.entries(rootFiles)) writeFileSync(join(root, file), body);
  return root;
}

test("the repo's goldens load", () => {
  assert.ok(loadGoldenScenarios().length > 0);
});

test("order is sample directory, then file order; sample is derived", () => {
  const root = fixture({ b: [timed("b2"), timed("b1")], a: [timed("a1")] });
  assert.deepEqual(
    loadGoldenScenarios(root).map((s) => `${s.sample}/${s.name}`),
    ["a/a1", "b/b2", "b/b1"],
  );
});

test("rejects the retired root golden-scenarios.json", () => {
  const root = fixture({ a: [timed("a1")] }, { "golden-scenarios.json": "{}" });
  assert.throws(() => loadGoldenScenarios(root), /retired/);
});

test("rejects a duplicate name across examples", () => {
  const root = fixture({ a: [timed("x")], b: [timed("x")] });
  assert.throws(() => loadGoldenScenarios(root), /duplicate scenario name 'x'/);
});

test("rejects malformed entries", () => {
  const cases = [
    [{}, /expected an array/],
    [[{ ...timed("a"), sample: "a" }], /unknown field 'sample'/],
    [[{ ...timed("a"), targets: [] }], /'targets'/],
    [[{ ...timed("a"), targets: ["ios"] }], /'targets'/],
    [[{ ...timed(""), name: "" }], /'name'/],
    [[{ ...timed("a"), inputScript: "x.input", captureAtFrame: 9 }], /'fixedTime', or 'inputScript'/],
    [[{ name: "a", targets: ["native"], inputScript: "x.input" }], /'fixedTime', or 'inputScript'/],
  ];
  for (const [list, err] of cases) {
    assert.throws(() => loadGoldenScenarios(fixture({ a: list })), err, JSON.stringify(list));
  }
});
