/** A PR's CI builds, shared by `npm run pr` (pr.mjs) and `f --pr` (f.mjs):
 * resolve the head commit, find its finished `Build CLI` / `Build Android`
 * run, download an artifact once into a per-commit cache, and install the
 * Quest APK. Needs `gh`, and `adb` for the APK.
 */

import { execFileSync } from "node:child_process";
import { chmodSync, existsSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const ROOT = dirname(dirname(fileURLToPath(import.meta.url)));
const PACKAGE = "dev.functor.runner";
const ACTIVITY = `${PACKAGE}/android.app.NativeActivity`;
const APK_FILE = "functor_runtime_oculus.apk";
// `${process.platform}-${process.arch}` → build-cli.yml's platform name.
const CLI_PLATFORMS = {
  "darwin-arm64": "macos-arm64",
  "linux-x64": "linux-x64",
};

/** build-cli.yml's platform name for this host, or undefined. */
export function cliPlatform(platform = process.platform, arch = process.arch) {
  return CLI_PLATFORMS[`${platform}-${arch}`];
}

// Output of a finished command; a failure carries its stdout + stderr.
export function run(command, args) {
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

export function prHead(pr) {
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

// The artifact's directory in the per-commit cache (FUNCTOR_PR_CACHE, default
// ~/.functor/pr-artifacts), downloaded on first use. CI uploads it only after
// its build passed, and one commit is one build, so a cached copy is never stale.
function artifact(workflow, name, head) {
  const cache = resolve(process.env.FUNCTOR_PR_CACHE || join(homedir(), ".functor", "pr-artifacts"));
  const dir = join(cache, head.sha, name);
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
  try {
    renameSync(staging, dir);
  } catch (error) {
    // A concurrent call finished the same download first; keep its copy.
    rmSync(staging, { recursive: true, force: true });
    if (!existsSync(dir)) throw error;
  }
  return dir;
}

/** The head commit's Quest APK, downloaded on first use. */
export function apkPath(head) {
  return join(artifact("build-android.yml", `functor-apk-${head.sha}`, head), APK_FILE);
}

/** The head commit's `functor` CLI for `platform`, downloaded on first use. */
export function cliPath(head, platform) {
  const cli = join(artifact("build-cli.yml", `functor-cli-${platform}-${head.sha}`, head), "functor");
  chmodSync(cli, 0o755); // artifact zips drop the executable bit
  return cli;
}

// ANDROID_SERIAL, else the single ready device. One headset listed twice
// (USB and Wi-Fi) reports one hardware serial; then its USB entry wins.
export function deviceSerial() {
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

// The per-device record of the last CI APK installed: its head sha, keyed by
// adb serial. The fallback when the runner can't report `/state.build`.
const record = (serial) => join(homedir(), ".functor", "apk-installed", serial);

/** The head sha of the PR APK last installed on `serial`, if recorded. */
export function recordedApk(serial) {
  try {
    return readFileSync(record(serial), "utf8").trim();
  } catch {
    return undefined;
  }
}

/** Installs head `sha`'s `apk` on the headset, records it, and launches it;
 * returns the device serial. */
export function installAndLaunch(apk, sha, allowUninstall, serial = deviceSerial()) {
  rmSync(record(serial), { force: true }); // a failed install leaves it unknown
  install(serial, apk, allowUninstall);
  mkdirSync(dirname(record(serial)), { recursive: true });
  writeFileSync(record(serial), `${sha}\n`);
  run("adb", ["-s", serial, "shell", "am", "start", "-S", "-n", ACTIVITY]);
  console.log(`launched ${PACKAGE} on ${serial}`);
  return serial;
}
