// Trusted tooling for the sticky "PR builds" comment: links each PR head's
// Build CLI / Build Android artifacts. Always run from the default branch by
// pr-builds-comment.yml. It reads run/artifact metadata only; nothing from the
// PR is checked out or executed.

// Commands:
//   authorize-run --run-id N   → GITHUB_OUTPUT authorized, pr_number
//   comment --pr N             → upsert the comment for the PR's CURRENT head

// Environment: GITHUB_TOKEN, GITHUB_REPOSITORY, GITHUB_OUTPUT (authorize-run),
// GITHUB_STEP_SUMMARY (optional), PR_BUILDS_COMMENT_AUTHOR (optional; the
// sticky comment's author, default github-actions[bot] — set it to your login
// to probe locally with a personal token).

import { appendFile } from "node:fs/promises";
import { pathToFileURL } from "node:url";

const ALLOWED_AUTHOR = "tommy-xr";
const ALLOWED_REPOSITORY = "tommy-xr/functor";
const ALLOWED_REPOSITORY_API_URL = `https://api.github.com/repos/${ALLOWED_REPOSITORY}`;
const COMMENT_MARKER = "<!-- functor-pr-builds -->";
// Workflow name → its file; the file path, not the editable name, is the authority.
const WORKFLOWS = {
  "Build CLI": ".github/workflows/build-cli.yml",
  "Build Android": ".github/workflows/build-android.yml",
};
const TARGETS = [
  { label: "CLI macOS arm64", workflow: WORKFLOWS["Build CLI"], artifact: (sha) => `functor-cli-macos-arm64-${sha}` },
  { label: "CLI Linux x64", workflow: WORKFLOWS["Build CLI"], artifact: (sha) => `functor-cli-linux-x64-${sha}` },
  { label: "Quest APK", workflow: WORKFLOWS["Build Android"], artifact: (sha) => `functor-apk-${sha}` },
];

// Pure: the comment body for `sha` from both workflows' runs and their
// artifacts. Anything for another SHA is ignored, so the caller's ordering
// never matters. A target links its artifact if one exists, else reflects the
// newest run for the head: none/unfinished → building…, failed → ✗.
export const render = ({ repository, pr, sha, runs, artifacts }) => {
  const line = (target) => {
    const candidates = runs
      .filter((run) => run.path === target.workflow && run.head_sha === sha && run.conclusion !== "skipped")
      .sort((a, b) => b.id - a.id);
    const ids = new Set(candidates.map((run) => run.id));
    const artifact = artifacts.find(
      (a) => a.name === target.artifact(sha) && !a.expired && ids.has(a.workflow_run?.id),
    );
    if (artifact) {
      const url = `https://github.com/${repository}/actions/runs/${artifact.workflow_run.id}/artifacts/${artifact.id}`;
      return `- [${target.label}](${url})`;
    }
    const latest = candidates[0];
    if (!latest || latest.status !== "completed") return `- ${target.label}: building…`;
    if (latest.conclusion !== "success") return `- ${target.label}: ✗ [${latest.conclusion}](${latest.html_url})`;
    return `- ${target.label}: missing ([run](${latest.html_url}))`;
  };
  const commit = `[\`${sha.slice(0, 7)}\`](https://github.com/${repository}/commit/${sha})`;
  return [
    COMMENT_MARKER,
    "### Functor PR builds",
    "",
    `Builds for ${commit}:`,
    "",
    ...TARGETS.map(line),
    "",
    `Try on Quest: \`npm run pr -- ${pr}\`, or with the \`f\` wrapper: \`f --pr ${pr} -d <example> run vr\``,
    "",
    "<sub>Downloads need a GitHub login and arrive zipped.</sub>",
  ].join("\n");
};

const flagValue = (args, name) => {
  const index = args.indexOf(`--${name}`);
  const raw = index >= 0 ? args[index + 1] : undefined;
  if (!raw || !/^[1-9]\d{0,15}$/.test(raw)) throw new Error(`--${name} must be a positive integer`);
  return Number(raw);
};

const requiredEnv = (name) => {
  const value = process.env[name];
  if (!value) throw new Error(`${name} is required`);
  return value;
};

const safeLog = (value) => String(value).replace(/[\r\n%]/g, " ");

const repositoryName = () => {
  const repository = requiredEnv("GITHUB_REPOSITORY");
  if (repository !== ALLOWED_REPOSITORY) throw new Error(`unexpected repository ${safeLog(repository)}`);
  return repository;
};

// Workflow-run payloads carry only `url`; pull endpoints carry `full_name`.
const isAllowedRepository = (repository) =>
  repository?.full_name === ALLOWED_REPOSITORY || repository?.url === ALLOWED_REPOSITORY_API_URL;

const githubRequest = async (path, init = {}) => {
  const response = await fetch(`https://api.github.com${path}`, {
    ...init,
    headers: {
      Accept: "application/vnd.github+json",
      Authorization: `Bearer ${requiredEnv("GITHUB_TOKEN")}`,
      "X-GitHub-Api-Version": "2026-03-10",
      "User-Agent": "functor-pr-builds",
      ...(init.body ? { "Content-Type": "application/json" } : {}),
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

// The reason an open PR may not get a comment, or null.
const pullRejection = (pull) => {
  if (pull.user?.login !== ALLOWED_AUTHOR) return "PR author is not allowlisted";
  if (!isAllowedRepository(pull.head?.repo)) return "PR head repository is not allowlisted";
  if (!isAllowedRepository(pull.base?.repo)) return "PR targets an unexpected repository";
  if (pull.state !== "open") return "PR is no longer open";
  return null;
};

const skip = async (reason) => {
  const safeReason = safeLog(reason);
  console.log(`PR builds comment skipped: ${safeReason}`);
  const summary = process.env.GITHUB_STEP_SUMMARY;
  if (summary) await appendFile(summary, `### PR builds comment skipped\n\n${safeReason}\n`);
};

const authorizeRun = async (args) => {
  const repository = repositoryName();
  const output = requiredEnv("GITHUB_OUTPUT");
  const reject = async (reason) => {
    await skip(reason);
    await appendFile(output, "authorized=false\n");
  };
  const run = await githubRequest(`/repos/${repository}/actions/runs/${flagValue(args, "run-id")}`);

  if (WORKFLOWS[run.name] === undefined || WORKFLOWS[run.name] !== run.path) {
    return reject("unexpected source workflow");
  }
  if (run.event !== "pull_request") return reject("source event was not pull_request");
  if (run.actor?.login !== ALLOWED_AUTHOR) return reject("source actor is not allowlisted");
  if (run.triggering_actor?.login && run.triggering_actor.login !== ALLOWED_AUTHOR) {
    return reject("rerun actor is not allowlisted");
  }
  if (!isAllowedRepository(run.head_repository)) return reject("source branch is not in the trusted repository");

  // Same fallback as pr-preview.mjs: historical payloads may omit pull_requests.
  let pullNumbers = [...new Set(
    (run.pull_requests ?? []).filter((p) => isAllowedRepository(p.base?.repo)).map((p) => p.number),
  )];
  if (pullNumbers.length === 0 && /^[0-9a-f]{40}$/i.test(run.head_sha ?? "")) {
    const associated = await githubRequest(`/repos/${repository}/commits/${run.head_sha}/pulls?per_page=100`);
    pullNumbers = [...new Set(
      associated
        .filter((p) => isAllowedRepository(p.base?.repo) && p.head?.sha?.toLowerCase() === run.head_sha.toLowerCase())
        .map((p) => p.number),
    )];
  }
  if (pullNumbers.length !== 1) return reject("source run does not identify exactly one PR");

  const pull = await githubRequest(`/repos/${repository}/pulls/${pullNumbers[0]}`);
  const rejection = pullRejection(pull);
  if (rejection) return reject(rejection);
  if (pull.head?.sha?.toLowerCase() !== run.head_sha?.toLowerCase()) {
    return reject("source run is stale for the current PR head");
  }
  await appendFile(output, `authorized=true\npr_number=${pull.number}\n`);
  console.log(`Authorized PR #${pull.number} at ${pull.head.sha}`);
};

const findStickyComment = async (repository, pr, author) => {
  for (let page = 1; page <= 20; page += 1) {
    const comments = await githubRequest(`/repos/${repository}/issues/${pr}/comments?per_page=100&page=${page}`);
    const found = comments.find((c) => c.user?.login === author && c.body?.includes(COMMENT_MARKER));
    if (found) return found;
    if (comments.length < 100) return null;
  }
  throw new Error("could not search all PR comments for the builds marker");
};

// Rebuilds the whole comment from API state for the PR's current head, so a
// run for an older SHA (or the second workflow to finish) only ever refreshes it.
const comment = async (args) => {
  const repository = repositoryName();
  const pr = flagValue(args, "pr");
  const pull = await githubRequest(`/repos/${repository}/pulls/${pr}`);
  const rejection = pullRejection(pull);
  if (rejection) return skip(rejection);
  const sha = pull.head.sha.toLowerCase();

  const runs = [];
  for (const path of Object.values(WORKFLOWS)) {
    const file = path.split("/").pop();
    const data = await githubRequest(
      `/repos/${repository}/actions/workflows/${file}/runs?head_sha=${sha}&per_page=20`,
    );
    runs.push(...(data?.workflow_runs ?? []).filter((run) => run.path === path && isAllowedRepository(run.head_repository)));
  }
  const artifacts = [];
  for (const run of runs) {
    const data = await githubRequest(`/repos/${repository}/actions/runs/${run.id}/artifacts?per_page=100`);
    artifacts.push(...(data?.artifacts ?? []));
  }

  const body = render({ repository, pr, sha, runs, artifacts });
  const author = process.env.PR_BUILDS_COMMENT_AUTHOR || "github-actions[bot]";
  const existing = await findStickyComment(repository, pr, author);
  if (existing) {
    await githubRequest(`/repos/${repository}/issues/comments/${existing.id}`, {
      method: "PATCH",
      body: JSON.stringify({ body }),
    });
    console.log(`Updated builds comment on PR #${pr} for ${sha}`);
  } else {
    await githubRequest(`/repos/${repository}/issues/${pr}/comments`, {
      method: "POST",
      body: JSON.stringify({ body }),
    });
    console.log(`Created builds comment on PR #${pr} for ${sha}`);
  }
};

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [, , command, ...args] = process.argv;
  try {
    if (command === "authorize-run") await authorizeRun(args);
    else if (command === "comment") await comment(args);
    else throw new Error(`unknown command ${JSON.stringify(command)}`);
  } catch (error) {
    console.error(`PR builds comment error: ${safeLog(error?.message ?? error)}`);
    process.exitCode = 1;
  }
}
