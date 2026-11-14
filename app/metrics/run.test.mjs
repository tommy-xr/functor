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

import { compare, judge, readJsonl, workspace } from "./run.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const scenarios = JSON.parse(fs.readFileSync(path.join(here, "scenarios.json"), "utf8"));
const exe = process.platform === "win32" ? ".exe" : "";
const functor = process.env.FUNCTOR_BIN ?? path.join(here, "../../target/debug", `functor${exe}`);

/** What a fixture records: tool calls by name, text blocks, the last usage. */
function recorded(id) {
  const lines = readJsonl(path.join(here, "fixtures", `${id}.jsonl`));
  const tools = {};
  let messages = 0;
  let usage = {};
  let detail = null;
  for (const line of lines) {
    const name = line.mcp ? `mcp__mis__${line.mcp.tool}` : line.event?.type === "tool_use" && line.event.name;
    if (name) tools[name] = (tools[name] ?? 0) + 1;
    if (line.event?.type === "text") messages += 1;
    if (line.event?.type === "usage") usage = line.event;
    if (line.event?.type === "usage_detail") detail = line.event;
  }
  return { tools, messages, input: usage.input, output: usage.output, detail };
}

/** A temp dir removed after test `t`. */
function tempDir(t) {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "functor-metrics-test-"));
  t.after(() => fs.rmSync(tmp, { recursive: true, force: true }));
  return tmp;
}

test("every scenario reproduces its fixture's numbers and passes", (t) => {
  const tmp = tempDir(t);
  const byId = Object.fromEntries(scenarios.map((s) => [s.id, s]));
  const review = recorded("review");
  const out = path.join(tmp, "fake.jsonl");
  const run = spawnSync(process.execPath, [path.join(here, "run.mjs"), "--out", out], { encoding: "utf8" });
  assert.equal(run.status, 0, run.stderr);
  const records = readJsonl(out);
  assert.deepEqual(
    records.map((r) => r.scenario),
    scenarios.map((s) => s.id),
  );
  for (const r of records) {
    const want = recorded(r.scenario);
    assert.equal(r.pass, true, `${r.scenario}: ${r.reason}`);
    assert.equal(r.ended, "ok", r.scenario);
    assert.equal(r.model, "claude-model", r.scenario);
    assert.equal(r.scope, "user_and_project", r.scenario);
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
    // The split as recorded, or none of it.
    assert.equal(r.uncached_input, want.detail?.total.input, r.scenario);
    assert.equal(r.cache_read, want.detail?.total.cache_read, r.scenario);
    assert.equal(r.cache_creation, want.detail?.total.cache_creation, r.scenario);
    assert.equal(r.turns, want.detail?.turns, r.scenario);
    assert.equal(r.cost_usd, want.detail?.cost_usd, r.scenario);
    assert.equal(r.api_ms, want.detail?.api_ms, r.scenario);
    assert.ok(r.first_event_ms != null && r.first_event_ms <= r.wall_ms, r.scenario);
    // A change is reviewed after the run (D31), its numbers apart; an answer is not.
    const edits = byId[r.scenario].checks.some((c) => c.changed);
    if (edits) {
      assert.equal(r.review.ended, "prepared", r.scenario);
      assert.equal(r.review.input_tokens, review.input, r.scenario);
      assert.equal(r.review.output_tokens, review.output, r.scenario);
      assert.equal(r.review.cache_read, review.detail.total.cache_read, r.scenario);
      assert.equal(r.review.turns, undefined, r.scenario);
      assert.equal(r.review.summary, "The agent changed one value in `game.fun`. Check the game.", r.scenario);
      assert.ok(r.review.wall_ms != null, r.scenario);
    } else {
      assert.equal(r.review, undefined, r.scenario);
    }
  }
  const md = fs.readFileSync(path.join(tmp, "fake.md"), "utf8");
  for (const s of scenarios) assert.match(md, new RegExp(`^\\| ${s.id} \\| pass \\|`, "m"));
  assert.match(md, /^fake run, .*, setup: user_and_project$/m);
  assert.match(md, /^\| weak-jump \| prepared \| claude-review-model \|/m);
  // The split's columns show where any record has them; a record without shows a dash.
  assert.match(md, /^\| scenario \| outcome \| .* \| output tokens \| uncached in \| cache read \| cache write \| cached % \| turns \| cost \$ \| API s \|$/m);
  assert.match(md, /^\| no-tools \| pass \| .* \| 4 \| 4 \| 30000 \| 5779 \| 84 \| 1 \| 0\.021 \| 1\.5 \|$/m);
  assert.match(md, /^\| state-question \| pass \| .* \| – \| – \| – \| – \| – \| – \| – \|$/m);
  assert.match(md, /^\| scenario \| review \| .* \| style flags \| uncached in \| cache read \| cache write \| cached % \|$/m);
});

test("compare shows each scenario's deltas, and those on one side only", () => {
  const a = [{ scenario: "x", pass: true, rev: "a", wall_ms: 1000, tool_calls: 4, input_tokens: 200, output_tokens: 10 }];
  const b = [
    { scenario: "x", pass: false, rev: "b", wall_ms: 500, tool_calls: 4, input_tokens: 300, output_tokens: 10 },
    { scenario: "y", pass: true, rev: "b" },
  ];
  const rows = compare(a, b).split("\n");
  assert.ok(rows.includes("| x | pass → fail | 1000 → 500 (-50%) | 4 → 4 (+0%) | 200 → 300 (+50%) | 10 → 10 (+0%) |"));
  assert.ok(rows.includes("| y | (missing) → pass | – → – | – → – | – → – | – → – |"));
});

test("checks fail on the wrong outcome, with the reason", (t) => {
  const tmp = tempDir(t);
  const byId = Object.fromEntries(scenarios.map((s) => [s.id, s]));
  const ws = workspace(byId["two-step"], tmp);
  // Nothing changed: every check of an edit fails.
  const failed = judge(byId["two-step"].checks, ws, {}, functor);
  assert.deepEqual(failed.slice(0, 3), [
    "game.fun unchanged",
    'game.fun has no /Ui\\.button\\("-1", *Dec\\)/',
    "game.fun has no /expect[^\\n]*\\bDec\\b/",
  ]);
  assert.match(failed[3], /^expect failed: Game\.update\(Game\.init, Game\.Dec\)\S* .*: error: /);
  assert.equal(failed.length, 4);
  // A wrong answer, a tool call and a stray file fail the question's checks.
  fs.writeFileSync(path.join(ws, "notes.txt"), "x");
  const metrics = { answer: "ready, I guess not", tool_calls: 1 };
  assert.deepEqual(judge(byId["no-tools"].checks, ws, metrics, functor), [
    "files changed: notes.txt",
    "1 tool calls (max 0)",
    "the answer has no /^\\W*ready\\W*$/i",
  ]);
  // A file the agent commits still counts as changed.
  const git = ["-c", "user.name=t", "-c", "user.email=t@example.invalid"];
  const env = { ...process.env, GIT_CONFIG_GLOBAL: os.devNull, GIT_CONFIG_NOSYSTEM: "1" };
  spawnSync("git", [...git, "add", "-A"], { cwd: ws, env });
  assert.equal(spawnSync("git", [...git, "commit", "-qm", "agent"], { cwd: ws, env }).status, 0);
  assert.deepEqual(judge([{ unchanged: true }], ws, {}, functor), ["files changed: notes.txt"]);
});
