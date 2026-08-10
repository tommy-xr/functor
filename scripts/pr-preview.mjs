// Trusted lifecycle tooling for Functor's per-PR static-site previews.
//
// This file is always run from the default branch by the secret-bearing
// workflow_run / pull_request_target workflows. The PR build artifact is data:
// validate it, upload it as static assets, but never execute anything from it.

// Commands:
//   authorize-run --run-id N
//   validate-artifact --dir site/dist --sha FULL_SHA
//   attach --pr N
//   cleanup --pr N
//   comment --pr N --sha FULL_SHA --state ready|closed|cleanup-failed

// Required environment is command-specific:
//   GITHUB_TOKEN, GITHUB_REPOSITORY, GITHUB_OUTPUT
//   GITHUB_STEP_SUMMARY (optional; records authorization rejections)
//   CLOUDFLARE_API_TOKEN, CLOUDFLARE_ACCOUNT_ID

// Keep the allowlist and namespace here, in trusted default-branch code. A PR
// that edits its own build workflow or a copy of this script cannot widen it.

import { appendFile, lstat, readFile, readdir } from "node:fs/promises";
import { resolve } from "node:path";

const ALLOWED_AUTHOR = "tommy-xr";
const ALLOWED_REPOSITORY = "tommy-xr/functor";
const ALLOWED_REPOSITORY_API_URL = `https://api.github.com/repos/${ALLOWED_REPOSITORY}`;
const BUILD_WORKFLOW_NAME = "Build PR Preview";
const BUILD_WORKFLOW_PATH = ".github/workflows/pr-preview-build.yml";
const ARTIFACT_NAME = "pr-preview-site";
const PREVIEW_ZONE = "functor.games";
const COMMENT_MARKER = "<!-- functor-pr-preview -->";
const MAX_ASSET_FILES = 20_000;
const MAX_ASSET_BYTES = 25 * 1024 * 1024;
const MAX_ASSET_TOTAL_BYTES = 100 * 1024 * 1024;
const MAX_ARTIFACT_ARCHIVE_BYTES = 50 * 1024 * 1024;
const FORBIDDEN_ASSET_FILES = new Set([".assetsignore", "_headers", "_redirects"]);
const CERTIFICATE_PACK_LOOKUP_DELAYS_MS = [0, 1_000, 3_000, 7_000];

const [, , command, ...rawArgs] = process.argv;

const flags = new Map();
for (let index = 0; index < rawArgs.length; index += 2) {
  const name = rawArgs[index];
  const value = rawArgs[index + 1];
  if (!name?.startsWith("--") || value === undefined) {
    throw new Error(`expected --name value arguments; got ${JSON.stringify(rawArgs)}`);
  }
  flags.set(name.slice(2), value);
}

const flag = (name) => {
  const value = flags.get(name);
  if (value === undefined) throw new Error(`missing --${name}`);
  return value;
};

const positiveInteger = (name) => {
  const raw = flag(name);
  if (!/^[1-9]\d*$/.test(raw)) throw new Error(`--${name} must be a positive integer`);
  const value = Number(raw);
  if (!Number.isSafeInteger(value)) throw new Error(`--${name} is too large`);
  return value;
};

const fullSha = (name = "sha") => {
  const raw = flag(name);
  if (!/^[0-9a-f]{40}$/i.test(raw)) {
    throw new Error(`--${name} must be a full 40-character Git commit SHA`);
  }
  return raw.toLowerCase();
};

const requiredEnv = (name) => {
  const value = process.env[name];
  if (!value) throw new Error(`${name} is required`);
  return value;
};

const preview = (pr) => ({
  worker: `functor-pr-${pr}`,
  hostname: `pr-${pr}.preview.${PREVIEW_ZONE}`,
  url: `https://pr-${pr}.preview.${PREVIEW_ZONE}`,
});

const safeLog = (value) => String(value).replace(/[\r\n%]/g, " ");

const writeOutput = async (values) => {
  const output = requiredEnv("GITHUB_OUTPUT");
  const lines = Object.entries(values).map(([key, value]) => `${key}=${value}\n`).join("");
  await appendFile(output, lines);
};

const githubRepository = () => {
  const repository = requiredEnv("GITHUB_REPOSITORY");
  if (repository !== ALLOWED_REPOSITORY) {
    throw new Error(`refusing to manage previews for repository ${safeLog(repository)}`);
  }
  return repository;
};

// Workflow-run payloads embed a minimal repository object with only `url`,
// while pull and repository endpoints include `full_name`. Accept either exact
// representation without falling back to a name-only comparison.
const isAllowedRepository = (repository) =>
  repository?.full_name === ALLOWED_REPOSITORY ||
  repository?.url === ALLOWED_REPOSITORY_API_URL;

const githubRequest = async (path, init = {}) => {
  const response = await fetch(`https://api.github.com${path}`, {
    ...init,
    headers: {
      Accept: "application/vnd.github+json",
      Authorization: `Bearer ${requiredEnv("GITHUB_TOKEN")}`,
      "X-GitHub-Api-Version": "2026-03-10",
      "User-Agent": "functor-pr-preview",
      ...(init.body ? { "Content-Type": "application/json" } : {}),
      ...init.headers,
    },
  });
  const text = await response.text();
  let data = null;
  if (text) {
    try {
      data = JSON.parse(text);
    } catch {
      throw new Error(`GitHub API returned non-JSON (${response.status})`);
    }
  }
  if (!response.ok) {
    throw new Error(`GitHub API ${response.status}: ${safeLog(data?.message ?? response.statusText)}`);
  }
  return data;
};

const authorizeRun = async () => {
  const runId = positiveInteger("run-id");
  const repository = githubRepository();
  const run = await githubRequest(`/repos/${repository}/actions/runs/${runId}`);

  let pullNumbers = [...new Set(
    (run.pull_requests ?? [])
      .filter((pull) => isAllowedRepository(pull.base?.repo))
      .map((pull) => pull.number),
  )];

  // GitHub occasionally omits pull_requests from historical workflow-run
  // payloads. The associated-pulls endpoint is a safe fallback, but ambiguity
  // is rejected rather than guessing which PR owns the deployment URL.
  if (pullNumbers.length === 0 && /^[0-9a-f]{40}$/i.test(run.head_sha ?? "")) {
    const associated = await githubRequest(
      `/repos/${repository}/commits/${run.head_sha}/pulls?per_page=100`,
    );
    pullNumbers = [...new Set(
      associated
        .filter(
          (pull) =>
            isAllowedRepository(pull.base?.repo) &&
            pull.head?.sha?.toLowerCase() === run.head_sha.toLowerCase(),
        )
        .map((pull) => pull.number),
    )];
  }

  const reject = async (reason) => {
    const safeReason = safeLog(reason);
    console.log(`PR preview skipped: ${safeReason}`);
    const summary = process.env.GITHUB_STEP_SUMMARY;
    if (summary) {
      await appendFile(summary, `### PR preview skipped\n\n${safeReason}\n`);
    }
    await writeOutput({ authorized: "false" });
  };

  if (run.name !== BUILD_WORKFLOW_NAME) return reject("unexpected source workflow");
  if (run.path !== BUILD_WORKFLOW_PATH) return reject("unexpected source workflow path");
  if (run.event !== "pull_request") return reject("source event was not pull_request");
  if (run.status !== "completed" || run.conclusion !== "success") {
    return reject("source build did not complete successfully");
  }
  if (run.actor?.login !== ALLOWED_AUTHOR) return reject("source actor is not allowlisted");
  if (run.triggering_actor?.login && run.triggering_actor.login !== ALLOWED_AUTHOR) {
    return reject("rerun actor is not allowlisted");
  }
  if (!isAllowedRepository(run.head_repository)) {
    return reject("source branch is not in the trusted repository");
  }
  if (pullNumbers.length !== 1) return reject("source run does not identify exactly one PR");

  const prNumber = pullNumbers[0];
  const pull = await githubRequest(`/repos/${repository}/pulls/${prNumber}`);
  if (pull.user?.login !== ALLOWED_AUTHOR) return reject("PR author is not allowlisted");
  if (!isAllowedRepository(pull.head?.repo)) {
    return reject("PR head repository is not allowlisted");
  }
  if (!isAllowedRepository(pull.base?.repo)) {
    return reject("PR targets an unexpected repository");
  }
  if (pull.state !== "open") return reject("PR is no longer open");
  if (pull.head?.sha?.toLowerCase() !== run.head_sha?.toLowerCase()) {
    return reject("source artifact is stale for the current PR head");
  }

  const artifactQuery = new URLSearchParams({ name: ARTIFACT_NAME, per_page: "100" });
  const artifactData = await githubRequest(
    `/repos/${repository}/actions/runs/${runId}/artifacts?${artifactQuery}`,
  );
  const artifacts = (artifactData?.artifacts ?? []).filter(
    (artifact) => artifact.name === ARTIFACT_NAME && artifact.expired !== true,
  );
  if (artifactData?.total_count !== 1 || artifacts.length !== 1) {
    return reject(`source run must contain exactly one ${ARTIFACT_NAME} artifact`);
  }
  const archiveBytes = artifacts[0].size_in_bytes;
  if (!Number.isSafeInteger(archiveBytes) || archiveBytes <= 0) {
    return reject("source artifact reports an invalid archive size");
  }
  if (archiveBytes > MAX_ARTIFACT_ARCHIVE_BYTES) {
    return reject("source artifact archive exceeds the 50 MiB preview limit");
  }

  const sha = pull.head.sha.toLowerCase();
  const target = preview(prNumber);
  await writeOutput({
    authorized: "true",
    pr_number: prNumber,
    head_sha: sha,
    worker_name: target.worker,
  });
  console.log(`Authorized PR #${prNumber} at ${sha}`);
};

const walkArtifact = async (directory) => {
  const root = resolve(directory);
  const files = [];
  let totalBytes = 0;
  const pending = [root];
  while (pending.length > 0) {
    const current = pending.pop();
    for (const name of await readdir(current)) {
      const path = resolve(current, name);
      const info = await lstat(path);
      if (info.isSymbolicLink()) throw new Error("preview artifact may not contain symbolic links");
      if (info.isDirectory()) {
        pending.push(path);
      } else if (info.isFile()) {
        if (info.size > MAX_ASSET_BYTES) {
          throw new Error("preview artifact contains a file larger than Cloudflare's 25 MiB limit");
        }
        totalBytes += info.size;
        if (totalBytes > MAX_ASSET_TOTAL_BYTES) {
          throw new Error("preview artifact exceeds the 100 MiB expanded-size limit");
        }
        files.push(path.slice(root.length + 1));
        if (files.length > MAX_ASSET_FILES) {
          throw new Error(`preview artifact exceeds Cloudflare's ${MAX_ASSET_FILES}-file limit`);
        }
      } else {
        throw new Error("preview artifact may contain only regular files and directories");
      }
    }
  }
  return { root, files };
};

const validateArtifact = async () => {
  const sha = fullSha();
  const { root, files } = await walkArtifact(flag("dir"));
  const required = [
    "index.html",
    "sandbox.html",
    "ide.html",
    "docs/index.html",
    "manual/index.html",
    "pkg/functor_runtime_web.js",
    "pkg/functor_runtime_web_bg.wasm",
    "pkg/functor_lang_wasm.js",
    "pkg/functor_lang_wasm_bg.wasm",
  ];
  const present = new Set(files);
  const missing = required.filter((path) => !present.has(path));
  if (missing.length > 0) {
    throw new Error(`preview artifact is incomplete; missing ${missing.join(", ")}`);
  }
  const forbidden = files.filter((path) => FORBIDDEN_ASSET_FILES.has(path));
  if (forbidden.length > 0) {
    throw new Error(
      `preview artifact may not contain Cloudflare deployment controls: ${forbidden.join(", ")}`,
    );
  }

  const expectedBadge =
    `<span class="version-badge" title="PR preview for commit ${sha}">` +
    `${sha.slice(0, 7)} · preview</span>`;
  for (const path of [
    "index.html",
    "sandbox.html",
    "ide.html",
    "docs/index.html",
    "manual/index.html",
  ]) {
    const html = await readFile(resolve(root, path), "utf8");
    const badges = html.match(/<span class="version-badge"[^>]*>[^<]*<\/span>/g) ?? [];
    if (badges.length !== 1 || badges[0] !== expectedBadge) {
      throw new Error(`${path} does not contain exactly one badge for its source commit`);
    }
  }
  console.log(`Validated ${files.length} static preview assets for ${sha}`);
};

const cloudflareRequest = async (path, init = {}, { allowNotFound = false } = {}) => {
  const response = await fetch(`https://api.cloudflare.com/client/v4${path}`, {
    ...init,
    headers: {
      Authorization: `Bearer ${requiredEnv("CLOUDFLARE_API_TOKEN")}`,
      ...(init.body ? { "Content-Type": "application/json" } : {}),
      ...init.headers,
    },
  });
  if (allowNotFound && response.status === 404) return null;
  const text = await response.text();
  let data = null;
  if (text) {
    try {
      data = JSON.parse(text);
    } catch {
      throw new Error(`Cloudflare API returned non-JSON (${response.status})`);
    }
  }
  if (!response.ok || data?.success === false) {
    const detail = data?.errors?.map((error) => error.message).join("; ") || response.statusText;
    throw new Error(`Cloudflare API ${response.status}: ${safeLog(detail)}`);
  }
  return data;
};

const cloudflareAccount = () => {
  const account = requiredEnv("CLOUDFLARE_ACCOUNT_ID");
  if (!/^[0-9a-f]{32}$/i.test(account)) throw new Error("CLOUDFLARE_ACCOUNT_ID is malformed");
  return account;
};

const listDomains = async (hostname) => {
  const account = cloudflareAccount();
  const query = new URLSearchParams({ hostname });
  const data = await cloudflareRequest(`/accounts/${account}/workers/domains?${query}`);
  return (data?.result ?? []).filter((domain) => domain.hostname === hostname);
};

const attach = async () => {
  const pr = positiveInteger("pr");
  const account = cloudflareAccount();
  const target = preview(pr);
  const domains = await listDomains(target.hostname);
  const foreign = domains.find((domain) => domain.service !== target.worker);
  if (foreign) {
    throw new Error(`refusing to replace ${target.hostname}; it belongs to another Worker`);
  }
  if (domains.some((domain) => domain.service === target.worker)) {
    console.log(`${target.hostname} is already attached to ${target.worker}`);
    return;
  }

  await cloudflareRequest(`/accounts/${account}/workers/domains`, {
    method: "PUT",
    body: JSON.stringify({
      hostname: target.hostname,
      service: target.worker,
      zone_name: PREVIEW_ZONE,
    }),
  });
  console.log(`Attached ${target.url} to ${target.worker}`);
};

const listCertificatePacks = async (zoneId) => {
  const packs = [];
  for (let page = 1; page <= 20; page += 1) {
    // A PR can close while its Custom Domain certificate is still provisioning,
    // so include non-active packs when resolving Cloudflare's returned cert_id.
    const query = new URLSearchParams({ status: "all", per_page: "50", page: String(page) });
    const data = await cloudflareRequest(`/zones/${zoneId}/ssl/certificate_packs?${query}`);
    packs.push(...(data?.result ?? []));
    if (page >= (data?.result_info?.total_pages ?? 1)) return packs;
  }
  throw new Error("certificate pack inventory exceeds 20 pages");
};

const packOnlyCovers = (pack, hostname) => {
  const hosts = [
    ...(Array.isArray(pack.hosts) ? pack.hosts : []),
    ...(Array.isArray(pack.certificates)
      ? pack.certificates.flatMap((certificate) => certificate.hosts ?? [])
      : []),
  ].map((host) => String(host).toLowerCase());
  return hosts.length > 0 && hosts.every((host) => host === hostname);
};

const normalizeCloudflareId = (value) => String(value).replaceAll("-", "").toLowerCase();

const resolveCertificatePack = async (domain) => {
  if (!domain.zone_id || !domain.cert_id) return null;
  const domainCertificateId = normalizeCloudflareId(domain.cert_id);
  let lastInventoryError = null;
  for (const delay of CERTIFICATE_PACK_LOOKUP_DELAYS_MS) {
    if (delay > 0) await new Promise((resolveDelay) => setTimeout(resolveDelay, delay));
    try {
      const packs = await listCertificatePacks(domain.zone_id);
      lastInventoryError = null;
      const candidates = packs.filter(
        (pack) =>
          !["deleted", "pending_deletion"].includes(pack.status) &&
          pack.certificates?.some(
            (certificate) => normalizeCloudflareId(certificate.id) === domainCertificateId,
          ),
      );
      if (candidates.length === 1 && packOnlyCovers(candidates[0], domain.hostname)) {
        return candidates[0];
      }
    } catch (error) {
      lastInventoryError = error;
    }
  }
  if (lastInventoryError) {
    console.warn(
      `Certificate inventory warning after retries: ${safeLog(lastInventoryError.message)}`,
    );
  }
  // Never substitute a hostname-only guess: certificate cleanup is optional,
  // and an exact ID miss is safer to leave for inventory review than delete.
  console.warn(
    `Could not safely identify the generated certificate for ${domain.hostname}; ` +
      "leaving it for manual review",
  );
  return null;
};

const cleanupCertificate = async (domain, pack) => {
  if (!pack) return;
  try {
    await cloudflareRequest(
      `/zones/${domain.zone_id}/ssl/certificate_packs/${pack.id}`,
      { method: "DELETE" },
      { allowNotFound: true },
    );
    console.log(`Deleted generated certificate for ${domain.hostname}`);
  } catch (error) {
    // Cloudflare documents leftover Custom Domain certificates as harmless.
    // Domain and Worker deletion are the quota-relevant cleanup; keep them
    // successful while making certificate inventory drift visible in logs.
    console.warn(`Certificate cleanup warning: ${safeLog(error.message)}`);
  }
};

const cleanup = async () => {
  const pr = positiveInteger("pr");
  const account = cloudflareAccount();
  const target = preview(pr);
  const detached = [];

  let domains;
  try {
    domains = await listDomains(target.hostname);
  } catch (error) {
    throw new Error(`preview cleanup incomplete: list domain: ${safeLog(error.message)}`);
  }

  const failures = [];
  for (const domain of domains) {
    if (domain.service !== target.worker) {
      failures.push(`${target.hostname} belongs to unexpected Worker ${domain.service}`);
      continue;
    }
    // Resolve the generated certificate while Cloudflare still exposes its
    // Custom Domain record. Detaching first made the certificate-pack lookup
    // race the provider's eventually consistent inventory.
    const certificatePack = await resolveCertificatePack(domain);
    try {
      await cloudflareRequest(
        `/accounts/${account}/workers/domains/${domain.id}`,
        { method: "DELETE" },
        { allowNotFound: true },
      );
      detached.push({ domain, certificatePack });
      console.log(`Detached ${target.hostname}`);
    } catch (error) {
      failures.push(`detach domain: ${error.message}`);
    }
  }

  // Do not delete a Worker while one of its Custom Domains may still be
  // attached. A later rerun can retry the detach without leaving a dangling URL.
  if (failures.length > 0) {
    throw new Error(`preview cleanup incomplete: ${failures.map(safeLog).join("; ")}`);
  }

  try {
    await cloudflareRequest(
      `/accounts/${account}/workers/scripts/${target.worker}`,
      { method: "DELETE" },
      { allowNotFound: true },
    );
    console.log(`Deleted ${target.worker} (or it was already absent)`);
  } catch (error) {
    failures.push(`delete Worker: ${error.message}`);
  }

  for (const { domain, certificatePack } of detached) {
    await cleanupCertificate(domain, certificatePack);
  }

  if (failures.length > 0) {
    throw new Error(`preview cleanup incomplete: ${failures.map(safeLog).join("; ")}`);
  }
};

const findStickyComment = async (repository, pr) => {
  for (let page = 1; page <= 20; page += 1) {
    const comments = await githubRequest(
      `/repos/${repository}/issues/${pr}/comments?per_page=100&page=${page}`,
    );
    const found = comments.find(
      (comment) =>
        comment.user?.login === "github-actions[bot]" && comment.body?.includes(COMMENT_MARKER),
    );
    if (found) return found;
    if (comments.length < 100) return null;
  }
  throw new Error("could not search all PR comments for the preview marker");
};

const comment = async () => {
  const repository = githubRepository();
  const pr = positiveInteger("pr");
  const sha = fullSha();
  const state = flag("state");
  const target = preview(pr);
  const commitUrl = `https://github.com/${repository}/commit/${sha}`;
  const commit = `[\`${sha.slice(0, 7)}\`](${commitUrl})`;

  let body;
  if (state === "ready") {
    body = `${COMMENT_MARKER}\n### Functor PR preview\n\n` +
      `Deployed commit ${commit}. The stable URL updates after each push. ` +
      `On this PR's first deploy, TLS provisioning may take a few minutes.\n\n` +
      `[Site](${target.url}/) · ` +
      `[Sandbox](${target.url}/sandbox.html) · ` +
      `[IDE](${target.url}/ide.html) · ` +
      `[API reference](${target.url}/docs/)\n\n` +
      `<sub>Full commit: <code>${sha}</code>. This temporary Worker is removed when the PR closes.</sub>`;
  } else if (state === "closed") {
    body = `${COMMENT_MARKER}\n### Functor PR preview\n\n` +
      `🧹 Preview removed because this PR closed. Last previewed commit: ${commit}.`;
  } else if (state === "cleanup-failed") {
    body = `${COMMENT_MARKER}\n### Functor PR preview\n\n` +
      `⚠️ Automatic cleanup failed for ${target.hostname}. ` +
      `Check the **Cleanup PR Preview** workflow logs. Last commit: ${commit}.`;
  } else {
    throw new Error("--state must be ready, closed, or cleanup-failed");
  }

  const existing = await findStickyComment(repository, pr);
  if (existing) {
    await githubRequest(`/repos/${repository}/issues/comments/${existing.id}`, {
      method: "PATCH",
      body: JSON.stringify({ body }),
    });
    console.log(`Updated preview comment on PR #${pr}`);
  } else if (state === "closed") {
    // A build can fail before ever creating a preview. Closing that PR should
    // still run idempotent resource cleanup, but should not add a misleading
    // new "removed" comment for a URL that never existed.
    console.log(`No preview comment exists on PR #${pr}; nothing to mark as removed`);
  } else {
    await githubRequest(`/repos/${repository}/issues/${pr}/comments`, {
      method: "POST",
      body: JSON.stringify({ body }),
    });
    console.log(`Created preview comment on PR #${pr}`);
  }
};

try {
  if (command === "authorize-run") await authorizeRun();
  else if (command === "validate-artifact") await validateArtifact();
  else if (command === "attach") await attach();
  else if (command === "cleanup") await cleanup();
  else if (command === "comment") await comment();
  else throw new Error(`unknown command ${JSON.stringify(command)}`);
} catch (error) {
  console.error(`PR preview error: ${safeLog(error?.message ?? error)}`);
  process.exitCode = 1;
}
