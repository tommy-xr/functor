#!/usr/bin/env node

/** Desktop per-stage frame cost, inline vs `--pipelined`, with and without
 * the sim thread's QoS raise.
 *
 * Each run is a hidden window kept off the 60 Hz sleep cap by a long
 * `--capture-time`, streaming `frame_stats` over `--json`. The configs are
 * interleaved round by round so host load hits them alike, and the table
 * reports the median of each config's per-run means. Use a release build:
 *
 *   npm run build:cli
 *   node scripts/pipeline-stages.mjs --functor target/release/functor \
 *     --samples synthwave,terrain --rounds 3 --seconds 12
 */

import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import os from "node:os";
import path from "node:path";

const CONFIGS = [
  { name: "inline", env: {}, args: [] },
  { name: "pipelined, default QoS", env: { FUNCTOR_THREAD_QOS: "off" }, args: ["--pipelined"] },
  { name: "pipelined, raised QoS", env: {}, args: ["--pipelined"] },
];

function parseArgs(argv) {
  const options = {
    functor: "target/release/functor",
    samples: ["synthwave", "terrain"],
    rounds: 3,
    seconds: 12,
  };
  for (let i = 0; i < argv.length; i += 2) {
    const [flag, value] = [argv[i], argv[i + 1]];
    if (value === undefined) throw new Error(`${flag} requires a value`);
    if (flag === "--functor") options.functor = value;
    else if (flag === "--samples") options.samples = value.split(",");
    else if (flag === "--rounds") options.rounds = Number(value);
    else if (flag === "--seconds") options.seconds = Number(value);
    else throw new Error(`unknown argument: ${flag}`);
  }
  return options;
}

/** A sample name under examples/, or a path to a project directory. */
function projectDir(sample) {
  return existsSync(sample) ? path.resolve(sample) : path.resolve("examples", sample);
}

/** One run: mean sim/render/wait over its stats windows after the first. */
function runOnce(options, sample, config) {
  const capture = path.join(os.tmpdir(), "functor-pipeline-stages.png");
  const args = [
    "--json", "-d", projectDir(sample), "run", "native", "--",
    "--capture-frame", capture, "--capture-time", String(options.seconds),
    "--capture-size", "1280x720", ...config.args,
  ];
  return new Promise((resolve, reject) => {
    const child = spawn(options.functor, args, {
      env: { ...process.env, ...config.env },
      stdio: ["ignore", "pipe", "ignore"],
    });
    let output = "";
    child.stdout.setEncoding("utf8");
    child.stdout.on("data", (chunk) => (output += chunk));
    child.on("error", reject);
    child.on("close", () => {
      const rows = output
        .split("\n")
        .filter((line) => line.includes('"frame_stats"'))
        .map((line) => JSON.parse(line));
      const use = rows.length > 1 ? rows.slice(1) : rows;
      if (use.length === 0) return reject(new Error(`no frame_stats from ${sample} (${config.name})`));
      const mean = (key) => use.reduce((sum, row) => sum + (row[key] ?? 0), 0) / use.length;
      resolve({ sim: mean("sim_us"), render: mean("render_us"), wait: mean("sim_wait_us") });
    });
  });
}

const median = (values) => {
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.floor(sorted.length / 2)];
};

const options = parseArgs(process.argv.slice(2));
if (!existsSync(options.functor)) throw new Error(`no functor binary at ${options.functor}`);
const results = new Map();
for (let round = 1; round <= options.rounds; round++) {
  console.error(`round ${round}/${options.rounds} (load ${os.loadavg()[0].toFixed(1)})`);
  for (const sample of options.samples) {
    for (const config of CONFIGS) {
      const key = `${sample}\u0000${config.name}`;
      const run = await runOnce(options, sample, config);
      results.set(key, [...(results.get(key) ?? []), run]);
    }
  }
}

console.log(`Median of ${options.rounds} interleaved ${options.seconds}s runs, µs per frame.\n`);
console.log("| Scene | Mode | sim_us | render_us | sim_wait_us |");
console.log("| --- | --- | ---: | ---: | ---: |");
for (const [key, runs] of results) {
  const [sample, mode] = key.split("\u0000");
  const cell = (field) => median(runs.map((run) => run[field])).toFixed(0);
  console.log(`| ${path.basename(sample)} | ${mode} | ${cell("sim")} | ${cell("render")} | ${cell("wait")} |`);
}
