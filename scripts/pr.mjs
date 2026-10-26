#!/usr/bin/env node

/** Try a PR's CI build without building it locally:
 *
 *   npm run pr -- <n> [--apk] [--cli] [--run <example>] [--allow-uninstall]
 *
 * Finds the successful `Build Android` / `Build CLI` run for the PR's head
 * commit (refusing a missing, running or failed one), downloads its artifacts
 * once into ~/.functor/pr-artifacts/<sha>/, then:
 *
 * - `--apk` installs the Quest APK (`adb install -r -g`) and launches it.
 *   An install signed with another key or versioned above the CI build must
 *   be uninstalled first, which `--allow-uninstall` permits.
 * - `--cli` prints the path of this host's `functor` binary.
 * - `--run <example>` (implies `--cli`) runs that binary on this checkout's
 *   `examples/<example>`: `run vr` with `--apk`, else `run native`.
 *
 * With neither `--apk` nor `--cli`, both. Needs `gh`, and `adb` for `--apk`.
 */

import { execFileSync, spawnSync } from "node:child_process";
import { chmodSync, existsSync, renameSync, rmSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = dirname(dirname(fileURLToPath(import.meta.url)));
const PACKAGE = "dev.functor.runner";
const ACTIVITY = `${PACKAGE}/android.app.NativeActivity`;
const APK_FILE = "functor_runtime_oculus.apk";
// `${process.platform}-${process.arch}` → build-cli.yml's platform name.
const CLI_PLATFORMS = {
  "darwin-arm64": "macos-arm64",
  "linux-x64": "linux-x64",
  "win32-x64": "windows-x64",
};

function parseArgs(argv) {
  const options = { apk: false, cli: false, allowUninstall: false };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--apk") options.apk = true;
    else if (arg === "--cli") options.cli = true;
    else if (arg === "--allow-uninstall") options.allowUninstall = true;
    else if (arg === "--run") {
      options.run = argv[++i];
      if (!options.run) throw new Error("--run requires an example name");
    } else if (/^[1-9]\d*$/.test(arg) && options.pr === undefined) options.pr = arg;
    else throw new Error(`unknown argument: ${arg}`);
  }
  if (options.pr === undefined) {
    throw new Error("usage: npm run pr -- <n> [--apk] [--cli] [--run <example>] [--allow-uninstall]");
  }
  if (options.run) options.cli = true;
  // Neither asked for: both, skipping a CLI this host has no build of.
  if (!options.apk && !options.cli) options.apk = options.cli = options.defaulted = true;
  return options;
}

// Output of a finished command; a failure carries its stdout + stderr.
function run(command, args) {
  try {
    return execFileSync(command, args, {
      cwd: ROOT,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    });
  } catch (error) {
    const output = `${error.stdout ?? ""}${error.stderr ?? ""}`.trim();
    throw new Error(`${command} ${args.join(" ")} failed${output ? `:\n${output}` : `: ${error.message}`}`);
  }
}

function prHead(pr) {
  const head = JSON.parse(
    run("gh", ["pr", "view", pr, "--json", "headRefOid,headRefName,isCrossRepository"]),
  );
  if (!/^[0-9a-f]{40}$/.test(head.headRefOid ?? "")) {
    throw new Error(`gh pr view ${pr} reported no head commit`);
  }
  // A fork's build is its author's code, debug-signed: never install it blindly.
  if (head.isCrossRepository) throw new Error(`PR #${pr} is from a fork; refusing its CI build`);
  return { sha: head.headRefOid, branch: head.headRefName };
}

// The finished run of `workflow` for the head commit: a successful one, else
// the latest (a CLI matrix with one failed leg still holds the other
// platforms' binaries). A missing or unfinished run is refused. `--branch`
// skips a fork's run of the same commit, which carries a debug-signed APK.
function finishedRun(workflow, { sha, branch }) {
  const runs = JSON.parse(
    run("gh", [
      "run", "list", "--workflow", workflow, "--commit", sha, "--branch", branch,
      "--json", "databaseId,status,conclusion,url",
    ]),
  );
  const latest = runs[0];
  if (!latest) {
    throw new Error(`no ${workflow} run for ${sha}; start one: gh workflow run ${workflow} --ref ${branch}`);
  }
  const success = runs.find((r) => r.status === "completed" && r.conclusion === "success");
  if (success) return success;
  if (latest.status !== "completed") {
    throw new Error(`${workflow} for ${sha} is still ${latest.status}: ${latest.url}`);
  }
  return latest;
}

// The artifact's directory in the per-commit cache, downloaded on first use.
// CI uploads it only after its build passed, and one commit is one build, so
// a cached copy is never stale.
function artifact(workflow, name, head) {
  const dir = join(homedir(), ".functor", "pr-artifacts", head.sha, name);
  if (existsSync(dir)) return dir;
  const found = finishedRun(workflow, head);
  // Staged per process, so an interrupted download is never reused.
  const staging = `${dir}.partial-${process.pid}`;
  rmSync(staging, { recursive: true, force: true });
  console.log(`downloading ${name} from ${found.url}`);
  try {
    run("gh", ["run", "download", String(found.databaseId), "-n", name, "-D", staging]);
  } catch (error) {
    rmSync(staging, { recursive: true, force: true });
    throw new Error(
      `${workflow} for ${head.sha} ended ${found.conclusion} without ${name} (artifacts expire after 30 days): ${found.url}\n${error.message}`,
    );
  }
  renameSync(staging, dir);
  return dir;
}

// ANDROID_SERIAL, else the single ready device. One headset listed twice
// (USB and Wi-Fi) reports one hardware serial; then its USB entry wins.
function deviceSerial() {
  if (process.env.ANDROID_SERIAL) return process.env.ANDROID_SERIAL;
  const ready = run("adb", ["devices"])
    .split("\n")
    .map((line) => line.trim().split(/\s+/))
    .filter(([serial, state]) => serial && state === "device")
    .map(([serial]) => serial);
  if (ready.length === 0) throw new Error("no adb device is ready (`adb devices`)");
  if (ready.length === 1) return ready[0];
  const hardware = new Set(
    ready.map((serial) => run("adb", ["-s", serial, "shell", "getprop", "ro.serialno"]).trim()),
  );
  const usb = ready.filter((serial) => !/:\d+$/.test(serial) && !serial.includes("._adb-tls"));
  if (hardware.size === 1 && usb.length === 1) return usb[0];
  throw new Error(`several adb devices (${ready.join(", ")}); set ANDROID_SERIAL to pick one`);
}

function install(serial, apk, allowUninstall) {
  const adbInstall = () => console.log(run("adb", ["-s", serial, "install", "-r", "-g", apk]).trim());
  try {
    adbInstall();
  } catch (error) {
    const reason = error.message.includes("INSTALL_FAILED_UPDATE_INCOMPATIBLE")
      ? "the installed runner is signed with another key"
      : error.message.includes("INSTALL_FAILED_VERSION_DOWNGRADE")
        ? "the installed runner has a higher versionCode (a release, or an older 0.1.0 local build)"
        : undefined;
    if (!reason) throw error;
    if (!allowUninstall) {
      throw new Error(`${reason}; replacing it uninstalls it and deletes its app data (rerun with --allow-uninstall)`);
    }
    console.log(`${reason}; uninstalling it`);
    run("adb", ["-s", serial, "uninstall", PACKAGE]);
    adbInstall();
  }
}

function main() {
  const options = parseArgs(process.argv.slice(2));
  const example = options.run && join(ROOT, "examples", options.run);
  if (example && !existsSync(join(example, "functor.json"))) {
    throw new Error(`no example at ${example}`);
  }
  const head = prHead(options.pr);
  console.log(`PR #${options.pr} ${head.branch} @ ${head.sha}`);

  // Every download precedes any device change, so a refusal touches nothing.
  const apk = options.apk && join(artifact("build-android.yml", `functor-apk-${head.sha}`, head), APK_FILE);
  const platform = CLI_PLATFORMS[`${process.platform}-${process.arch}`];
  let cli;
  if (options.cli && !platform) {
    const message = `CI builds no functor CLI for ${process.platform}-${process.arch}`;
    if (!options.defaulted) throw new Error(message);
    console.log(`${message}; skipping it`);
  } else if (options.cli) {
    const name = process.platform === "win32" ? "functor.exe" : "functor";
    cli = join(artifact("build-cli.yml", `functor-cli-${platform}-${head.sha}`, head), name);
    chmodSync(cli, 0o755); // artifact zips drop the executable bit
    console.log(`functor CLI: ${cli}`);
  }

  let serial;
  if (apk) {
    serial = deviceSerial();
    install(serial, apk, options.allowUninstall);
    run("adb", ["-s", serial, "shell", "am", "start", "-S", "-n", ACTIVITY]);
    console.log(`launched ${PACKAGE} on ${serial}`);
  }
  if (example) {
    const result = spawnSync(cli, ["-d", example, "run", apk ? "vr" : "native"], {
      stdio: "inherit",
      env: serial ? { ...process.env, ANDROID_SERIAL: serial } : process.env,
    });
    if (result.error) throw result.error;
    process.exitCode = result.status ?? 1;
  }
}

try {
  main();
} catch (error) {
  console.error(`pr: ${error instanceof Error ? error.message : error}`);
  process.exitCode = 1;
}
