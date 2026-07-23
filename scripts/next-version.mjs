// Compute the next release version from the git history. Used by the release
// pipeline (.github/workflows/release.yml) when a release is dispatched
// without an explicit version; runnable locally the same way:
//
//   node scripts/next-version.mjs     # prints X.Y.Z on stdout, reasoning on stderr
//
// Rules (pre-1.0 semver — breaking changes bump the minor, everything else
// the patch):
//   - no release tag yet                                → 0.1.0
//   - a breaking commit since the last release tag
//     (`type!:` subject or a "BREAKING CHANGE:" body)   → minor bump
//   - any change to the language/prelude surface
//     (functor-prelude/prelude/*.funi or
//     functor-prelude/stdlib/*.fun) since the last tag,
//     even if no commit message says so — these bundled
//     modules are the honest record of that surface       → minor bump
//   - anything else                                     → patch bump
// From 1.0.0 on, the same signals map to major (breaking) and minor
// (feat / surface change).

import { execFileSync } from "node:child_process";

const git = (...args) => execFileSync("git", args, { encoding: "utf8" }).trim();
const explain = (msg) => process.stderr.write(`${msg}\n`);

// Highest existing release tag (not nearest-ancestor: on a single main-only
// release train they coincide, and version order is robust to clone depth).
const tags = git("tag", "--list", "v*", "--sort=-v:refname")
  .split("\n")
  .filter((t) => /^v\d+\.\d+\.\d+$/.test(t));

if (tags.length === 0) {
  explain("no release tag yet → first release");
  console.log("0.1.0");
  process.exit(0);
}

const last = tags[0];
const [major, minor, patch] = last.slice(1).split(".").map(Number);

const subjects = git("log", `${last}..HEAD`, "--format=%s").split("\n").filter(Boolean);
if (subjects.length === 0) {
  console.error(`error: no commits since ${last} — nothing to release`);
  process.exit(1);
}

const breaking =
  subjects.some((s) => /^[a-z]+(\(.+\))?!:/.test(s)) ||
  /BREAKING[ -]CHANGE:/.test(git("log", `${last}..HEAD`, "--format=%b"));
const feature = subjects.some((s) => /^feat(\(.+\))?:/.test(s));
const surface = git(
  "diff",
  "--name-only",
  `${last}..HEAD`,
  "--",
  "functor-prelude/prelude/*.funi",
  "functor-prelude/stdlib/*.fun",
)
  .split("\n")
  .filter(Boolean);

explain(`${subjects.length} commit(s) since ${last}`);
if (breaking) explain("breaking commit (`type!:` / BREAKING CHANGE) → breaking bump");
if (surface.length) explain(`language surface changed (${surface.join(", ")}) → at least a minor bump`);

// A surface diff can be additive, so it never forces a major on its own —
// only an explicitly breaking commit does.
let next;
if (breaking && major >= 1) {
  next = `${major + 1}.0.0`;
} else if (breaking || surface.length || (feature && major >= 1)) {
  next = `${major}.${minor + 1}.0`;
} else {
  next = `${major}.${minor}.${patch + 1}`;
}

explain(`${last} → ${next}`);
console.log(next);
