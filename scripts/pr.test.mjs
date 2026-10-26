import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { chmod, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { delimiter, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const script = fileURLToPath(new URL("./pr.mjs", import.meta.url));
const sha = "0123456789abcdef0123456789abcdef01234567";

// Fake `gh`/`adb` on PATH that log every call to <root>/calls. PR 7's head is
// `sha`; its one run per workflow ends as FAKE_RUN_STATUS (default success).
const FAKE_GH = `#!/usr/bin/env node
const fs = require("node:fs"), path = require("node:path");
const root = process.env.FAKE_ROOT, a = process.argv.slice(2);
fs.appendFileSync(path.join(root, "calls"), JSON.stringify(["gh", ...a]) + "\\n");
const sha = "${sha}";
if (a[0] === "pr" && a[1] === "view" && a[2] === "7") {
  console.log(JSON.stringify({ headRefOid: sha, headRefName: "feat/x" }));
} else if (a[0] === "run" && a[1] === "list") {
  if (a[a.indexOf("--commit") + 1] !== sha) process.exit(2);
  const status = process.env.FAKE_RUN_STATUS ?? "success";
  if (status === "none") { console.log("[]"); process.exit(0); }
  const done = status === "success" || status === "failure";
  console.log(JSON.stringify([{ databaseId: 42, status: done ? "completed" : status,
    conclusion: done ? status : "", url: "https://runs/42" }]));
} else if (a[0] === "run" && a[1] === "download" && a[2] === "42") {
  const name = a[a.indexOf("-n") + 1], out = a[a.indexOf("-D") + 1];
  fs.mkdirSync(out, { recursive: true });
  if (name === "functor-apk-" + sha) fs.writeFileSync(path.join(out, "functor_runtime_oculus.apk"), "CI key");
  else if (name.startsWith("functor-cli-") && name.endsWith(sha)) {
    fs.writeFileSync(path.join(out, process.platform === "win32" ? "functor.exe" : "functor"),
      "#!/bin/sh\\necho '[\\"functor\\"]' >> \\"$FAKE_ROOT/calls\\"\\necho \\"$ANDROID_SERIAL $*\\" > \\"$FAKE_ROOT/functor-args\\"\\n");
  } else process.exit(3);
} else process.exit(4);
`;

// One headset listed twice (USB + Wi-Fi). <root>/installed-key holds the
// installed APK's contents; a different one is a signing-key mismatch.
const FAKE_ADB = `#!/usr/bin/env node
const fs = require("node:fs"), path = require("node:path");
const root = process.env.FAKE_ROOT, a = process.argv.slice(2);
fs.appendFileSync(path.join(root, "calls"), JSON.stringify(["adb", ...a]) + "\\n");
const key = path.join(root, "installed-key");
if (a[0] === "devices") {
  console.log("List of devices attached\\n2G0YC5ZF87037Q\\tdevice\\n192.168.1.9:5555\\tdevice\\n");
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

async function harness(t) {
  const root = await mkdtemp(join(tmpdir(), "functor-pr-test-"));
  t.after(() => rm(root, { recursive: true, force: true }));
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
  const pr = (args, env = {}) =>
    new Promise((resolve) => {
      const environment = {
        ...process.env,
        PATH: `${root}${delimiter}${process.env.PATH}`,
        HOME: root,
        FAKE_ROOT: root,
        ...env,
      };
      delete environment.ANDROID_SERIAL;
      execFile(process.execPath, [script, ...args], { env: environment }, (error, stdout, stderr) =>
        resolve({ code: error ? error.code : 0, stdout, stderr }),
      );
    });
  return { root, calls, pr };
}

test("refuses a missing, running or failed build before touching the device", async (t) => {
  for (const [status, message] of [
    ["none", "gh workflow run build-android.yml --ref feat/x"],
    ["in_progress", "still in_progress: https://runs/42"],
    ["failure", "ended failure: https://runs/42"],
  ]) {
    const h = await harness(t);
    const result = await h.pr(["7", "--apk"], { FAKE_RUN_STATUS: status });
    assert.equal(result.code, 1, status);
    assert.match(result.stderr, new RegExp(message.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")));
    assert.ok(!(await h.calls()).some((call) => call[0] === "adb"), status);
  }
});

test("uninstalls a differently signed runner only with --allow-uninstall, reusing the cache", async (t) => {
  const h = await harness(t);
  await writeFile(join(h.root, "installed-key"), "local key");

  const refused = await h.pr(["7", "--apk"]);
  assert.equal(refused.code, 1);
  assert.match(refused.stderr, /signed with another key.*--allow-uninstall/);
  let calls = await h.calls();
  assert.ok(!calls.some((call) => call.includes("uninstall") || call.includes("am")));
  assert.equal(await readFile(join(h.root, "installed-key"), "utf8"), "local key");

  const allowed = await h.pr(["7", "--apk", "--allow-uninstall"]);
  assert.equal(allowed.code, 0, allowed.stderr);
  calls = await h.calls();
  assert.equal(calls.filter((call) => call[1] === "run" && call[2] === "download").length, 1);
  const index = (word) => calls.findIndex((call) => call.includes(word));
  assert.ok(index("uninstall") < index("am"));
  assert.equal(await readFile(join(h.root, "installed-key"), "utf8"), "CI key");
});

test("--run downloads the host CLI and runs it on the example", { skip: process.platform === "win32" }, async (t) => {
  const h = await harness(t);
  const result = await h.pr(["7", "--run", "hello"]);
  assert.equal(result.code, 0, result.stderr);
  const args = await readFile(join(h.root, "functor-args"), "utf8");
  assert.match(args, /^ -d .*examples[/\\]hello run native\n$/);
  assert.ok(!(await h.calls()).some((call) => call[0] === "adb"));
});
