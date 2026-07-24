// Regenerate the gitignored API reference artifacts from the exact `.funi`
// prelude embedded in Functor. `--check` validates both renderers without
// requiring generated files to exist.
//
//   npm run generate:docs
//   npm run check:docs

import { spawnSync } from "node:child_process";

const check = process.argv.includes("--check");
const outputs = [
  ["markdown", "docs/api-reference.md"],
  ["json", "site/generated/api-reference.json"],
];

for (const [format, path] of outputs) {
  const mode = check ? [] : ["--output", path];
  const result = spawnSync(
    "cargo",
    [
      "run",
      "-q",
      "-p",
      "functor-docgen",
      "--",
      "--deny-undocumented",
      "--format",
      format,
      ...mode,
    ],
    { stdio: check ? ["ignore", "ignore", "inherit"] : "inherit" },
  );
  if (result.status !== 0) process.exit(result.status ?? 1);
  console.log(check ? `✓ ${format} generation` : `generated ${path}`);
}
