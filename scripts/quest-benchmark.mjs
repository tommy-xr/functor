#!/usr/bin/env node

/** Measure the currently loaded Functor game on an adb-attached Quest.
 *
 * Uses Meta's one-second VrApi telemetry instead of host-side frame timing, so
 * the result includes the actual device scheduler/compositor path. The script
 * also samples process memory after the run. It never installs an APK or pushes
 * a game: launch/push the exact release build and workload you want first.
 */

import { execFileSync, spawn } from "node:child_process";

const PACKAGE = "dev.functor.runner";
const ACTIVITY = `${PACKAGE}/android.app.NativeActivity`;

function parseArgs(argv) {
  const result = { seconds: 20, warmup: 5, label: "current game" };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    const value = argv[++i];
    if (value === undefined) throw new Error(`${arg} requires a value`);
    if (arg === "--seconds") result.seconds = Number(value);
    else if (arg === "--warmup") result.warmup = Number(value);
    else if (arg === "--label") result.label = value;
    else throw new Error(`unknown argument: ${arg}`);
  }
  for (const [name, value] of [
    ["seconds", result.seconds],
    ["warmup", result.warmup],
  ]) {
    if (!Number.isFinite(value) || value < 0) {
      throw new Error(`--${name} must be a non-negative number`);
    }
  }
  if (result.seconds < 3) throw new Error("--seconds must be at least 3");
  return result;
}

function adb(args, options = {}) {
  return execFileSync("adb", args, {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
    ...options,
  }).trim();
}

const delay = (milliseconds) =>
  new Promise((resolve) => setTimeout(resolve, milliseconds));

async function waitForRuntime(timeoutMs = 20_000) {
  adb(["forward", "tcp:8123", "tcp:8123"]);
  const deadline = Date.now() + timeoutMs;
  let lastError;
  while (Date.now() < deadline) {
    try {
      const response = await fetch("http://127.0.0.1:8123/state", {
        signal: AbortSignal.timeout(1_000),
      });
      if (response.ok) return response.json();
      lastError = new Error(`GET /state returned ${response.status}`);
    } catch (error) {
      lastError = error;
    }
    await delay(250);
  }
  throw new Error(`debug runtime did not become ready: ${lastError}`);
}

async function collectVrApi(seconds) {
  const child = spawn(
    "adb",
    ["logcat", "-v", "brief", "-T", "1", "VrApi:I", "*:S"],
    { stdio: ["ignore", "pipe", "pipe"] },
  );
  let output = "";
  let errors = "";
  child.stdout.setEncoding("utf8");
  child.stderr.setEncoding("utf8");
  child.stdout.on("data", (chunk) => (output += chunk));
  child.stderr.on("data", (chunk) => (errors += chunk));

  await delay(seconds * 1_000);
  child.kill("SIGTERM");
  await new Promise((resolve) => {
    child.once("close", resolve);
    setTimeout(resolve, 2_000);
  });
  if (!output.includes("FPS=")) {
    throw new Error(`no VrApi telemetry received${errors ? `: ${errors}` : ""}`);
  }
  return output;
}

function field(line, name) {
  const match = line.match(new RegExp(`(?:^|[ ,])${name}=(-?[0-9.]+)`));
  return match ? Number(match[1]) : undefined;
}

function summarize(values) {
  if (values.length === 0) return undefined;
  const sorted = values.toSorted((a, b) => a - b);
  const sum = values.reduce((total, value) => total + value, 0);
  const p95 = sorted[Math.ceil(sorted.length * 0.95) - 1];
  const p5 = sorted[Math.floor((sorted.length - 1) * 0.05)];
  return {
    mean: Number((sum / values.length).toFixed(3)),
    min: sorted[0],
    p5,
    p95,
    max: sorted.at(-1),
  };
}

function parseTelemetry(text) {
  // `-T 1` may include one pre-measurement tail line. Dropping the first
  // sample also avoids counting a partial one-second telemetry bucket.
  const lines = text.split("\n").filter((line) => line.includes("FPS=")).slice(1);
  if (lines.length < 2) throw new Error("not enough complete VrApi samples");
  const values = (name) =>
    lines.map((line) => field(line, name)).filter((value) => value !== undefined);
  return {
    samples: lines.length,
    fps: summarize(values("FPS")),
    app_ms: summarize(values("App")),
    compositor_ms: summarize(values("TW")),
    cpu_gpu_ms: summarize(values("CPU&GPU")),
    gpu_load: summarize(values("GPU%")),
    cpu_load: summarize(values("CPU%")),
    stale_frames: values("Stale").reduce((sum, value) => sum + value, 0),
    torn_frames: values("Tear").reduce((sum, value) => sum + value, 0),
  };
}

function parseMemory(text) {
  const number = (pattern) => {
    const match = text.match(pattern);
    return match ? Number(match[1]) : undefined;
  };
  const mib = (kilobytes) =>
    kilobytes === undefined ? undefined : kilobytes / 1024;
  return {
    total_pss_mib: mib(number(/TOTAL PSS:\s+(\d+)/)),
    total_rss_mib: mib(number(/TOTAL RSS:\s+(\d+)/)),
    graphics_pss_mib: mib(number(/Graphics:\s+(\d+)/)),
  };
}

function printMetric(label, metric, suffix = "") {
  if (!metric) return;
  console.log(
    `| ${label} | ${metric.mean}${suffix} | ${metric.p95}${suffix} | ${metric.max}${suffix} |`,
  );
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  const serial = adb(["get-serialno"]);
  const model = adb(["shell", "getprop", "ro.product.model"]);
  const packageInfo = adb(["shell", "dumpsys", "package", PACKAGE]);
  if (/\bDEBUGGABLE\b/.test(packageInfo)) {
    throw new Error(
      "installed Functor APK is debuggable; install a --release cargo-apk build before benchmarking",
    );
  }
  let runningPid = "";
  try {
    runningPid = adb(["shell", "pidof", PACKAGE]);
  } catch {
    // `pidof` exits non-zero when no process exists.
  }
  if (!runningPid) {
    throw new Error(
      "Functor is not already running; launch it and push the exact workload before benchmarking",
    );
  }
  let preflight;
  try {
    preflight = await waitForRuntime(2_000);
  } catch {
    throw new Error(
      "Functor process exists but its debug runtime is not alive; launch it and push the exact workload before benchmarking",
    );
  }

  // The proximity override makes unattended runs enter FOCUSED. It remains
  // enabled deliberately so a caller can benchmark several workloads. The
  // live `/state` preflight above proves the pushed runtime exists, and
  // reorder-to-front resumes that activity instead of creating a new one.
  // Undo with the automation_disable command printed below.
  adb(["shell", "am", "broadcast", "-a", "com.oculus.vrpowermanager.prox_close"]);
  adb(["shell", "am", "start", "--activity-reorder-to-front", "-n", ACTIVITY]);
  const initial = await waitForRuntime();
  const resumedPid = adb(["shell", "pidof", PACKAGE]);
  if (resumedPid !== runningPid || initial.frame < preflight.frame) {
    throw new Error(
      "Functor restarted while foregrounding; push the intended workload again before benchmarking",
    );
  }
  await delay(options.warmup * 1_000);
  const before = await waitForRuntime();
  const telemetryText = await collectVrApi(options.seconds);
  const after = await waitForRuntime();
  if (after.frame <= before.frame) {
    throw new Error(`runtime did not render during sample (frame stayed ${after.frame})`);
  }

  const telemetry = parseTelemetry(telemetryText);
  const memory = parseMemory(adb(["shell", "dumpsys", "meminfo", PACKAGE]));
  const views = (after.views ?? []).map((view) =>
    `${view.name} ${view.viewport.width}x${view.viewport.height}`,
  );

  console.log(`# Quest device benchmark: ${options.label}`);
  console.log("");
  console.log(`- Device: ${model} (${serial})`);
  console.log(`- Runtime frames: ${before.frame} → ${after.frame}`);
  console.log(`- Views: ${views.length ? views.join(", ") : `${after.viewport.width}x${after.viewport.height}`}`);
  console.log(`- Sample: ${telemetry.samples} complete VrApi one-second buckets after ${options.warmup}s warmup`);
  console.log(`- FPS: ${telemetry.fps.mean} mean, ${telemetry.fps.p5} p5, ${telemetry.fps.min} min`);
  console.log("");
  console.log("| Metric | Mean | p95 | Max |");
  console.log("| --- | ---: | ---: | ---: |");
  printMetric("App", telemetry.app_ms, " ms");
  printMetric("Timewarp/compositor", telemetry.compositor_ms, " ms");
  printMetric("CPU + GPU", telemetry.cpu_gpu_ms, " ms");
  printMetric("GPU load", telemetry.gpu_load);
  printMetric("CPU load", telemetry.cpu_load);
  console.log("");
  console.log(`- Stale frames: ${telemetry.stale_frames}`);
  console.log(`- Torn frames: ${telemetry.torn_frames}`);
  console.log(`- Memory: ${memory.total_pss_mib?.toFixed(1)} MiB PSS, ${memory.total_rss_mib?.toFixed(1)} MiB RSS, ${memory.graphics_pss_mib?.toFixed(1)} MiB graphics PSS`);
  console.log("- Restore proximity automation: `adb shell am broadcast -a com.oculus.vrpowermanager.automation_disable`");

  // Keep this read so the initial state is not an accidental unused probe: it
  // confirms the endpoint before warmup and helps diagnose a reset in output.
  if (initial.frame > before.frame) {
    console.error("warning: runtime frame counter moved backwards during warmup");
  }
}

main().catch((error) => {
  console.error(`quest-benchmark: ${error instanceof Error ? error.message : error}`);
  process.exitCode = 1;
});
