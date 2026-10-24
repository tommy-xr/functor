#!/usr/bin/env node

/** Quest A/B for the pipelined sim thread and XR thread hints: for each
 * display rate × {inline, pipelined} × {untagged, tagged}, relaunch the APK
 * with the matching `debug.functor.*` properties, push the project, request
 * the rate, and run `scripts/quest-benchmark.mjs`, adding the shell's own
 * `stages over 300 frames` lines from logcat.
 *
 * Needs a release APK installed, one attached headset (or ANDROID_SERIAL), a
 * `functor` CLI for `run vr`, and a runtime with `POST /display-refresh-rate`.
 * It refuses to run while the headset reports lost 6DoF tracking, which makes
 * every configuration fall far below the target rate.
 *
 *   node scripts/quest-pipeline-matrix.mjs --functor target/release/functor \
 *     -d examples/terrain --rates 90,120 --warmup 10 --seconds 30
 *
 * `--rates default` skips the rate request; `--modes inline` (or `pipelined`)
 * runs only those configurations.
 */

import { execFileSync, spawn } from "node:child_process";
import path from "node:path";

const CONFIGS = [
  { name: "inline, untagged", pipelined: "0", tags: "0" },
  { name: "inline, RENDERER_MAIN", pipelined: "0", tags: "1" },
  { name: "pipelined, untagged", pipelined: "1", tags: "0" },
  { name: "pipelined, tagged", pipelined: "1", tags: "1" },
];
const STATE = "http://127.0.0.1:8123/state";

function parseArgs(argv) {
  const options = {
    functor: "target/release/functor",
    dir: "examples/terrain",
    rates: [90, 120],
    modes: ["inline", "pipelined"],
    warmup: 10,
    seconds: 30,
  };
  for (let i = 0; i < argv.length; i += 2) {
    const [flag, value] = [argv[i], argv[i + 1]];
    if (value === undefined) throw new Error(`${flag} requires a value`);
    if (flag === "--functor") options.functor = value;
    else if (flag === "-d") options.dir = value;
    else if (flag === "--rates") options.rates = value.split(",").map((r) => (r === "default" ? null : Number(r)));
    else if (flag === "--modes") options.modes = value.split(",");
    else if (flag === "--warmup") options.warmup = Number(value);
    else if (flag === "--seconds") options.seconds = Number(value);
    else throw new Error(`unknown argument: ${flag}`);
  }
  return options;
}

const adb = (...args) => execFileSync("adb", args, { encoding: "utf8" }).trim();
const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function state() {
  try {
    const response = await fetch(STATE, { signal: AbortSignal.timeout(2_000) });
    return response.ok ? await response.json() : undefined;
  } catch {
    return undefined;
  }
}

function trackingLost() {
  return adb("logcat", "-d", "-t", "200").includes("no 6dof tracking");
}

/** Wait until two `/state` samples show the frame advancing. */
async function waitRendering(timeoutMs = 60_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const first = await state();
    await delay(1_000);
    const second = await state();
    if (first && second && second.frame > first.frame) return second;
  }
  throw new Error("the runtime never started rendering (is the headset awake and tracking?)");
}

async function requestRate(hz) {
  await fetch("http://127.0.0.1:8123/display-refresh-rate", { method: "POST", body: JSON.stringify({ hz }) });
  for (let i = 0; i < 15; i++) {
    const current = (await state())?.display_refresh_rate?.current;
    if (current && Math.abs(current - hz) < 1) return;
    await delay(1_000);
  }
  throw new Error(`display did not switch to ${hz} Hz`);
}

async function measure(options, config, hz) {
  adb("shell", "setprop", "debug.functor.pipelined", config.pipelined);
  adb("shell", "setprop", "debug.functor.thread_settings", config.tags);
  adb("shell", "am", "force-stop", "dev.functor.runner");
  adb("shell", "am", "broadcast", "-a", "com.oculus.vrpowermanager.prox_close");
  adb("logcat", "-c");
  const runVr = spawn(options.functor, ["-d", path.resolve(options.dir), "run", "vr"], { stdio: "ignore" });
  try {
    const ready = await waitRendering();
    if (trackingLost()) throw new Error("the headset reports no 6dof tracking; fix tracking and re-run");
    if (hz !== null) await requestRate(hz);
    const label = `${path.basename(options.dir)} ${config.name} ${hz ?? "default"} Hz`;
    const report = execFileSync(
      "node",
      ["scripts/quest-benchmark.mjs", "--label", label, "--warmup", String(options.warmup), "--seconds", String(options.seconds)],
      { encoding: "utf8" },
    );
    const stages = adb("logcat", "-d", "-s", "functor")
      .split("\n")
      .filter((line) => line.includes("stages over"))
      .slice(-3);
    return `${report}\n- sim_thread: ${ready.sim_thread}\n${stages.map((line) => `- ${line.trim()}`).join("\n")}\n`;
  } finally {
    runVr.kill();
  }
}

const options = parseArgs(process.argv.slice(2));
adb("shell", "am", "broadcast", "-a", "com.oculus.vrpowermanager.prox_close");
await delay(5_000);
if (trackingLost()) {
  throw new Error("the headset reports no 6dof tracking; fix tracking (lights, cameras) before measuring");
}
for (const hz of options.rates) {
  for (const config of CONFIGS.filter((c) => options.modes.some((m) => c.name.startsWith(m)))) {
    console.log(await measure(options, config, hz));
  }
}
adb("shell", "setprop", "debug.functor.pipelined", "0");
// Back to the defaults: inline, hints only when pipelined (an empty value).
adb("shell", "setprop", "debug.functor.thread_settings", "\"\"");
adb("shell", "am", "broadcast", "-a", "com.oculus.vrpowermanager.automation_disable");
