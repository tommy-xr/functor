#!/usr/bin/env node
// Agent-run metrics (plan 1.19): runs each scenario in scenarios.json through
// `functor-app task` on a fresh copy of its example, judges the outcome by
// its checks, and records turns, tokens, time and outcome per scenario.
//
//   node app/metrics/run.mjs                 FakeAgent, replaying fixtures/<id>.jsonl
//   FUNCTOR_APP_LIVE=1 node app/metrics/run.mjs --live
//                                            the real agent (spends tokens)
//   node app/metrics/run.mjs --compare a.jsonl b.jsonl
//
// Options: --only id,id  --out file.jsonl (default results/<stamp>-<mode>.jsonl,
// with a .md table beside it). Binaries: FUNCTOR_APP_BIN, FUNCTOR_BIN (default
// target/debug/functor-app and target/debug/functor).

import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(here, "../..");
const exe = process.platform === "win32" ? ".exe" : "";

// Live limits per scenario: past either, the run is stopped and no further
// scenario starts.
const MAX_INPUT = 600_000;
const LIVE_TIMEOUT_S = 300;
const FAKE_TIMEOUT_S = 60;

// Git that reads no user or system config.
const gitEnv = { ...process.env, GIT_CONFIG_GLOBAL: os.devNull, GIT_CONFIG_NOSYSTEM: "1" };
const git = (cwd, ...args) =>
  spawnSync("git", ["-c", "user.name=metrics", "-c", "user.email=metrics@example.invalid", ...args], {
    cwd,
    env: gitEnv,
    encoding: "utf8",
  });

/** A fresh git workspace for `scenario` in `tmp`: its example at the root,
 *  the repo's functor-lang skill (the preamble points to it), setup applied. */
export function workspace(scenario, tmp) {
  const ws = path.join(tmp, "ws");
  fs.cpSync(path.join(repo, "examples", scenario.project), ws, { recursive: true });
  const skill = ".claude/skills/functor-lang";
  fs.cpSync(path.join(repo, skill), path.join(ws, skill), { recursive: true });
  for (const { file, old, new: replacement } of scenario.setup ?? []) {
    const text = fs.readFileSync(path.join(ws, file), "utf8");
    if (!text.includes(old)) throw new Error(`${scenario.id}: ${file} has no ${JSON.stringify(old)}`);
    fs.writeFileSync(path.join(ws, file), text.replace(old, replacement));
  }
  for (const args of [["init", "-q", "-b", "main"], ["add", "-A"], ["commit", "-q", "-m", "scenario"]]) {
    const out = git(ws, ...args);
    if (out.status !== 0) throw new Error(`git ${args.join(" ")}: ${out.stderr}`);
  }
  return ws;
}

/** Each check's failure, or null: `checks` against the workspace `ws` after
 *  the run and the run's `metrics`. */
export function judge(checks, ws, metrics, functor) {
  const failures = [];
  for (const check of checks) {
    const why = checkOne(check, ws, metrics, functor);
    if (why) failures.push(why);
  }
  return failures;
}

function checkOne(check, ws, metrics, functor) {
  if (check.unchanged) {
    const status = git(ws, "status", "--porcelain").stdout.trim();
    return status && `files changed: ${status.split("\n").map((l) => l.slice(3)).join(", ")}`;
  }
  if (check.changed) {
    const diff = git(ws, "status", "--porcelain", "--", check.changed).stdout.trim();
    return !diff && `${check.changed} unchanged`;
  }
  if (check.matches) {
    const { file, pattern } = check.matches;
    const text = fs.existsSync(path.join(ws, file)) ? fs.readFileSync(path.join(ws, file), "utf8") : "";
    return !new RegExp(pattern).test(text) && `${file} has no /${pattern}/`;
  }
  if (check.answer) {
    return !new RegExp(check.answer, "i").test(metrics.answer ?? "") && `the answer has no /${check.answer}/i`;
  }
  if ("max_tools" in check) {
    return metrics.tool_calls > check.max_tools && `${metrics.tool_calls} tool calls (max ${check.max_tools})`;
  }
  if (check.expect) {
    // A sibling module of expects, run by `functor test`, then removed.
    const file = path.join(ws, "outcome_check.fun");
    fs.writeFileSync(file, check.expect.map((e) => `expect ${e}\n`).join(""));
    const out = spawnSync(functor, ["-d", ws, "test"], { encoding: "utf8" });
    fs.rmSync(file);
    return out.status !== 0 && `expect failed: ${check.expect.join("; ")}`;
  }
  if (check.functor) {
    const out = spawnSync(functor, ["-d", ws, check.functor], { encoding: "utf8" });
    return out.status !== 0 && `functor ${check.functor} exited ${out.status}`;
  }
  throw new Error(`unknown check ${JSON.stringify(check)}`);
}

/** Runs `functor-app task` for `scenario`; its metrics record. */
async function runScenario(scenario, { live, app, functor, rev }) {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), `functor-metrics-${scenario.id}-`));
  try {
    const ws = workspace(scenario, tmp);
    const metricsFile = path.join(tmp, "metrics.json");
    const env = { ...process.env, FUNCTOR_APP_HOME: path.join(tmp, "home"), FUNCTOR_BIN: functor };
    delete env.FUNCTOR_APP_FAKE_AGENT;
    if (!live) env.FUNCTOR_APP_FAKE_AGENT = path.join(here, "fixtures", `${scenario.id}.jsonl`);
    const args = ["--headless", "--no-device-link", "-d", ws];
    // Live sessions get a GPU, as in the app, so captures work.
    if (!live) args.push("--no-gpu");
    args.push("task", scenario.prompt, "--harness", "claude", "--start-session");
    args.push("--metrics", metricsFile, "--max-input", String(MAX_INPUT));
    const timeout = live ? LIVE_TIMEOUT_S : FAKE_TIMEOUT_S;
    const { code, timedOut } = await run(app, args, env, timeout);
    if (!fs.existsSync(metricsFile)) throw new Error(`${scenario.id}: no metrics (exit ${code})`);
    const m = JSON.parse(fs.readFileSync(metricsFile, "utf8"));
    if (timedOut) m.cut = `over ${timeout} s`;
    const failures = m.ended === "ok" ? judge(scenario.checks, ws, m, functor) : [`the run ended ${m.ended}`];
    return {
      scenario: scenario.id,
      mode: live ? "live" : "fake",
      rev,
      date: new Date().toISOString(),
      harness: m.harness,
      model: m.model,
      wall_ms: m.wall_ms,
      first_event_ms: m.first_event_ms,
      messages: m.messages,
      tool_calls: m.tool_calls,
      calls: m.calls,
      tools: m.tools,
      input_tokens: m.input_tokens,
      output_tokens: m.output_tokens,
      ended: m.ended,
      cut: m.cut,
      pass: failures.length === 0,
      reason: failures.length ? failures.join("; ") : `${scenario.checks.length} checks pass`,
      exit: code,
      answer: m.answer,
    };
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }
}

/** Runs `cmd`, its output passed through to stderr; SIGINT after `timeout`
 *  seconds (the app then stops the run and still writes its metrics). */
function run(cmd, args, env, timeout) {
  return new Promise((resolve, reject) => {
    const child = spawn(cmd, args, { env, stdio: ["ignore", "pipe", "pipe"] });
    child.stdout.pipe(process.stderr);
    child.stderr.pipe(process.stderr);
    let timedOut = false;
    const timer = setTimeout(() => {
      timedOut = true;
      child.kill("SIGINT");
    }, timeout * 1000);
    child.on("error", reject);
    child.on("close", (code) => {
      clearTimeout(timer);
      resolve({ code, timedOut });
    });
  });
}

const secs = (ms) => (ms == null ? "–" : (ms / 1000).toFixed(1));
const kinds = (calls) =>
  Object.entries(calls ?? {})
    .map(([k, n]) => `${k} ${n}`)
    .join(", ") || "none";

/** The records as a Markdown table. */
export function table(records) {
  const head = records[0] ?? {};
  const lines = [
    `${head.mode} run, ${head.date?.slice(0, 10)}, rev ${head.rev}, ${head.harness} (${head.model ?? "model unknown"})`,
    "",
    "| scenario | outcome | wall s | first event s | messages | tool calls | by kind | input tokens | output tokens |",
    "| --- | --- | --- | --- | --- | --- | --- | --- | --- |",
  ];
  for (const r of records) {
    const outcome = r.pass ? "pass" : `FAIL: ${r.reason}`;
    const cut = r.cut ? ` (cut: ${r.cut})` : "";
    lines.push(
      `| ${r.scenario} | ${outcome}${cut} | ${secs(r.wall_ms)} | ${secs(r.first_event_ms)} | ${r.messages} | ` +
        `${r.tool_calls} | ${kinds(r.calls)} | ${r.input_tokens ?? "–"} | ${r.output_tokens ?? "–"} |`,
    );
  }
  return lines.join("\n") + "\n";
}

/** `b` against `a`, per scenario both ran. */
export function compare(a, b) {
  const delta = (x, y) => {
    if (x == null || y == null) return `${x ?? "–"} → ${y ?? "–"}`;
    const pct = x === 0 ? "" : ` (${y >= x ? "+" : ""}${Math.round(((y - x) / x) * 100)}%)`;
    return `${x} → ${y}${pct}`;
  };
  const lines = [
    `rev ${a[0]?.rev} → ${b[0]?.rev}`,
    "",
    "| scenario | outcome | wall ms | tool calls | input tokens | output tokens |",
    "| --- | --- | --- | --- | --- | --- |",
  ];
  for (const x of a) {
    const y = b.find((r) => r.scenario === x.scenario);
    if (!y) continue;
    const outcome = `${x.pass ? "pass" : "fail"} → ${y.pass ? "pass" : "fail"}`;
    lines.push(
      `| ${x.scenario} | ${outcome} | ${delta(x.wall_ms, y.wall_ms)} | ${delta(x.tool_calls, y.tool_calls)} | ` +
        `${delta(x.input_tokens, y.input_tokens)} | ${delta(x.output_tokens, y.output_tokens)} |`,
    );
  }
  return lines.join("\n") + "\n";
}

const readJsonl = (file) =>
  fs
    .readFileSync(file, "utf8")
    .split("\n")
    .filter((l) => l.trim())
    .map((l) => JSON.parse(l));

async function main(argv) {
  const opt = (name) => {
    const i = argv.indexOf(name);
    return i < 0 ? undefined : argv[i + 1];
  };
  if (argv.includes("--compare")) {
    const i = argv.indexOf("--compare");
    process.stdout.write(compare(readJsonl(argv[i + 1]), readJsonl(argv[i + 2])));
    return 0;
  }
  const live = argv.includes("--live");
  if (live && process.env.FUNCTOR_APP_LIVE !== "1") {
    console.error("--live spends real tokens: set FUNCTOR_APP_LIVE=1 to run it");
    return 2;
  }
  const app = process.env.FUNCTOR_APP_BIN ?? path.join(repo, "target/debug", `functor-app${exe}`);
  const functor = process.env.FUNCTOR_BIN ?? path.join(repo, "target/debug", `functor${exe}`);
  for (const bin of [app, functor]) {
    if (!fs.existsSync(bin)) {
      console.error(`no ${bin}: build it, or set FUNCTOR_APP_BIN / FUNCTOR_BIN`);
      return 2;
    }
  }
  const head = git(repo, "rev-parse", "--short", "HEAD").stdout.trim();
  const dirty = git(repo, "status", "--porcelain").stdout.trim() ? "-dirty" : "";
  const rev = `${head}${dirty}`;
  const only = opt("--only")?.split(",");
  const scenarios = JSON.parse(fs.readFileSync(path.join(here, "scenarios.json"), "utf8")).filter(
    (s) => !only || only.includes(s.id),
  );
  const stamp = new Date().toISOString().replace(/[:.]/g, "-");
  const out = opt("--out") ?? path.join(here, "results", `${stamp}-${live ? "live" : "fake"}.jsonl`);
  fs.mkdirSync(path.dirname(out), { recursive: true });
  fs.writeFileSync(out, "");
  const records = [];
  for (const scenario of scenarios) {
    console.error(`[metrics] ${scenario.id}`);
    const record = await runScenario(scenario, { live, app, functor, rev });
    records.push(record);
    fs.appendFileSync(out, JSON.stringify(record) + "\n");
    if (live && record.cut) {
      console.error(`[metrics] ${scenario.id} was cut (${record.cut}): stopping`);
      break;
    }
  }
  const md = table(records);
  fs.writeFileSync(out.replace(/\.jsonl$/, "") + ".md", md);
  process.stdout.write(md);
  console.error(`[metrics] wrote ${out}`);
  return records.every((r) => r.pass) ? 0 : 1;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  process.exitCode = await main(process.argv.slice(2));
}
