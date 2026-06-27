import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";

import { findRepoRoot, formatCrashOutput, stepAll } from "../src/index.js";

// Pure unit tests — no runtime required, always run.

test("findRepoRoot walks up to the cargo workspace root", () => {
  const root = findRepoRoot(process.cwd());
  assert.ok(root !== undefined, "should find a workspace root from the SDK dir");
  assert.ok(
    existsSync(join(root, "Cargo.toml")),
    "found root should contain Cargo.toml",
  );
});

test("findRepoRoot returns undefined when there's no workspace above", () => {
  assert.equal(findRepoRoot("/"), undefined);
});

test("formatCrashOutput keeps the panic and its context", () => {
  const lines = ["init", "loading", "ok", "panicked at foo.rs:1", "stack:", "  0: x"];
  const out = formatCrashOutput(lines);
  assert.match(out, /panicked at foo\.rs:1/);
  assert.match(out, /0: x/, "should include lines after the panic");
  assert.match(out, /ok/, "should include a little context before the panic");
});

test("formatCrashOutput falls back to the tail when there's no panic", () => {
  const lines = Array.from({ length: 50 }, (_, i) => `line ${i}`);
  const out = formatCrashOutput(lines);
  assert.match(out, /line 49/, "should include the last line");
  assert.doesNotMatch(out, /line 0\b/, "should drop the earliest lines");
});

test("stepAll advances every client by the same dt, concurrently", async () => {
  const calls: number[] = [];
  const fake = () => ({
    step: async (dt: number) => {
      calls.push(dt);
    },
  });
  const clients = [fake(), fake(), fake()];

  await stepAll(clients, 0.25);

  assert.deepEqual(calls, [0.25, 0.25, 0.25]);
});
