import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import {
  mkdir,
  mkdtemp,
  readFile,
  rm,
  symlink,
  truncate,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);
const script = fileURLToPath(new URL("./pr-preview.mjs", import.meta.url));
const sha = "0123456789abcdef0123456789abcdef01234567";
const repository = "tommy-xr/functor";
const repositoryApiUrl = `https://api.github.com/repos/${repository}`;
const accountId = "a".repeat(32);
const zoneId = "b".repeat(32);
const certificatePackId = "c".repeat(32);
const domainCertificateId = "9fdf92c8-64c2-4a3d-b1af-e15304961145";
const packedCertificateId = domainCertificateId.replaceAll("-", "");
const previewHostname = "pr-690.preview.functor.games";
const headerPages = [
  "index.html",
  "sandbox.html",
  "ide.html",
  "docs/index.html",
  "manual/index.html",
];
const runtimeFiles = [
  "pkg/functor_runtime_web.js",
  "pkg/functor_runtime_web_bg.wasm",
  "pkg/functor_lang_wasm.js",
  "pkg/functor_lang_wasm_bg.wasm",
];
const badge =
  `<span class="version-badge" title="PR preview for commit ${sha}">` +
  `${sha.slice(0, 7)} · preview</span>`;

const temporaryDirectory = async (t, prefix) => {
  const directory = await mkdtemp(join(tmpdir(), prefix));
  t.after(() => rm(directory, { recursive: true, force: true }));
  return directory;
};

const makePreviewArtifact = async (t) => {
  const root = await temporaryDirectory(t, "functor-pr-preview-artifact-");
  for (const path of [...headerPages, ...runtimeFiles]) {
    const target = join(root, path);
    await mkdir(dirname(target), { recursive: true });
    await writeFile(
      target,
      path.endsWith(".html") ? `<!doctype html><body>${badge}</body>` : "fixture",
    );
  }
  return root;
};

const runPreview = async (args, env = {}) => {
  try {
    const result = await execFileAsync(process.execPath, [script, ...args], {
      env: { ...process.env, ...env },
      maxBuffer: 1024 * 1024,
    });
    return { code: 0, stdout: result.stdout, stderr: result.stderr };
  } catch (error) {
    return {
      code: typeof error.code === "number" ? error.code : 1,
      stdout: String(error.stdout ?? ""),
      stderr: String(error.stderr ?? error.message ?? error),
    };
  }
};

const validateArtifact = (directory, commit = sha) =>
  runPreview(["validate-artifact", "--dir", directory, "--sha", commit]);

const validArtifacts = () => ({
  total_count: 1,
  artifacts: [{ name: "pr-preview-site", expired: false, size_in_bytes: 1024 }],
});

const writeGitHubStub = async (
  t,
  run,
  pull,
  artifacts = validArtifacts(),
  associated = [],
) => {
  const root = await temporaryDirectory(t, "functor-pr-preview-github-");
  const stub = join(root, "fetch-stub.mjs");
  await writeFile(
    stub,
    `const run = ${JSON.stringify(run)};\n` +
      `const pull = ${JSON.stringify(pull)};\n` +
      `const artifacts = ${JSON.stringify(artifacts)};\n` +
      `const associated = ${JSON.stringify(associated)};\n` +
      `globalThis.fetch = async (input) => {\n` +
      `  const path = new URL(String(input)).pathname;\n` +
      `  const body = /\\/commits\\/[0-9a-f]{40}\\/pulls$/.test(path)\n` +
      `    ? associated\n` +
      `    : /\\/actions\\/runs\\/\\d+\\/artifacts$/.test(path)\n` +
      `    ? artifacts\n` +
      `    : /\\/actions\\/runs\\/\\d+$/.test(path) ? run : pull;\n` +
      `  return new Response(JSON.stringify(body), {\n` +
      `    status: 200, headers: { "Content-Type": "application/json" },\n` +
      `  });\n` +
      `};\n`,
  );
  return pathToFileURL(stub).href;
};

const writeCloudflareStub = async (
  t,
  {
    certificateInventoryFailures = 0,
    certificateInventoryMisses = 0,
    failDomainList = false,
    failDomainDetach = false,
    foreignDomain = false,
  } = {},
) => {
  const root = await temporaryDirectory(t, "functor-pr-preview-cloudflare-");
  const stub = join(root, "fetch-stub.mjs");
  const log = join(root, "requests.jsonl");
  await writeFile(log, "");
  await writeFile(
    stub,
    `import { appendFileSync } from "node:fs";\n` +
      `const log = ${JSON.stringify(log)};\n` +
      `const failDomainList = ${JSON.stringify(failDomainList)};\n` +
      `const failDomainDetach = ${JSON.stringify(failDomainDetach)};\n` +
      `const foreignDomain = ${JSON.stringify(foreignDomain)};\n` +
      `const certificateInventoryFailures = ${JSON.stringify(certificateInventoryFailures)};\n` +
      `const certificateInventoryMisses = ${JSON.stringify(certificateInventoryMisses)};\n` +
      `let certificateInventoryRequests = 0;\n` +
      `globalThis.setTimeout = (callback) => { callback(); return 0; };\n` +
      `const json = (body, status = 200) => new Response(JSON.stringify(body), {\n` +
      `  status, headers: { "Content-Type": "application/json" },\n` +
      `});\n` +
      `globalThis.fetch = async (input, init = {}) => {\n` +
      `  const url = new URL(String(input));\n` +
      `  const method = init.method ?? "GET";\n` +
      `  appendFileSync(log, JSON.stringify({ method, path: url.pathname + url.search }) + "\\n");\n` +
      `  if (method === "GET" && url.pathname.endsWith("/workers/domains")) {\n` +
      `    if (failDomainList) return json({ success: false, errors: [{ message: "denied" }] }, 403);\n` +
      `    return json({ success: true, result: [{\n` +
      `      id: "domain-id", hostname: ${JSON.stringify(previewHostname)},\n` +
      `      service: foreignDomain ? "production-worker" : "functor-pr-690",\n` +
      `      zone_id: ${JSON.stringify(zoneId)},\n` +
      `      cert_id: ${JSON.stringify(domainCertificateId)},\n` +
      `    }] });\n` +
      `  }\n` +
      `  if (method === "GET" && url.pathname.endsWith("/ssl/certificate_packs")) {\n` +
      `    certificateInventoryRequests += 1;\n` +
      `    if (certificateInventoryRequests <= certificateInventoryFailures) {\n` +
      `      return json({ success: false, errors: [{ message: "temporarily unavailable" }] }, 503);\n` +
      `    }\n` +
      `    if (certificateInventoryRequests <= certificateInventoryFailures + certificateInventoryMisses) {\n` +
      `      return json({ success: true, result: [], result_info: { total_pages: 1 } });\n` +
      `    }\n` +
      `    return json({ success: true, result: [{\n` +
      `      id: ${JSON.stringify(certificatePackId)}, status: "active",\n` +
      `      hosts: [${JSON.stringify(previewHostname)}],\n` +
      `      certificates: [{ id: ${JSON.stringify(packedCertificateId)}, hosts: [${JSON.stringify(previewHostname)}] }],\n` +
      `    }], result_info: { total_pages: 1 } });\n` +
      `  }\n` +
      `  if (method === "DELETE" && url.pathname.endsWith("/workers/domains/domain-id") && failDomainDetach) {\n` +
      `    return json({ success: false, errors: [{ message: "detach denied" }] }, 403);\n` +
      `  }\n` +
      `  if (method === "DELETE") return json({ success: true, result: null });\n` +
      `  return json({ success: false, errors: [{ message: "unexpected request" }] }, 500);\n` +
      `};\n`,
  );
  return { log, stub: pathToFileURL(stub).href };
};

const writeGitHubCommentStub = async (t, comments = []) => {
  const root = await temporaryDirectory(t, "functor-pr-preview-comments-");
  const stub = join(root, "fetch-stub.mjs");
  const log = join(root, "requests.jsonl");
  await writeFile(log, "");
  await writeFile(
    stub,
    `import { appendFileSync } from "node:fs";\n` +
      `const comments = ${JSON.stringify(comments)};\n` +
      `const log = ${JSON.stringify(log)};\n` +
      `const json = (body, status = 200) => new Response(JSON.stringify(body), {\n` +
      `  status, headers: { "Content-Type": "application/json" },\n` +
      `});\n` +
      `globalThis.fetch = async (input, init = {}) => {\n` +
      `  const url = new URL(String(input));\n` +
      `  const method = init.method ?? "GET";\n` +
      `  const body = init.body ? JSON.parse(init.body) : null;\n` +
      `  appendFileSync(log, JSON.stringify({ method, path: url.pathname, body }) + "\\n");\n` +
      `  if (method === "GET" && url.pathname.endsWith("/issues/690/comments")) return json(comments);\n` +
      `  if (method === "POST" && url.pathname.endsWith("/issues/690/comments")) return json({ id: 1 });\n` +
      `  if (method === "PATCH" && /\\/issues\\/comments\\/\\d+$/.test(url.pathname)) return json({ id: 1 });\n` +
      `  return json({ message: "unexpected request" }, 500);\n` +
      `};\n`,
  );
  return { log, stub: pathToFileURL(stub).href };
};

const validRun = () => ({
  name: "Build PR Preview",
  path: ".github/workflows/pr-preview-build.yml",
  event: "pull_request",
  status: "completed",
  conclusion: "success",
  actor: { login: "tommy-xr" },
  triggering_actor: { login: "tommy-xr" },
  head_repository: { full_name: repository },
  head_sha: sha,
  pull_requests: [{
    number: 690,
    base: { repo: { id: 800538382, name: "functor", url: repositoryApiUrl } },
  }],
});

const validPull = () => ({
  number: 690,
  user: { login: "tommy-xr" },
  head: { sha, repo: { full_name: repository } },
  base: { repo: { full_name: repository } },
  state: "open",
});

const authorizationEnvironment = (stub, output, summary) => ({
  GITHUB_TOKEN: "test-token",
  GITHUB_REPOSITORY: repository,
  GITHUB_OUTPUT: output,
  GITHUB_STEP_SUMMARY: summary,
  NODE_OPTIONS: [process.env.NODE_OPTIONS, `--import=${stub}`].filter(Boolean).join(" "),
});

const runAuthorization = async (
  t,
  {
    run = validRun(),
    pull = validPull(),
    artifacts = validArtifacts(),
    associated = [],
  } = {},
) => {
  const root = await temporaryDirectory(t, "functor-pr-preview-authorization-");
  const output = join(root, "output");
  const summary = join(root, "summary");
  const stub = await writeGitHubStub(t, run, pull, artifacts, associated);
  const result = await runPreview(
    ["authorize-run", "--run-id", "123"],
    authorizationEnvironment(stub, output, summary),
  );
  return { output, result, summary };
};

const cloudflareEnvironment = (stub) => ({
  CLOUDFLARE_API_TOKEN: "test-token",
  CLOUDFLARE_ACCOUNT_ID: accountId,
  NODE_OPTIONS: [process.env.NODE_OPTIONS, `--import=${stub}`].filter(Boolean).join(" "),
});

const runCleanup = (stub) =>
  runPreview(["cleanup", "--pr", "690"], cloudflareEnvironment(stub));

test("accepts a complete static artifact with exact commit badges", async (t) => {
  const artifact = await makePreviewArtifact(t);
  const result = await validateArtifact(artifact);
  assert.equal(result.code, 0, result.stderr);
  assert.match(result.stdout, /Validated 9 static preview assets/);
});

test("requires a full commit SHA", async () => {
  const result = await validateArtifact(".", "deadbeef");
  assert.notEqual(result.code, 0);
  assert.match(result.stderr, /full 40-character Git commit SHA/);
});

test("requires positive, safe PR numbers", async () => {
  const zero = await runPreview(["cleanup", "--pr", "0"]);
  assert.notEqual(zero.code, 0);
  assert.match(zero.stderr, /--pr must be a positive integer/);

  const unsafe = await runPreview(["cleanup", "--pr", "99999999999999999"]);
  assert.notEqual(unsafe.code, 0);
  assert.match(unsafe.stderr, /--pr is too large/);
});

test("rejects an incomplete artifact", async (t) => {
  const artifact = await makePreviewArtifact(t);
  await rm(join(artifact, "pkg/functor_lang_wasm.js"));
  const result = await validateArtifact(artifact);
  assert.notEqual(result.code, 0);
  assert.match(result.stderr, /missing pkg\/functor_lang_wasm\.js/);
});

for (const control of [".assetsignore", "_headers", "_redirects"]) {
  test(`rejects Cloudflare deployment control ${control}`, async (t) => {
    const artifact = await makePreviewArtifact(t);
    await writeFile(join(artifact, control), "/* fixture */");
    const result = await validateArtifact(artifact);
    assert.notEqual(result.code, 0);
    assert.match(result.stderr, /may not contain Cloudflare deployment controls/);
    assert.match(result.stderr, new RegExp(control.replace(".", "\\.")));
  });
}

test("rejects symbolic links", async (t) => {
  const artifact = await makePreviewArtifact(t);
  await symlink("index.html", join(artifact, "linked-index.html"));
  const result = await validateArtifact(artifact);
  assert.notEqual(result.code, 0);
  assert.match(result.stderr, /may not contain symbolic links/);
});

test("rejects files over Cloudflare's static-asset limit", async (t) => {
  const artifact = await makePreviewArtifact(t);
  const oversized = join(artifact, "oversized.bin");
  await writeFile(oversized, "");
  await truncate(oversized, 25 * 1024 * 1024 + 1);
  const result = await validateArtifact(artifact);
  assert.notEqual(result.code, 0);
  assert.match(result.stderr, /larger than Cloudflare's 25 MiB limit/);
});

test("rejects artifacts over the expanded-size limit", async (t) => {
  const artifact = await makePreviewArtifact(t);
  for (let index = 0; index < 5; index += 1) {
    const file = join(artifact, `expanded-${index}.bin`);
    await writeFile(file, "");
    await truncate(file, 21 * 1024 * 1024);
  }
  const result = await validateArtifact(artifact);
  assert.notEqual(result.code, 0);
  assert.match(result.stderr, /exceeds the 100 MiB expanded-size limit/);
});

test("rejects a spoofed commit badge", async (t) => {
  const artifact = await makePreviewArtifact(t);
  const sandbox = join(artifact, "sandbox.html");
  const html = await readFile(sandbox, "utf8");
  await writeFile(sandbox, html.replace(`${sha.slice(0, 7)} · preview`, "deadbee · preview"));
  const result = await validateArtifact(artifact);
  assert.notEqual(result.code, 0);
  assert.match(result.stderr, /sandbox\.html does not contain exactly one badge/);
});

test("rejects duplicate commit badges", async (t) => {
  const artifact = await makePreviewArtifact(t);
  const index = join(artifact, "index.html");
  await writeFile(index, `${await readFile(index, "utf8")}\n${badge}`);
  const result = await validateArtifact(artifact);
  assert.notEqual(result.code, 0);
  assert.match(result.stderr, /index\.html does not contain exactly one badge/);
});

test("authorizes the exact allowlisted run and current PR head", async (t) => {
  const { output, result } = await runAuthorization(t);
  assert.equal(result.code, 0, result.stderr);
  const values = await readFile(output, "utf8");
  assert.match(values, /^authorized=true$/m);
  assert.match(values, /^pr_number=690$/m);
  assert.match(values, new RegExp(`^head_sha=${sha}$`, "m"));
  assert.match(values, /^worker_name=functor-pr-690$/m);
});

test("resolves a PR through GitHub's associated-pulls fallback", async (t) => {
  const { output, result } = await runAuthorization(t, {
    run: { ...validRun(), pull_requests: [] },
    associated: [validPull()],
  });
  assert.equal(result.code, 0, result.stderr);
  assert.match(await readFile(output, "utf8"), /^authorized=true$/m);
});

for (const [label, options, reason] of [
  [
    "source actor",
    { run: { ...validRun(), actor: { login: "not-allowlisted" } } },
    /source actor is not allowlisted/,
  ],
  [
    "rerun actor",
    { run: { ...validRun(), triggering_actor: { login: "not-allowlisted" } } },
    /rerun actor is not allowlisted/,
  ],
  [
    "source repository",
    { run: { ...validRun(), head_repository: { full_name: "someone/fork" } } },
    /source branch is not in the trusted repository/,
  ],
  [
    "PR author",
    { pull: { ...validPull(), user: { login: "not-allowlisted" } } },
    /PR author is not allowlisted/,
  ],
  [
    "closed PR",
    { pull: { ...validPull(), state: "closed" } },
    /PR is no longer open/,
  ],
  [
    "stale head",
    { run: { ...validRun(), head_sha: "f".repeat(40) } },
    /source artifact is stale for the current PR head/,
  ],
]) {
  test(`rejects a non-authorized ${label}`, async (t) => {
    const { output, result, summary } = await runAuthorization(t, options);
    assert.equal(result.code, 0, result.stderr);
    assert.match(await readFile(output, "utf8"), /^authorized=false$/m);
    assert.match(await readFile(summary, "utf8"), reason);
  });
}

test("rejects duplicate source artifacts", async (t) => {
  const duplicate = { name: "pr-preview-site", expired: false, size_in_bytes: 1024 };
  const { output, result, summary } = await runAuthorization(t, {
    artifacts: {
      total_count: 2,
      artifacts: [duplicate, { ...duplicate, id: 2 }],
    },
  });
  assert.equal(result.code, 0, result.stderr);
  assert.match(await readFile(output, "utf8"), /^authorized=false$/m);
  assert.match(await readFile(summary, "utf8"), /must contain exactly one pr-preview-site artifact/);
});

test("rejects an oversized source artifact archive", async (t) => {
  const { output, result, summary } = await runAuthorization(t, {
    artifacts: {
      total_count: 1,
      artifacts: [{
        name: "pr-preview-site",
        expired: false,
        size_in_bytes: 50 * 1024 * 1024 + 1,
      }],
    },
  });
  assert.equal(result.code, 0, result.stderr);
  assert.match(await readFile(output, "utf8"), /^authorized=false$/m);
  assert.match(await readFile(summary, "utf8"), /archive exceeds the 50 MiB preview limit/);
});

test("records an authorization rejection in the step summary", async (t) => {
  const { output, result, summary } = await runAuthorization(t, {
    run: { ...validRun(), name: "Unexpected Workflow" },
  });
  assert.equal(result.code, 0, result.stderr);
  assert.match(await readFile(output, "utf8"), /^authorized=false$/m);
  assert.match(await readFile(summary, "utf8"), /unexpected source workflow/);
});

test("creates a ready sticky comment with the preview URL and commit", async (t) => {
  const { log, stub } = await writeGitHubCommentStub(t);
  const result = await runPreview(
    ["comment", "--pr", "690", "--sha", sha, "--state", "ready"],
    {
      GITHUB_TOKEN: "test-token",
      GITHUB_REPOSITORY: repository,
      NODE_OPTIONS: [process.env.NODE_OPTIONS, `--import=${stub}`].filter(Boolean).join(" "),
    },
  );
  assert.equal(result.code, 0, result.stderr);
  const requests = (await readFile(log, "utf8")).trim().split("\n").map(JSON.parse);
  const created = requests.find((request) => request.method === "POST");
  assert.equal(created.path, `/repos/${repository}/issues/690/comments`);
  assert.match(created.body.body, /<!-- functor-pr-preview -->/);
  assert.match(created.body.body, new RegExp(`Deployed commit.*${sha.slice(0, 7)}`));
  assert.match(created.body.body, /https:\/\/pr-690\.preview\.functor\.games\/sandbox\.html/);
  assert.match(created.body.body, new RegExp(`Full commit: <code>${sha}</code>`));
});

test("cleans up the exact certificate despite Cloudflare's ID formatting", async (t) => {
  const { log, stub } = await writeCloudflareStub(t);
  const result = await runCleanup(stub);
  assert.equal(result.code, 0, result.stderr);
  const requests = (await readFile(log, "utf8")).trim().split("\n").map(JSON.parse);
  const inventoryIndex = requests.findIndex(
    (request) => request.method === "GET" && request.path.includes("/ssl/certificate_packs?"),
  );
  const detachIndex = requests.findIndex(
    (request) => request.method === "DELETE" && request.path.endsWith("/workers/domains/domain-id"),
  );
  const certificateDeleteIndex = requests.findIndex(
    (request) => request.method === "DELETE" &&
      request.path === `/client/v4/zones/${zoneId}/ssl/certificate_packs/${certificatePackId}`,
  );
  assert.ok(inventoryIndex >= 0 && inventoryIndex < detachIndex);
  assert.ok(detachIndex < certificateDeleteIndex);
});

test("retries exact certificate inventory before detaching the domain", async (t) => {
  const { log, stub } = await writeCloudflareStub(t, { certificateInventoryMisses: 2 });
  const result = await runCleanup(stub);
  assert.equal(result.code, 0, result.stderr);
  const requests = (await readFile(log, "utf8")).trim().split("\n").map(JSON.parse);
  const inventoryIndexes = requests.flatMap((request, index) =>
    request.method === "GET" && request.path.includes("/ssl/certificate_packs?") ? [index] : [],
  );
  const detachIndex = requests.findIndex(
    (request) => request.method === "DELETE" && request.path.endsWith("/workers/domains/domain-id"),
  );
  assert.equal(inventoryIndexes.length, 3);
  assert.ok(inventoryIndexes.every((index) => index < detachIndex));
  assert.match(
    await readFile(log, "utf8"),
    new RegExp(`"method":"DELETE","path":"/client/v4/zones/${zoneId}/ssl/certificate_packs/${certificatePackId}"`),
  );
});

test("retries a transient certificate inventory error before detaching", async (t) => {
  const { log, stub } = await writeCloudflareStub(t, { certificateInventoryFailures: 1 });
  const result = await runCleanup(stub);
  assert.equal(result.code, 0, result.stderr);
  const requests = (await readFile(log, "utf8")).trim().split("\n").map(JSON.parse);
  const inventoryIndexes = requests.flatMap((request, index) =>
    request.method === "GET" && request.path.includes("/ssl/certificate_packs?") ? [index] : [],
  );
  const detachIndex = requests.findIndex(
    (request) => request.method === "DELETE" && request.path.endsWith("/workers/domains/domain-id"),
  );
  assert.equal(inventoryIndexes.length, 2);
  assert.ok(inventoryIndexes.every((index) => index < detachIndex));
  assert.match(
    await readFile(log, "utf8"),
    new RegExp(`"method":"DELETE","path":"/client/v4/zones/${zoneId}/ssl/certificate_packs/${certificatePackId}"`),
  );
});

test("does not guess a certificate pack when the exact ID stays unavailable", async (t) => {
  const { log, stub } = await writeCloudflareStub(t, { certificateInventoryMisses: 99 });
  const result = await runCleanup(stub);
  assert.equal(result.code, 0, result.stderr);
  assert.match(result.stderr, /leaving it for manual review/);
  const requests = await readFile(log, "utf8");
  assert.match(requests, /"method":"DELETE","path":"\/client\/v4\/accounts\//);
  assert.doesNotMatch(
    requests,
    new RegExp(`"method":"DELETE","path":"/client/v4/zones/${zoneId}/ssl/certificate_packs/`),
  );
});

test("does not report a recovered inventory error as the final outcome", async (t) => {
  const { stub } = await writeCloudflareStub(t, {
    certificateInventoryFailures: 1,
    certificateInventoryMisses: 99,
  });
  const result = await runCleanup(stub);
  assert.equal(result.code, 0, result.stderr);
  assert.match(result.stderr, /leaving it for manual review/);
  assert.doesNotMatch(result.stderr, /inventory warning after retries/i);
});

test("refuses to replace a preview domain owned by another Worker", async (t) => {
  const { log, stub } = await writeCloudflareStub(t, { foreignDomain: true });
  const result = await runPreview(
    ["attach", "--pr", "690"],
    cloudflareEnvironment(stub),
  );
  assert.notEqual(result.code, 0);
  assert.match(result.stderr, /belongs to another Worker/);
  assert.doesNotMatch(await readFile(log, "utf8"), /"method":"PUT"/);
});

test("does not delete the Worker when domain inventory fails", async (t) => {
  const { log, stub } = await writeCloudflareStub(t, { failDomainList: true });
  const result = await runCleanup(stub);
  assert.notEqual(result.code, 0);
  assert.match(result.stderr, /preview cleanup incomplete: list domain/);
  assert.doesNotMatch(await readFile(log, "utf8"), /workers\/scripts\/functor-pr-690/);
});

test("does not delete the Worker when domain detach fails", async (t) => {
  const { log, stub } = await writeCloudflareStub(t, { failDomainDetach: true });
  const result = await runCleanup(stub);
  assert.notEqual(result.code, 0);
  assert.match(result.stderr, /preview cleanup incomplete: detach domain/);
  const requests = await readFile(log, "utf8");
  assert.doesNotMatch(requests, /workers\/scripts\/functor-pr-690/);
  assert.doesNotMatch(
    requests,
    new RegExp(`"method":"DELETE","path":"/client/v4/zones/${zoneId}/ssl/certificate_packs/`),
  );
});
