// Rebake the office's night and day lightmaps and record their provenance.
//
//   node examples/liminal-office/bake.mjs           # bake into baked/, write baked/provenance.txt
//   node examples/liminal-office/bake.mjs --check   # bake into a temp dir; fail unless byte-identical to baked/
//
// The CLI is $FUNCTOR_BIN, else target/release/functor, else target/debug/functor.
// provenance.txt records the layout hash (provenance.fun computes the same one
// at load and reports a mismatch as a stale bake), the bakes, the baker's
// version and the settings, both read back from the PNGs the baker wrote.
// Both bakes unwrap the same mesh, so their .mesh.bin files must match; only
// night.mesh.bin is kept.
import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const DIR = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(DIR, "../..");
const SETTINGS = {
  "texels-per-meter": 12,
  bounces: 2,
  "gi-samples": 32,
  "ao-samples": 64,
  "ao-distance": 1,
  seed: 0,
  "max-size": 2048,
};
// Must match `sources` in provenance.fun, and `Building.bakes`.
const SOURCES = ["layout.fun", "building.fun"];
const BAKES = ["night", "day"];
const ARTIFACTS = [...BAKES.map((b) => `${b}.png`), "night.mesh.bin"];

const bin =
  process.env.FUNCTOR_BIN ??
  [join(ROOT, "target/release/functor"), join(ROOT, "target/debug/functor")].find(existsSync) ??
  "functor";

// provenance.fun's hash: u32le words then tail bytes, h = (h * 1000003 + x) mod
// 4294967291. Every intermediate stays below 2^53, so doubles are exact.
function layoutHash() {
  let h = 0;
  const step = (x) => (h = (h * 1000003 + x) % 4294967291);
  for (const name of SOURCES) {
    const b = readFileSync(join(DIR, name));
    const words = Math.floor(b.length / 4);
    for (let i = 0; i < words; i++) step(b.readUInt32LE(4 * i));
    for (let i = 4 * words; i < b.length; i++) step(b[i]);
  }
  return h;
}

// A PNG's tEXt chunks as { keyword: text }.
function pngText(path) {
  const b = readFileSync(path);
  const out = {};
  for (let at = 8; at < b.length; ) {
    const len = b.readUInt32BE(at);
    const type = b.toString("latin1", at + 4, at + 8);
    if (type === "tEXt") {
      const body = b.subarray(at + 8, at + 8 + len);
      const nul = body.indexOf(0);
      out[body.toString("latin1", 0, nul)] = body.toString("latin1", nul + 1);
    }
    at += 12 + len;
  }
  return out;
}

function bake(output) {
  const flags = Object.entries(SETTINGS).flatMap(([k, v]) => [`--${k}`, String(v)]);
  const start = Date.now();
  const run = spawnSync(bin, ["-d", DIR, "bake", "--output", output, ...flags], { stdio: "inherit" });
  if (run.status !== 0) process.exit(run.status ?? 1);
  const seconds = (Date.now() - start) / 1000;
  const mesh = readFileSync(join(output, "night.mesh.bin"));
  for (const b of BAKES.filter((b) => b !== "night")) {
    if (!readFileSync(join(output, `${b}.mesh.bin`)).equals(mesh)) {
      console.error(`${b}.mesh.bin differs from night.mesh.bin: the bakes must share one mesh`);
      process.exit(1);
    }
    rmSync(join(output, `${b}.mesh.bin`));
  }
  return seconds;
}

const baked = join(DIR, "baked");
if (process.argv.includes("--check")) {
  const temp = mkdtempSync(join(tmpdir(), "liminal-office-bake-"));
  try {
    const seconds = bake(temp);
    const differ = ARTIFACTS.filter((f) => !readFileSync(join(temp, f)).equals(readFileSync(join(baked, f))));
    console.log(`rebaked in ${seconds.toFixed(1)} s`);
    if (differ.length) {
      console.error(`not byte-identical to baked/: ${differ.join(", ")}`);
      process.exit(1);
    }
    console.log(`byte-identical to baked/: ${ARTIFACTS.join(", ")}`);
  } finally {
    rmSync(temp, { recursive: true, force: true });
  }
} else {
  // Hash before and after: an edit during the bake would otherwise certify the
  // old bake against the new layout.
  const hash = layoutHash();
  const seconds = bake(baked);
  if (layoutHash() !== hash) {
    console.error(`${SOURCES.join(" or ")} changed during the bake: rerun it`);
    process.exit(1);
  }
  const made = BAKES.map((b) => {
    const text = pngText(join(baked, `${b}.png`));
    return [text["functor-bake-version"], text["functor-bake-settings"]];
  });
  const [version, settings] = made[0];
  if (!version || !settings || made.some(([v, s]) => v !== version || s !== settings)) {
    console.error("the PNGs lack or disagree on their functor-bake-version / functor-bake-settings text chunks");
    process.exit(1);
  }
  const record = [
    `layout=${hash}`,
    `sources=${SOURCES.join(" ")}`,
    `bakes=${BAKES.join(" ")}`,
    `bakeVersion=${version}`,
    `settings=${settings}`,
    "",
  ].join("\n");
  writeFileSync(join(baked, "provenance.txt"), record);
  console.log(`baked in ${seconds.toFixed(1)} s; wrote baked/provenance.txt`);
}
