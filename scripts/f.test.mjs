import assert from "node:assert/strict";
import { chmod, copyFile, mkdir, mkdtemp, readdir, readFile, realpath, rm, stat, utimes, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { localBinary, parseArgs, wantsVr } from "./f.mjs";
import { cliPlatform } from "./pr-builds.mjs";
import { harness as fakeHarness, sha } from "./pr-test-fakes.mjs";

const script = fileURLToPath(new URL("./f.mjs", import.meta.url));
const harness = (t) => fakeHarness(t, script);
// PR mode needs a platform CI builds a CLI for.
const prMode = { skip: !cliPlatform() && "CI builds no CLI for this host" };

test("parses --pr anywhere and passes the rest through", () => {
  assert.deepEqual(parseArgs(["-d", "ex", "--pr", "836", "run", "vr", "--entry", "server"]), {
    help: false, apk: true, forceApk: false, allowUninstall: false, pr: "836",
    args: ["-d", "ex", "run", "vr", "--entry", "server"],
  });
  const flags = parseArgs(["--pr=12", "--force-apk", "--allow-uninstall", "test"]);
  assert.equal(flags.pr, "12");
  assert.equal(flags.forceApk, true);
  assert.equal(flags.allowUninstall, true);
  assert.deepEqual(flags.args, ["test"]);
  assert.equal(parseArgs(["--no-apk", "run", "vr"]).apk, false); // local mode too
  // From `--` on, everything is functor's, even `--pr`.
  assert.deepEqual(parseArgs(["--pr", "5", "run", "--", "--pr", "9"]).args, ["run", "--", "--pr", "9"]);
  assert.deepEqual(parseArgs(["-d", "ex", "test"]), {
    help: false, apk: true, forceApk: false, allowUninstall: false, args: ["-d", "ex", "test"],
  });
  assert.equal(parseArgs(["--help"]).help, true);
  assert.equal(parseArgs(["--pr", "5", "--help"]).help, false); // the PR CLI's help
  assert.throws(() => parseArgs(["--pr"]), /--pr requires a PR number/);
  assert.throws(() => parseArgs(["--pr", "abc"]), /--pr requires a PR number/);
  assert.throws(() => parseArgs(["--force-apk", "run", "vr"]), /need --pr/);
  assert.throws(() => parseArgs(["--pr", "5", "--no-apk", "--force-apk"]), /conflict/);
});

test("detects a headset push", () => {
  assert.ok(wantsVr(["-d", "ex", "run", "vr"]));
  assert.ok(wantsVr(["develop", "vr", "--entry", "server"]));
  assert.ok(!wantsVr(["-d", "ex", "run", "native"]));
  assert.ok(wantsVr(["run", "--entry", "server", "vr"]));
  assert.ok(wantsVr(["--mount", "a=b", "develop", "--no-debug", "vr"]));
  assert.ok(!wantsVr(["vr"]));
  assert.ok(!wantsVr(["-d", "run", "test", "vr"])); // `run` is -d's value
  assert.ok(!wantsVr(["run", "vr", "--help"]));
  assert.ok(!wantsVr(["test", "--", "run", "vr"]));
});

test("maps hosts to CI platforms", () => {
  assert.equal(cliPlatform("darwin", "arm64"), "macos-arm64");
  assert.equal(cliPlatform("linux", "x64"), "linux-x64");
  assert.equal(cliPlatform("darwin", "x64"), undefined);
  assert.equal(cliPlatform("win32", "x64"), undefined);
});

test("picks the newer local build, or explains how to make one", async (t) => {
  const root = await mkdtemp(join(tmpdir(), "functor-f-test-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  assert.throws(() => localBinary(root), /no functor build in .*target; build one with `npm run build:cli`/);

  const build = async (profile, seconds) => {
    await mkdir(join(root, "target", profile), { recursive: true });
    const path = join(root, "target", profile, "functor");
    await writeFile(path, "");
    await utimes(path, seconds, seconds);
    return path;
  };
  const debug = await build("debug", 2000);
  assert.equal(localBinary(root), debug);
  const release = await build("release", 1000);
  assert.equal(localBinary(root), debug);
  await utimes(release, 3000, 3000);
  assert.equal(localBinary(root), release);
});

test("downloads a PR's CLI once, then runs it from the cache in the caller's cwd", prMode, async (t) => {
  const h = await harness(t);
  const cwd = await mkdtemp(join(tmpdir(), "functor-f-cwd-"));
  t.after(() => rm(cwd, { recursive: true, force: true }));
  const cache = join(cwd, "cache"); // given relative to the caller's cwd
  const args = ["-d", "examples/x", "--pr", "7", "--entry", "server", "test", "--", "--fixed-time", "1"];

  for (const _ of [1, 2]) {
    const result = await h.run(args, { FUNCTOR_PR_CACHE: "cache" }, cwd);
    assert.equal(result.code, 0, result.stderr);
    assert.equal(result.stdout, ""); // progress stays off functor's stdout
    assert.match(result.stderr, /using PR #7 @ 0123456\n/);
    assert.equal(
      await readFile(join(h.root, "functor-args"), "utf8"),
      " -d examples/x --entry server test -- --fixed-time 1\n",
    );
    assert.equal((await readFile(join(h.root, "functor-cwd"), "utf8")).trim(), await realpath(cwd));
  }
  const calls = await h.calls();
  assert.equal(calls.filter((call) => call[1] === "run" && call[2] === "download").length, 1);
  assert.ok(!calls.some((call) => call[0] === "adb"));
  const cli = join(cache, sha, `functor-cli-${cliPlatform()}-${sha}`, "functor");
  assert.equal((await stat(cli)).mode & 0o111, 0o111);
});

test("an interrupted download leaves no cache entry", prMode, async (t) => {
  const h = await harness(t);
  const cache = join(h.root, "cache");
  const failed = await h.run(["--pr", "7", "--version"], { FUNCTOR_PR_CACHE: cache, FAKE_DOWNLOAD_FAIL: "1" });
  assert.equal(failed.code, 1);
  assert.deepEqual(await readdir(join(cache, sha)), []);
  assert.ok(!(await h.calls()).some((call) => call[0] === "functor"));

  const retried = await h.run(["--pr", "7", "--version"], { FUNCTOR_PR_CACHE: cache });
  assert.equal(retried.code, 0, retried.stderr);
});

test("refuses a missing, running or failed build with its link", prMode, async (t) => {
  for (const [status, message] of [
    ["none", "no build-cli.yml run for"],
    ["in_progress", "still in_progress: https://runs/42"],
    ["failure", "ended failure without functor-cli-"],
  ]) {
    const h = await harness(t);
    const result = await h.run(["--pr", "7", "--version"], { FAKE_RUN_STATUS: status });
    assert.equal(result.code, 1, status);
    assert.ok(result.stderr.includes(message), result.stderr);
    assert.ok(!(await h.calls()).some((call) => call[0] === "functor"), status);
  }
});

test("passes the binary's exit code and terminating signal through", prMode, async (t) => {
  const h = await harness(t);
  assert.equal((await h.run(["--pr", "7", "test"], { FAKE_EXIT: "3" })).code, 3);
  assert.equal((await h.run(["--pr", "7", "test"], { FAKE_SIGNAL: "TERM" })).signal, "SIGTERM");

  // A SIGINT sent to f alone reaches the binary.
  const waiting = h.run(["--pr", "7", "test"], { FAKE_WAIT: "1" });
  while (!(await stat(join(h.root, "ready")).catch(() => false))) await new Promise((r) => setTimeout(r, 20));
  waiting.child.kill("SIGINT");
  assert.equal((await waiting).code, 130);
  assert.equal(await readFile(join(h.root, "got"), "utf8"), "INT\n");
});

// A fake headset runner answering `GET /` and `GET /state`; returns the env
// pointing f at it.
async function fakeRunner(t, protocol, build) {
  const server = createServer((request, response) => {
    const body = request.url === "/" ? { protocol_version: protocol } : request.url === "/state" ? { build } : {};
    response.setHeader("content-type", "application/json");
    response.end(JSON.stringify(body));
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  t.after(() => server.close());
  return { FUNCTOR_VR_HOST_PORT: String(server.address().port) };
}

const record = (h) => join(h.root, ".functor", "apk-installed", "2G0YC5ZF87037Q");
const installed = async (h) => (await h.calls()).some((call) => call[0] === "adb" && call.includes("install"));
const vrArgs = ["--pr", "7", "-d", "ex", "run", "vr"];

test("run vr downloads the CLI, then the APK, before any device call; installs an unknown APK", prMode, async (t) => {
  const h = await harness(t);
  const result = await h.run(vrArgs);
  assert.equal(result.code, 0, result.stderr);
  assert.match(result.stderr, /installing PR #7 @ 0123456's APK \(runner unreachable, no install recorded; --no-apk skips\)/);
  assert.equal(await readFile(join(h.root, "installed-key"), "utf8"), "CI key");
  assert.equal(await readFile(record(h), "utf8"), `${sha}\n`);
  assert.equal(await readFile(join(h.root, "functor-args"), "utf8"), "2G0YC5ZF87037Q -d ex run vr\n");
  const calls = await h.calls();
  const index = (match) => calls.findIndex(match);
  const download = (name) => index((call) => call[2] === "download" && call.some((a) => a.startsWith(name)));
  assert.ok(download("functor-cli-") >= 0 && download("functor-cli-") < download("functor-apk-"));
  assert.ok(download("functor-apk-") < index((call) => call[0] === "adb"));
  assert.ok(index((call) => call.includes("install")) < index((call) => call[0] === "functor"));
});

test("run vr skips the install when the headset already runs the PR's APK", prMode, async (t) => {
  // Its runner reports the PR's build label.
  const reported = await harness(t);
  const result = await reported.run(vrArgs, await fakeRunner(t, 25, "pr | 0123456"));
  assert.equal(result.code, 0, result.stderr);
  assert.match(result.stderr, /headset already runs PR #7 @ 0123456 \(its runner reports `pr \| 0123456`\) — skipping install/);
  assert.ok(!(await installed(reported)));
  assert.equal(await readFile(join(reported.root, "functor-args"), "utf8"), "2G0YC5ZF87037Q -d ex run vr\n");

  // Its runner is down, but the last install recorded it.
  const recorded = await harness(t);
  await mkdir(dirname(record(recorded)), { recursive: true });
  await writeFile(record(recorded), `${sha}\n`);
  const fallback = await recorded.run(vrArgs);
  assert.equal(fallback.code, 0, fallback.stderr);
  assert.match(fallback.stderr, /already runs PR #7 @ 0123456 \(runner unreachable; last recorded install: 0123456\)/);
  assert.ok(!(await installed(recorded)));
});

test("run vr installs over another build, and --force-apk over a matching one", prMode, async (t) => {
  // The runner's report outranks a (stale) matching record.
  const other = await harness(t);
  await mkdir(dirname(record(other)), { recursive: true });
  await writeFile(record(other), `${sha}\n`);
  const result = await other.run(vrArgs, await fakeRunner(t, 25, "pr | 1111111"));
  assert.equal(result.code, 0, result.stderr);
  assert.match(result.stderr, /installing PR #7 @ 0123456's APK \(its runner reports `pr \| 1111111`/);
  assert.ok(await installed(other));

  const forced = await harness(t);
  const again = await forced.run([...vrArgs, "--force-apk"], await fakeRunner(t, 25, "pr | 0123456"));
  assert.equal(again.code, 0, again.stderr);
  assert.match(again.stderr, /installing PR #7 @ 0123456's APK \(--force-apk/);
  assert.ok(await installed(forced));
});

// f in a stand-in checkout: a fake local release build, on debug protocol v25.
async function localCheckout(t) {
  const root = await mkdtemp(join(tmpdir(), "functor-f-checkout-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  await mkdir(join(root, "scripts"));
  for (const file of ["f.mjs", "pr-builds.mjs"]) {
    await copyFile(fileURLToPath(new URL(`./${file}`, import.meta.url)), join(root, "scripts", file));
  }
  await mkdir(join(root, "runtime/functor-runtime-common/src"), { recursive: true });
  await writeFile(join(root, "runtime/functor-runtime-common/src/debug_protocol.rs"),
    "pub const DEBUG_PROTOCOL_VERSION: u32 = 25;\n");
  const h = await fakeHarness(t, join(root, "scripts", "f.mjs"));
  await mkdir(join(root, "target", "release"), { recursive: true });
  await copyFile(join(h.root, "fake-functor"), join(root, "target", "release", "functor"));
  await chmod(join(root, "target", "release", "functor"), 0o755);
  return h;
}

test("local run vr refuses a headset runner on another protocol", async (t) => {
  const h = await localCheckout(t);
  const result = await h.run(["-d", "ex", "run", "vr"], await fakeRunner(t, 24, "dev"));
  assert.equal(result.code, 1);
  assert.match(result.stderr, /speaks debug protocol v24, this checkout's CLI v25/);
  assert.match(result.stderr, /npm run build:oculus:apk && adb install -r .*f --pr <n>/s);
  assert.ok(!(await h.calls()).some((call) => call[0] === "functor"));

  // --no-apk skips the check.
  assert.equal((await h.run(["--no-apk", "-d", "ex", "run", "vr"], await fakeRunner(t, 24, "dev"))).code, 0);
});

test("local run vr warns on a PR build or an unreachable runner, then runs", async (t) => {
  const h = await localCheckout(t);
  const pr = await h.run(["-d", "ex", "run", "vr"], await fakeRunner(t, 25, "pr | 0123456"));
  assert.equal(pr.code, 0, pr.stderr);
  assert.match(pr.stderr, /headset runs `pr \| 0123456`, not a local build \(same protocol v25\); continuing/);
  assert.equal(await readFile(join(h.root, "functor-args"), "utf8"), "2G0YC5ZF87037Q -d ex run vr\n");

  const down = await h.run(["-d", "ex", "run", "vr"]);
  assert.equal(down.code, 0, down.stderr);
  assert.match(down.stderr, /runner isn't reachable, so it's unchecked against this CLI; continuing/);

  const same = await h.run(["-d", "ex", "run", "vr"], await fakeRunner(t, 25, "dev"));
  assert.equal(same.code, 0, same.stderr);
  assert.doesNotMatch(same.stderr, /continuing/);
});

test("run vr --no-apk leaves the headset alone", prMode, async (t) => {
  const skip = await harness(t);
  const skipped = await skip.run(["--pr", "7", "--no-apk", "-d", "ex", "run", "vr"]);
  assert.equal(skipped.code, 0, skipped.stderr);
  assert.match(skipped.stderr, /--no-apk: pushing to the headset's installed APK as is/);
  assert.ok(!(await skip.calls()).some((call) => call[0] === "adb" || call.some((a) => a.startsWith?.("functor-apk"))));
});
