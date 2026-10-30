import assert from "node:assert/strict";
import test from "node:test";

import { render } from "./pr-builds-comment.mjs";

const repository = "tommy-xr/functor";
const sha = "0123456789abcdef0123456789abcdef01234567";
const oldSha = "fedcba9876543210fedcba9876543210fedcba98";
const runUrl = (id) => `https://github.com/${repository}/actions/runs/${id}`;
const artifactUrl = (run, id) => `${runUrl(run)}/artifacts/${id}`;

const run = (id, name, overrides = {}) => ({
  id,
  name,
  head_sha: sha,
  status: "completed",
  conclusion: "success",
  html_url: runUrl(id),
  ...overrides,
});
const artifact = (id, name, runId) => ({ id, name, expired: false, workflow_run: { id: runId } });

const cliRun = run(100, "Build CLI");
const apkRun = run(200, "Build Android");
const cliArtifacts = [
  artifact(1, `functor-cli-macos-arm64-${sha}`, 100),
  artifact(2, `functor-cli-linux-x64-${sha}`, 100),
];
const apkArtifact = artifact(3, `functor-apk-${sha}`, 200);

const body = (runs, artifacts, headSha = sha) => render({ repository, pr: 42, sha: headSha, runs, artifacts });

test("all builds succeeded: every target links its artifact", () => {
  const text = body([cliRun, apkRun], [...cliArtifacts, apkArtifact]);
  assert.ok(text.startsWith("<!-- functor-pr-builds -->\n"));
  assert.ok(text.includes(`[\`0123456\`](https://github.com/${repository}/commit/${sha})`));
  assert.ok(text.includes(`- [CLI macOS arm64](${artifactUrl(100, 1)})`));
  assert.ok(text.includes(`- [CLI Linux x64](${artifactUrl(100, 2)})`));
  assert.ok(text.includes(`- [Quest APK](${artifactUrl(200, 3)})`));
  assert.ok(text.includes("`npm run pr -- 42`"));
  assert.ok(text.includes("`f --pr 42 -d <example> run vr`"));
  assert.ok(text.includes("GitHub login"));
  assert.ok(!text.includes("building…"));
});

test("a workflow still running, or not started, is building", () => {
  const running = body([cliRun, run(200, "Build Android", { status: "in_progress", conclusion: null })], cliArtifacts);
  assert.ok(running.includes(`- [CLI Linux x64](${artifactUrl(100, 2)})`));
  assert.ok(running.includes("- Quest APK: building…"));

  const notStarted = body([cliRun], cliArtifacts);
  assert.ok(notStarted.includes("- Quest APK: building…"));
});

test("a failed workflow links its run", () => {
  const text = body([cliRun, run(200, "Build Android", { conclusion: "failure" })], cliArtifacts);
  assert.ok(text.includes(`- Quest APK: ✗ [failure](${runUrl(200)})`));
});

test("runs and artifacts for another SHA are ignored", () => {
  const stale = [
    run(90, "Build CLI", { head_sha: oldSha }),
    run(91, "Build Android", { head_sha: oldSha }),
  ];
  const staleArtifacts = [
    artifact(7, `functor-cli-macos-arm64-${oldSha}`, 90),
    artifact(8, `functor-apk-${oldSha}`, 91),
    // Same-named artifact attributed to a stale run: not this head's build.
    artifact(9, `functor-cli-linux-x64-${sha}`, 90),
  ];
  const text = body(stale, staleArtifacts);
  assert.equal(text.match(/building…/g)?.length, 3);
  assert.ok(!text.includes(oldSha.slice(0, 7)));
  assert.ok(!text.includes("/artifacts/"));
});

test("a missing platform shows its own state", () => {
  // macOS failed under fail-fast: false; Linux still uploaded.
  const partial = body(
    [run(100, "Build CLI", { conclusion: "failure" }), apkRun],
    [cliArtifacts[1], apkArtifact],
  );
  assert.ok(partial.includes(`- CLI macOS arm64: ✗ [failure](${runUrl(100)})`));
  assert.ok(partial.includes(`- [CLI Linux x64](${artifactUrl(100, 2)})`));

  // A successful run whose artifact expired.
  const expired = body([cliRun, apkRun], [...cliArtifacts, { ...apkArtifact, expired: true }]);
  assert.ok(expired.includes(`- Quest APK: missing ([run](${runUrl(200)}))`));
});

test("a newer run supersedes an older failure for the same SHA", () => {
  const text = body(
    [run(200, "Build Android", { conclusion: "failure" }), run(201, "Build Android", { status: "queued", conclusion: null }), cliRun],
    cliArtifacts,
  );
  assert.ok(text.includes("- Quest APK: building…"));
});
