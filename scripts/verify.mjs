// verify — a fast, GPU-optional smoke check of a runtime-facing change.
//
//   npm run verify
//
// Builds the CLI WITHOUT the wasm bundle (`--no-default-features`, seconds, no
// `npm run build:cli` needed), then:
//   1. typechecks every `examples/*/game.mle` under the host prelude (catches
//      load / type breaks — no GPU, no assets),
//   2. captures a frame from a couple of procedural (asset-free) scenes
//      (catches render breaks — needs a GL display; skip with `--no-render`).
//
// Exits non-zero on any failure, listing what broke.

import { execSync, spawnSync } from "node:child_process";
import { readdirSync, existsSync } from "node:fs";
import { resolve } from "node:path";

const render = !process.argv.includes("--no-render");
const bin = "target/debug/functor";
const failed = [];

console.log("▸ building native-only CLI (no wasm bundle)…");
execSync("cargo build -q -p functor-cli --no-default-features", { stdio: "inherit" });

const examples = readdirSync("examples", { withFileTypes: true })
  .filter((d) => d.isDirectory() && existsSync(`examples/${d.name}/game.mle`))
  .map((d) => d.name)
  .sort();

console.log(`\n▸ typechecking ${examples.length} examples…`);
for (const ex of examples) {
  const r = spawnSync(bin, ["-d", `examples/${ex}`, "build", "native"], {
    encoding: "utf8",
  });
  const ok = r.status === 0;
  console.log(`  ${ok ? "✓" : "✗"} ${ex}`);
  if (!ok) {
    failed.push(ex);
    process.stdout.write((r.stdout || "") + (r.stderr || ""));
  }
}

if (render) {
  console.log("\n▸ rendering procedural scenes (needs a GL display)…");
  for (const ex of ["hello-cubes", "primitives"].filter((e) =>
    examples.includes(e),
  )) {
    // `run` chdir's into the game dir, so the capture path must be absolute.
    const out = resolve(`target/verify-${ex}.png`);
    const r = spawnSync(
      bin,
      [
        "-d", `examples/${ex}`, "run", "native",
        "--capture-frame", out, "--capture-time", "1", "--fixed-time", "1",
      ],
      { encoding: "utf8" },
    );
    const ok = r.status === 0 && existsSync(out);
    console.log(`  ${ok ? "✓" : "✗"} render ${ex} → ${out}`);
    if (!ok) {
      failed.push(`render:${ex}`);
      process.stdout.write((r.stdout || "") + (r.stderr || ""));
    }
  }
}

if (failed.length) {
  console.error(`\n✗ verify failed: ${failed.join(", ")}`);
  process.exit(1);
}
console.log("\n✓ verify passed");
