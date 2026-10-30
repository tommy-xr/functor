#!/usr/bin/env node

/** `f`: run the functor CLI — this checkout's build, or a PR's CI build.
 *
 *   f <functor args>                 target/release or target/debug functor, the newer
 *   f --pr <n> <functor args>        PR <n>'s CI-built CLI (cached per commit)
 *
 * `--pr` may sit anywhere before a `--`. With `run vr` it first installs and
 * launches the PR's Quest APK (via pr-builds.mjs, as `npm run pr -- <n> --apk`
 * does), so the PR CLI pushes to a matching runner; `--no-apk` skips that and
 * `--allow-uninstall` permits replacing a differently signed runner.
 * Runs in the caller's cwd, so `-d` paths resolve as typed.
 */

import { spawn } from "node:child_process";
import { realpathSync, statSync } from "node:fs";
import { constants } from "node:os";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { apkPath, cliPath, cliPlatform, installAndLaunch, prHead, ROOT } from "./pr-builds.mjs";

const USAGE = `usage: f [--pr <n> [--no-apk] [--allow-uninstall]] <functor args>

  f -d examples/hello run native         this checkout's target/{release,debug}/functor
  f --pr 836 -d examples/hello test      PR #836's CI-built functor (cached per commit)
  f --pr 836 -d examples/hello run vr    also installs PR #836's Quest APK first

  --no-apk           with run vr: keep the headset's installed APK
  --allow-uninstall  with run vr: replace a differently signed/newer runner
  f help             functor's own help

Cache: ~/.functor/pr-artifacts (FUNCTOR_PR_CACHE overrides).`;

/** Splits f's own flags from the functor args; everything from `--` on passes through. */
export function parseArgs(argv) {
  const options = { help: argv[0] === "--help" || argv[0] === "-h", apk: true, allowUninstall: false, args: [] };
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
    else if (arg === "--allow-uninstall") options.allowUninstall = true;
    else options.args.push(arg);
  }
  if (options.pr === undefined && (!options.apk || options.allowUninstall)) {
    throw new Error("--no-apk and --allow-uninstall need --pr <n>");
  }
  return options;
}

/** Whether the functor args push to a headset (`run vr` / `develop vr`). */
export function wantsVr(args) {
  return args.some((arg, i) => (arg === "run" || arg === "develop") && args[i + 1] === "vr");
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

// Runs `bin` with stdio and cwd inherited, then exits as it did: same code,
// or re-raises its terminating signal.
function exec(bin, args, env) {
  const child = spawn(bin, args, { stdio: "inherit", env });
  // A terminal's Ctrl-C / Ctrl-\ already reaches the child (same process
  // group), so only swallow those; forward the rest.
  const handlers = {
    SIGINT: () => {},
    SIGQUIT: () => {},
    SIGTERM: () => child.kill("SIGTERM"),
    SIGHUP: () => child.kill("SIGHUP"),
  };
  for (const [signal, handler] of Object.entries(handlers)) process.on(signal, handler);
  child.on("error", (error) => {
    console.error(`f: ${bin}: ${error.message}`);
    process.exit(1);
  });
  child.on("exit", (code, signal) => {
    for (const [name, handler] of Object.entries(handlers)) process.off(name, handler);
    if (!signal) process.exit(code ?? 1);
    process.exitCode = 128 + constants.signals[signal]; // if Node ignores it (SIGPIPE)
    process.kill(process.pid, signal);
  });
}

function main() {
  const options = parseArgs(process.argv.slice(2));
  if (options.help) return console.log(USAGE);
  if (options.pr === undefined) {
    const bin = localBinary(ROOT);
    console.error(`f: using ${relative(ROOT, bin)}`);
    return exec(bin, options.args, process.env);
  }

  // stdout belongs to functor; the shared helpers' progress goes to stderr.
  console.log = console.error;
  const platform = cliPlatform();
  if (!platform) {
    throw new Error(`CI builds no functor CLI for ${process.platform}-${process.arch} (only darwin-arm64, linux-x64)`);
  }
  const head = prHead(options.pr);
  const cli = cliPath(head, platform);
  const vr = wantsVr(options.args);
  // Both downloads precede any device change, so a refusal touches nothing.
  const apk = vr && options.apk && apkPath(head);
  console.error(`f: using PR #${options.pr} @ ${head.sha.slice(0, 7)}`);
  let env = process.env;
  if (apk) {
    console.error(`f: run vr: installing PR #${options.pr}'s APK so the headset matches (--no-apk skips)`);
    env = { ...env, ANDROID_SERIAL: installAndLaunch(apk, options.allowUninstall) };
  } else if (vr) {
    console.error("f: --no-apk: pushing to the headset's installed APK as is");
  }
  exec(cli, options.args, env);
}

// Run only as a script (also through an `npm link` symlink), not on import.
if (process.argv[1] && realpathSync(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main();
  } catch (error) {
    console.error(`f: ${error instanceof Error ? error.message : error}`);
    process.exitCode = 1;
  }
}
