// The metrics runner against FakeAgent: every scenario's recorded numbers
// come back exactly, and the outcome checks fail when they should. Needs the
// functor-app and functor binaries (see run.mjs).

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { compare, judge, workspace } from "./run.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const scenarios = JSON.parse(fs.readFileSync(path.join(here, "scenarios.json"), "utf8"));
const exe = process.platform === "win32" ? ".exe" : "";
const functor = process.env.FUNCTOR_BIN ?? path.join(here, "../../target/debug", `functor${exe}`);

/** What a fixture records: tool calls by name, text blocks, the last usage. */
function recorded(id) {
  const lines = fs
    .readFileSync(path.join(here, "fixtures", `${id}.jsonl`), "utf8")
    .split("\n")
    .filter((l) => l.trim())
    .map((l) => JSON.parse(l));
  const tools = {};
  let messages = 0;
  let usage = {};
  for (const line of lines) {
    const name = line.mcp ? `mcp__mis__${line.mcp.tool}` : line.event?.type === "tool_use" && line.event.name;
    if (name) tools[name] = (tools[name] ?? 0) + 1;
    if (line.event?.type === "text") messages += 1;
    if (line.event?.type === "usage") usage = line.event;
  }
  return { tools, messages, input: usage.input, output: usage.output };
}

test("every scenario reproduces its fixture's numbers and passes", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "functor-metrics-test-"));
  const out = path.join(tmp, "fake.jsonl");
  const run = spawnSync(process.execPath, [path.join(here, "run.mjs"), "--out", out], { encoding: "utf8" });
  assert.equal(run.status, 0, run.stderr);
  const records = fs
    .readFileSync(out, "utf8")
    .split("\n")
    .filter((l) => l.trim())
    .map((l) => JSON.parse(l));
  assert.deepEqual(
    records.map((r) => r.scenario),
    scenarios.map((s) => s.id),
  );
  for (const r of records) {
    const want = recorded(r.scenario);
    assert.equal(r.pass, true, `${r.scenario}: ${r.reason}`);
    assert.equal(r.ended, "ok", r.scenario);
    assert.equal(r.model, "claude-model", r.scenario);
    assert.deepEqual(r.tools, want.tools, r.scenario);
    assert.equal(r.tool_calls, Object.values(want.tools).reduce((a, n) => a + n, 0), r.scenario);
    assert.equal(
      Object.values(r.calls).reduce((a, n) => a + n, 0),
      r.tool_calls,
      r.scenario,
    );
    assert.equal(r.messages, want.messages, r.scenario);
    assert.equal(r.input_tokens, want.input, r.scenario);
    assert.equal(r.output_tokens, want.output, r.scenario);
    assert.ok(r.first_event_ms <= r.wall_ms, r.scenario);
  }
  const md = fs.readFileSync(path.join(tmp, "fake.md"), "utf8");
  for (const s of scenarios) assert.match(md, new RegExp(`^\\| ${s.id} \\| pass \\|`, "m"));
  assert.doesNotMatch(compare(records, records), /\(\+[1-9]|\(-/);
  fs.rmSync(tmp, { recursive: true, force: true });
});

test("checks fail on the wrong outcome, with the reason", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "functor-metrics-test-"));
  const byId = Object.fromEntries(scenarios.map((s) => [s.id, s]));
  const ws = workspace(byId["two-step"], tmp);
  // Nothing changed: every check of an edit fails.
  assert.deepEqual(judge(byId["two-step"].checks, ws, {}, functor), [
    "game.fun unchanged",
    'game.fun has no /Ui\\.button\\("-1"/',
    "expect failed: Game.update(Game.init, Game.Dec).count == Game.init.count - 1.0",
  ]);
  // A wrong answer, a tool call and a stray file fail the question's checks.
  fs.writeFileSync(path.join(ws, "notes.txt"), "x");
  const metrics = { answer: "ready, I guess not", tool_calls: 1 };
  assert.deepEqual(judge(byId["no-tools"].checks, ws, metrics, functor), [
    "files changed: notes.txt",
    "1 tool calls (max 0)",
    "the answer has no /^\\W*ready\\W*$/i",
  ]);
  fs.rmSync(tmp, { recursive: true, force: true });
});
