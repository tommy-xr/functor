// Fake `gh`, `adb` and CI-built `functor` for the pr.mjs and f.mjs tests.

import { execFile } from "node:child_process";
import { chmod, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { delimiter, join } from "node:path";

export const sha = "0123456789abcdef0123456789abcdef01234567";

// Fake `gh`/`adb` on PATH that log every call to <root>/calls. PR 7's head is
// `sha` (from a fork with FAKE_FORK); its one run per workflow ends as
// FAKE_RUN_STATUS (default success), and a failed run holds no artifacts;
// FAKE_DOWNLOAD_FAIL fails a download after writing part of it.
const FAKE_GH = `#!/usr/bin/env node
const fs = require("node:fs"), path = require("node:path");
const root = process.env.FAKE_ROOT, a = process.argv.slice(2);
fs.appendFileSync(path.join(root, "calls"), JSON.stringify(["gh", ...a]) + "\\n");
const sha = "${sha}";
if (a[0] === "pr" && a[1] === "view" && a[2] === "7") {
  console.log(JSON.stringify({ headRefOid: sha, headRefName: "feat/x", isCrossRepository: !!process.env.FAKE_FORK }));
} else if (a[0] === "run" && a[1] === "list") {
  if (a[a.indexOf("--commit") + 1] !== sha || a[a.indexOf("--branch") + 1] !== "feat/x") process.exit(2);
  const status = process.env.FAKE_RUN_STATUS ?? "success";
  if (status === "none") { console.log("[]"); process.exit(0); }
  const done = status === "success" || status === "failure";
  console.log(JSON.stringify([{ databaseId: 42, status: done ? "completed" : status,
    conclusion: done ? status : "", url: "https://runs/42" }]));
} else if (a[0] === "run" && a[1] === "download" && a[2] === "42") {
  if (process.env.FAKE_RUN_STATUS === "failure") { console.error("no artifact matches"); process.exit(1); }
  const name = a[a.indexOf("-n") + 1], out = a[a.indexOf("-D") + 1];
  fs.mkdirSync(out, { recursive: true });
  if (process.env.FAKE_DOWNLOAD_FAIL) { fs.writeFileSync(path.join(out, "functor"), "trunc"); process.exit(1); }
  if (name === "functor-apk-" + sha) fs.writeFileSync(path.join(out, "functor_runtime_oculus.apk"), "CI key");
  else if (name.startsWith("functor-cli-") && name.endsWith(sha)) {
    // Not executable, like an artifact zip's.
    fs.writeFileSync(path.join(out, "functor"), fs.readFileSync(path.join(root, "fake-functor")), { mode: 0o644 });
  } else process.exit(3);
} else process.exit(4);
`;

// One headset listed twice (USB + Wi-Fi), or two headsets with FAKE_DISTINCT.
// <root>/installed-key holds the installed APK's contents; a different one is
// a signing-key mismatch.
const FAKE_ADB = `#!/usr/bin/env node
const fs = require("node:fs"), path = require("node:path");
const root = process.env.FAKE_ROOT, a = process.argv.slice(2);
fs.appendFileSync(path.join(root, "calls"), JSON.stringify(["adb", ...a]) + "\\n");
const key = path.join(root, "installed-key");
if (a[0] === "devices") {
  console.log("List of devices attached\\n2G0YC5ZF87037Q\\tdevice\\n192.168.1.9:5555\\tdevice\\n");
  process.exit(0);
}
if (a[2] === "shell" && a[3] === "getprop" && a[4] === "ro.serialno") {
  console.log(process.env.FAKE_DISTINCT && a[1] !== "2G0YC5ZF87037Q" ? "OTHER" : "2G0YC5ZF87037Q");
  process.exit(0);
}
if (a[0] !== "-s" || a[1] !== "2G0YC5ZF87037Q") process.exit(5);
const cmd = a.slice(2);
if (cmd[0] === "install" && cmd[1] === "-r" && cmd[2] === "-g") {
  const apk = fs.readFileSync(cmd[3], "utf8");
  if (fs.existsSync(key) && fs.readFileSync(key, "utf8") !== apk) {
    console.error("adb: failed to install: Failure [INSTALL_FAILED_UPDATE_INCOMPATIBLE: signatures do not match]");
    process.exit(1);
  }
  fs.writeFileSync(key, apk);
  console.log("Success");
} else if (cmd[0] === "uninstall" && cmd[1] === "dev.functor.runner") {
  fs.rmSync(key);
  console.log("Success");
} else if (cmd[0] !== "shell" || cmd[1] !== "am") process.exit(6);
`;

// The CI-built `functor`: logs its args (with ANDROID_SERIAL) and cwd, then
// exits FAKE_EXIT or kills itself with FAKE_SIGNAL. With FAKE_WAIT it touches
// <root>/ready and waits for a SIGINT, which it logs to <root>/got.
const FAKE_FUNCTOR = `#!/bin/sh
echo '["functor"]' >> "$FAKE_ROOT/calls"
echo "$ANDROID_SERIAL $*" > "$FAKE_ROOT/functor-args"
pwd -P > "$FAKE_ROOT/functor-cwd"
[ -n "$FAKE_SIGNAL" ] && kill -"$FAKE_SIGNAL" $$
if [ -n "$FAKE_WAIT" ]; then
  trap 'kill $!; echo INT > "$FAKE_ROOT/got"; exit 130' INT
  sleep 10 & touch "$FAKE_ROOT/ready"; wait
fi
exit "\${FAKE_EXIT:-0}"
`;

// A temp root holding the fakes, and `run(args, env, cwd)`, which runs
// `script` with them first on PATH and HOME at the root; its promise carries
// the process as `.child`.
export async function harness(t, script) {
  const root = await mkdtemp(join(tmpdir(), "functor-pr-test-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  await writeFile(join(root, "fake-functor"), FAKE_FUNCTOR);
  for (const [name, source] of [["gh", FAKE_GH], ["adb", FAKE_ADB]]) {
    await writeFile(join(root, name), source);
    await chmod(join(root, name), 0o755);
  }
  const calls = async () =>
    (await readFile(join(root, "calls"), "utf8").catch(() => ""))
      .trim()
      .split("\n")
      .filter(Boolean)
      .map((line) => JSON.parse(line));
  const run = (args, env = {}, cwd = undefined) => {
    let child;
    const result = new Promise((resolve) => {
      const { ANDROID_SERIAL, FUNCTOR_PR_CACHE, ...inherited } = process.env;
      const environment = {
        ...inherited,
        PATH: `${root}${delimiter}${process.env.PATH}`,
        HOME: root,
        FAKE_ROOT: root,
        ...env,
      };
      child = execFile(process.execPath, [script, ...args], { env: environment, cwd }, (error, stdout, stderr) =>
        resolve({ code: error ? error.code : 0, signal: error?.signal ?? null, stdout, stderr }),
      );
    });
    return Object.assign(result, { child });
  };
  return { root, calls, run };
}
