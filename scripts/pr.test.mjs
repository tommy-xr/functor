import assert from "node:assert/strict";
import { readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { harness as fakeHarness } from "./pr-test-fakes.mjs";

const script = fileURLToPath(new URL("./pr.mjs", import.meta.url));
const harness = (t) => fakeHarness(t, script);

test("refuses a missing, running or failed build before touching the device", async (t) => {
  for (const [status, message] of [
    ["none", "gh workflow run build-android.yml --ref feat/x"],
    ["in_progress", "still in_progress: https://runs/42"],
    ["failure", "ended failure without functor-apk-"],
  ]) {
    const h = await harness(t);
    const result = await h.run(["7", "--apk"], { FAKE_RUN_STATUS: status });
    assert.equal(result.code, 1, status);
    assert.ok(result.stderr.includes(message), result.stderr);
    assert.ok(!(await h.calls()).some((call) => call[0] === "adb"), status);
  }
});

test("uninstalls a differently signed runner only with --allow-uninstall, reusing the cache", async (t) => {
  const h = await harness(t);
  await writeFile(join(h.root, "installed-key"), "local key");

  const refused = await h.run(["7", "--apk"]);
  assert.equal(refused.code, 1);
  assert.match(refused.stderr, /signed with another key.*--allow-uninstall/);
  let calls = await h.calls();
  assert.ok(!calls.some((call) => call.includes("uninstall") || call.includes("am")));
  assert.equal(await readFile(join(h.root, "installed-key"), "utf8"), "local key");

  const allowed = await h.run(["7", "--apk", "--allow-uninstall"]);
  assert.equal(allowed.code, 0, allowed.stderr);
  calls = await h.calls();
  assert.equal(calls.filter((call) => call[1] === "run" && call[2] === "download").length, 1);
  const index = (word) => calls.findIndex((call) => call.includes(word));
  assert.ok(index("uninstall") < index("am"));
  assert.equal(await readFile(join(h.root, "installed-key"), "utf8"), "CI key");
});

test("--run downloads the host CLI and runs it on the example", async (t) => {
  const h = await harness(t);
  const result = await h.run(["7", "--run", "hello"]);
  assert.equal(result.code, 0, result.stderr);
  const args = await readFile(join(h.root, "functor-args"), "utf8");
  assert.match(args, /^ -d .*examples[/\\]hello run native\n$/);
  assert.ok(!(await h.calls()).some((call) => call[0] === "adb"));
});

test("refuses a fork PR and an ambiguous device list", async (t) => {
  const fork = await harness(t);
  const forked = await fork.run(["7", "--apk"], { FAKE_FORK: "1" });
  assert.equal(forked.code, 1);
  assert.match(forked.stderr, /from a fork/);
  assert.ok(!(await fork.calls()).some((call) => call[1] === "run"));

  const two = await harness(t);
  const ambiguous = await two.run(["7", "--apk"], { FAKE_DISTINCT: "1" });
  assert.equal(ambiguous.code, 1);
  assert.match(ambiguous.stderr, /set ANDROID_SERIAL/);
  assert.ok(!(await two.calls()).some((call) => call.includes("install")));
});

test("--apk --run installs, then runs vr on the chosen headset", async (t) => {
  const h = await harness(t);
  const result = await h.run(["7", "--apk", "--run", "hello"]);
  assert.equal(result.code, 0, result.stderr);
  const args = await readFile(join(h.root, "functor-args"), "utf8");
  assert.match(args, /^2G0YC5ZF87037Q -d .*examples[/\\]hello run vr\n$/);
  assert.equal(await readFile(join(h.root, "installed-key"), "utf8"), "CI key");
});
