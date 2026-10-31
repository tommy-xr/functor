// Rebake the office's night and day lightmaps and record their provenance.
//
//   node examples/liminal-office/bake.mjs           # bake, copy into baked/, write baked/provenance.txt
//   node examples/liminal-office/bake.mjs --check   # bake into a temp dir; fail unless byte-identical to baked/
//
// The CLI is $FUNCTOR_BIN, else target/release/functor, else target/debug/functor.
// provenance.txt records the layout hash (provenance.fun computes the same one
// at load and reports a mismatch as a stale bake), the bakes, the baker's
// version and the settings, both read back from the PNGs the baker wrote.
// Both bakes unwrap the same mesh, so their .mesh.bin files must match; only
// night.mesh.bin is kept.
import { spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
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

// Bake into `output`, then check the bakes share one mesh and keep only
// night.mesh.bin. Throws on any failure.
function bake(output) {
  const flags = Object.entries(SETTINGS).flatMap(([k, v]) => [`--${k}`, String(v)]);
  const start = Date.now();
  const run = spawnSync(bin, ["-d", DIR, "bake", "--output", output, ...flags], { stdio: "inherit" });
  if (run.status !== 0) throw new Error(`functor bake exited with ${run.status}`);
  const seconds = (Date.now() - start) / 1000;
  const mesh = readFileSync(join(output, "night.mesh.bin"));
  for (const b of BAKES.filter((b) => b !== "night")) {
    if (!readFileSync(join(output, `${b}.mesh.bin`)).equals(mesh)) {
      throw new Error(`${b}.mesh.bin differs from night.mesh.bin: the bakes must share one mesh`);
    }
    rmSync(join(output, `${b}.mesh.bin`));
  }
  return seconds;
}

// Both modes bake into a temp dir, so a failed bake leaves baked/ untouched.
const baked = join(DIR, "baked");
const temp = mkdtempSync(join(tmpdir(), "liminal-office-bake-"));
try {
  if (process.argv.includes("--check")) {
    const seconds = bake(temp);
    const differ = ARTIFACTS.filter((f) => !readFileSync(join(temp, f)).equals(readFileSync(join(baked, f))));
    console.log(`rebaked in ${seconds.toFixed(1)} s`);
    if (differ.length) throw new Error(`not byte-identical to baked/: ${differ.join(", ")}`);
    console.log(`byte-identical to baked/: ${ARTIFACTS.join(", ")}`);
  } else {
    // Hash before and after: an edit during the bake would otherwise certify
    // the old bake against the new layout.
    const hash = layoutHash();
    const seconds = bake(temp);
    if (layoutHash() !== hash) throw new Error(`${SOURCES.join(" or ")} changed during the bake: rerun it`);
    const made = BAKES.map((b) => {
      const text = pngText(join(temp, `${b}.png`));
      return [text["functor-bake-version"], text["functor-bake-settings"]];
    });
    const [version, settings] = made[0];
    if (!version || !settings || made.some(([v, s]) => v !== version || s !== settings)) {
      throw new Error("the PNGs lack or disagree on their functor-bake-version / functor-bake-settings text chunks");
    }
    for (const f of ARTIFACTS) copyFileSync(join(temp, f), join(baked, f));
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
} catch (e) {
  console.error(e.message);
  process.exitCode = 1;
} finally {
  rmSync(temp, { recursive: true, force: true });
}
