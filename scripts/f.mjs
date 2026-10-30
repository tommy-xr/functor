#!/usr/bin/env node

/** `f`: run the functor CLI — this checkout's build, or a PR's CI build.
 *
 *   f <functor args>                 target/release or target/debug functor, the newer
 *   f --pr <n> <functor args>        PR <n>'s CI-built CLI (cached per commit)
 *
 * `--pr` may sit anywhere before a `--`. With `run vr`, PR mode also downloads
 * the PR's Quest APK and installs it unless the headset already runs it (per
 * its runner's `/state.build`, else the last install record), so the PR CLI
 * pushes to a matching runner. Local mode instead checks the headset runner
 * speaks this checkout's debug protocol. Runs in the caller's cwd, so `-d`
 * paths resolve as typed.
 */

import { spawn } from "node:child_process";
import { readFileSync, realpathSync, statSync } from "node:fs";
import { constants } from "node:os";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import {
  apkPath, cliPath, cliPlatform, deviceSerial, installAndLaunch, prHead, recordedApk, ROOT, run,
} from "./pr-builds.mjs";

const USAGE = `usage: f [--pr <n>] [--no-apk] [--force-apk] [--allow-uninstall] <functor args>

  f -d examples/hello run native         this checkout's target/{release,debug}/functor
  f --pr 836 -d examples/hello test      PR #836's CI-built functor (cached per commit)
  f --pr 836 -d examples/hello run vr    also installs PR #836's Quest APK, unless the
                                         headset already runs it

  --no-apk           with run vr: skip all APK handling (install, and local mode's check)
  --force-apk        with --pr and run vr: reinstall the PR's APK even if it matches
  --allow-uninstall  with --pr and run vr: replace a differently signed/newer runner
  f help             functor's own help

Cache: ~/.functor/pr-artifacts (FUNCTOR_PR_CACHE overrides).`;
// functor's global flags that take a value, e.g. `--entry server`.
const VALUE_FLAGS = ["-d", "--dir", "--entry", "--mount"];
const FORWARDED = ["SIGINT", "SIGQUIT", "SIGTERM", "SIGHUP"];
// The headset runner's debug port, which `run vr` also forwards.
const VR_PORT = 8123;
const LOCAL_APK = "npm run build:oculus:apk && adb install -r target-android/debug/apk/functor_runtime_oculus.apk";

/** Splits f's own flags from the functor args; everything from `--` on passes through. */
export function parseArgs(argv) {
  const options = {
    help: argv[0] === "--help" || argv[0] === "-h", apk: true, forceApk: false, allowUninstall: false, args: [],
  };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--") {
      options.args.push(...argv.slice(i));
      break;
    }
    if (arg === "--pr" || arg.startsWith("--pr=")) {
      const pr = arg === "--pr" ? argv[++i] : arg.slice("--pr=".length);
      if (!/^[1-9]\d*$/.test(pr ?? "")) throw new Error("--pr requires a PR number");
      options.pr = pr;
    } else if (arg === "--no-apk") options.apk = false;
    else if (arg === "--force-apk") options.forceApk = true;
    else if (arg === "--allow-uninstall") options.allowUninstall = true;
    else options.args.push(arg);
  }
  if (options.pr === undefined && (options.forceApk || options.allowUninstall)) {
    throw new Error("--force-apk and --allow-uninstall need --pr <n>");
  }
  if (!options.apk && options.forceApk) throw new Error("--no-apk and --force-apk conflict");
  return options;
}

/** Whether the functor args push to a headset (`run vr` / `develop vr`). */
export function wantsVr(args) {
  // The positional words before `--`: `-d ex run --entry s vr` → run, vr.
  const words = [];
  for (let i = 0; i < args.length && args[i] !== "--"; i++) {
    if (args[i] === "--help" || args[i] === "-h") return false;
    if (VALUE_FLAGS.includes(args[i])) i++;
    else if (!args[i].startsWith("-")) words.push(args[i]);
  }
  return (words[0] === "run" || words[0] === "develop") && words[1] === "vr";
}

/** The newer of `root`'s release and debug functor builds. */
export function localBinary(root) {
  const builds = ["release", "debug"]
    .map((profile) => join(root, "target", profile, "functor"))
    .flatMap((path) => {
      try {
        return [{ path, mtime: statSync(path).mtimeMs }];
      } catch {
        return [];
      }
    })
    .sort((a, b) => b.mtime - a.mtime);
  if (builds.length === 0) {
    throw new Error(`no functor build in ${join(root, "target")}; build one with \`npm run build:cli\` (or use --pr <n>)`);
  }
  return builds[0].path;
}

// What the headset's runner reports — `{ protocol, build }` from `GET /` and
// `GET /state` (`build` undefined if that fails) — or undefined when it can't
// be reached.
async function headsetRuntime(serial) {
  const port = process.env.FUNCTOR_VR_HOST_PORT ?? VR_PORT; // tests point this at a fake
  const get = async (path) => {
    const response = await fetch(`http://127.0.0.1:${port}${path}`, { signal: AbortSignal.timeout(3000) });
    if (!response.ok) throw new Error(`GET ${path}: ${response.status}`);
    return response.json();
  };
  let protocol;
  try {
    run("adb", ["-s", serial, "forward", `tcp:${port}`, `tcp:${VR_PORT}`]);
    ({ protocol_version: protocol } = await get("/"));
  } catch {
    return undefined;
  }
  const build = await get("/state").then((state) => state.build, () => undefined);
  return { protocol, build };
}

// Whether `serial` needs head `sha`'s APK, and why: its runner's build label
// decides, else the last install record; unknown installs.
async function apkDecision(serial, sha) {
  const runtime = await headsetRuntime(serial);
  if (runtime) {
    const label = runtime.build ? `\`${runtime.build}\`` : "no build label";
    return { install: runtime.build !== `pr | ${sha.slice(0, 7)}`, why: `its runner reports ${label}` };
  }
  const recorded = recordedApk(serial);
  return {
    install: recorded !== sha,
    why: recorded
      ? `runner unreachable; last recorded install: ${recorded.slice(0, 7)}`
      : "runner unreachable, no current install record",
  };
}

// Local mode's `run vr`: refuses a headset runner on another debug protocol
// than this checkout's (the protocol is read from source, so it trusts the
// local build is current); warns on a non-local build or no answer.
async function checkLocalHeadset() {
  let serial;
  try {
    serial = deviceSerial();
  } catch (error) {
    console.error(`f: can't check the headset (${error.message}); continuing`);
    return undefined;
  }
  const runtime = await headsetRuntime(serial);
  if (!runtime) {
    console.error("f: the headset's runner isn't reachable, so it's unchecked against this CLI; continuing");
    return serial;
  }
  const file = join(ROOT, "runtime/functor-runtime-common/src/debug_protocol.rs");
  const match = readFileSync(file, "utf8").match(/DEBUG_PROTOCOL_VERSION: u32 = (\d+)/);
  if (!match) throw new Error(`no DEBUG_PROTOCOL_VERSION in ${file}`);
  const local = Number(match[1]);
  if (runtime.protocol !== local) {
    throw new Error(
      `the headset's runner speaks debug protocol v${runtime.protocol}, this checkout v${local} ` +
        `(if your local CLI predates the checkout, \`npm run build:cli\` first). ` +
        `Install a matching APK (${LOCAL_APK}), or run a PR's CLI + APK pair (f --pr <n> …).`,
    );
  }
  if (runtime.build && runtime.build !== "dev") {
    console.error(`f: the headset runs \`${runtime.build}\`, not a local build (same protocol v${local}); continuing`);
  }
  return serial;
}

// Runs `bin` with stdio and cwd inherited, then exits as it did: same code,
// or re-raises its terminating signal.
function exec(bin, args, env) {
  const child = spawn(bin, args, { stdio: "inherit", env });
  // Forward signals sent to f. A terminal's Ctrl-C then reaches the child
  // twice (it is in the same process group), which functor tolerates.
  const forward = (signal) => child.kill(signal);
  for (const signal of FORWARDED) process.on(signal, forward);
  child.on("error", (error) => {
    console.error(`f: ${bin}: ${error.message}`);
    process.exit(1);
  });
  child.on("exit", (code, signal) => {
    if (!signal) process.exit(code ?? 1);
    // Die by the same signal so a calling shell sees it; a core-dumping one
    // (SIGABRT, SIGQUIT) or one Node ignores (SIGPIPE) becomes 128 + n.
    process.exitCode = 128 + constants.signals[signal];
    if (!["SIGINT", "SIGTERM", "SIGHUP", "SIGKILL"].includes(signal)) return;
    for (const name of FORWARDED) process.off(name, forward);
    process.kill(process.pid, signal);
  });
}

const withSerial = (serial) => (serial ? { ...process.env, ANDROID_SERIAL: serial } : process.env);

async function main() {
  const options = parseArgs(process.argv.slice(2));
  if (options.help) return console.log(USAGE);
  const vr = wantsVr(options.args) && options.apk;
  if (options.pr === undefined) {
    const bin = localBinary(ROOT);
    console.error(`f: using ${relative(ROOT, bin)}`);
    return exec(bin, options.args, withSerial(vr && (await checkLocalHeadset())));
  }

  // stdout belongs to functor; the shared helpers' progress goes to stderr.
  console.log = console.error;
  const platform = cliPlatform();
  if (!platform) {
    throw new Error(`CI builds no functor CLI for ${process.platform}-${process.arch} (only darwin-arm64, linux-x64)`);
  }
  const head = prHead(options.pr);
  const pr = `PR #${options.pr} @ ${head.sha.slice(0, 7)}`;
  // Both downloads (CLI first) precede any device call, so a missing or
  // unfinished artifact fails before anything touches the headset.
  const cli = cliPath(head, platform);
  let apk;
  try {
    apk = vr && apkPath(head);
  } catch (error) {
    throw new Error(`${error.message}\n(--no-apk runs against the headset's installed APK)`);
  }
  console.error(`f: using ${pr}`);
  if (!apk) {
    if (wantsVr(options.args)) console.error("f: --no-apk: pushing to the headset's installed APK as is");
    return exec(cli, options.args, process.env);
  }
  const serial = deviceSerial();
  const decision = options.forceApk ? { install: true, why: "--force-apk" } : await apkDecision(serial, head.sha);
  if (decision.install) {
    console.error(`f: installing ${pr}'s APK (${decision.why}; --no-apk skips)`);
    installAndLaunch(apk, head.sha, options.allowUninstall, serial);
  } else {
    console.error(`f: headset already runs ${pr} (${decision.why}) — skipping install (--force-apk reinstalls)`);
  }
  exec(cli, options.args, withSerial(serial));
}

// Run only as a script (also through an `npm link` symlink), not on import.
if (process.argv[1] && realpathSync(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(`f: ${error instanceof Error ? error.message : error}`);
    process.exitCode = 1;
  });
}
