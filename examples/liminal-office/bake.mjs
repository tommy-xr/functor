// Rebake every bake's night and day lightmaps and record their provenance.
//
//   node examples/liminal-office/bake.mjs           # bake, copy into baked/<archetype>/, write its provenance.txt
//   node examples/liminal-office/bake.mjs --check   # bake into a temp dir; fail unless byte-identical to baked/
//
// The CLI is $FUNCTOR_BIN, else target/release/functor, else target/debug/functor.
// One `functor bake` makes every bake's `<bake>-<hour>` jobs (an archetype's
// own, or a tower's two); each lands in baked/<bake>/<hour>.png. Its
// provenance.txt records the layout
// hash (provenance.fun computes the same one at load and reports a mismatch
// as a stale bake), the bakes, the baker's version and the settings, both read
// back from the PNGs the baker wrote. Both bakes unwrap the same mesh, so their
// .mesh.bin files must match; only night.mesh.bin is kept.
import { spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
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
// Each bake's sources: must match archetypes.fun's `bakeOf` calls.
const TOWER = ["layout.fun", "towerLayout.fun", "building.fun"];
const ARCHETYPES = { office: ["layout.fun", "officeLayout.fun", "building.fun"], "tower-typical": TOWER, "tower-ends": TOWER };
const BAKES = ["night", "day"];
const ARTIFACTS = [...BAKES.map((b) => `${b}.png`), "night.mesh.bin"];

const bin =
  process.env.FUNCTOR_BIN ??
  [join(ROOT, "target/release/functor"), join(ROOT, "target/debug/functor")].find(existsSync) ??
  "functor";

// provenance.fun's hash: u32le words then tail bytes, h = (h * 1000003 + x) mod
// 4294967291. Every intermediate stays below 2^53, so doubles are exact.
function layoutHash(sources) {
  let h = 0;
  const step = (x) => (h = (h * 1000003 + x) % 4294967291);
  for (const name of sources) {
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

// Bake into `output`, then sort each archetype's jobs into output/<archetype>/,
// check its bakes share one mesh and keep only night.mesh.bin. Throws on any
// failure.
function bake(output) {
  const flags = Object.entries(SETTINGS).flatMap(([k, v]) => [`--${k}`, String(v)]);
  const start = Date.now();
  const run = spawnSync(bin, ["-d", DIR, "bake", "--output", output, ...flags], { stdio: "inherit" });
  if (run.status !== 0) throw new Error(`functor bake exited with ${run.status}`);
  const seconds = (Date.now() - start) / 1000;
  for (const a of Object.keys(ARCHETYPES)) {
    mkdirSync(join(output, a));
    for (const b of BAKES) {
      renameSync(join(output, `${a}-${b}.png`), join(output, a, `${b}.png`));
      renameSync(join(output, `${a}-${b}.mesh.bin`), join(output, a, `${b}.mesh.bin`));
    }
    const mesh = readFileSync(join(output, a, "night.mesh.bin"));
    for (const b of BAKES.filter((b) => b !== "night")) {
      if (!readFileSync(join(output, a, `${b}.mesh.bin`)).equals(mesh)) {
        throw new Error(`${a}: ${b}.mesh.bin differs from night.mesh.bin: the bakes must share one mesh`);
      }
      rmSync(join(output, a, `${b}.mesh.bin`));
    }
  }
  // A job left over belongs to an archetype missing from ARCHETYPES.
  const left = readdirSync(output).filter((f) => f.endsWith(".png") || f.endsWith(".mesh.bin"));
  if (left.length) throw new Error(`bakes of no archetype in ARCHETYPES: ${left.join(", ")}`);
  return seconds;
}

// Both modes bake into a temp dir, so a failed bake leaves baked/ untouched.
const baked = join(DIR, "baked");
const temp = mkdtempSync(join(tmpdir(), "liminal-office-bake-"));
try {
  if (process.argv.includes("--check")) {
    const seconds = bake(temp);
    console.log(`rebaked in ${seconds.toFixed(1)} s`);
    const files = Object.keys(ARCHETYPES).flatMap((a) => ARTIFACTS.map((f) => join(a, f)));
    const differ = files.filter((f) => !readFileSync(join(temp, f)).equals(readFileSync(join(baked, f))));
    if (differ.length) throw new Error(`not byte-identical to baked/: ${differ.join(", ")}`);
    console.log(`byte-identical to baked/: ${files.join(", ")}`);
  } else {
    // Hash before and after: an edit during the bake would otherwise certify
    // the old bake against the new layout.
    const hashes = Object.values(ARCHETYPES).map(layoutHash);
    const seconds = bake(temp);
    Object.entries(ARCHETYPES).forEach(([a, sources], i) => {
      if (layoutHash(sources) !== hashes[i]) throw new Error(`${sources.join(" or ")} changed during the bake: rerun it`);
    });
    const records = Object.entries(ARCHETYPES).map(([a, sources], i) => {
      const made = BAKES.map((b) => {
        const text = pngText(join(temp, a, `${b}.png`));
        return [text["functor-bake-version"], text["functor-bake-settings"]];
      });
      const [version, settings] = made[0];
      if (!version || !settings || made.some(([v, s]) => v !== version || s !== settings)) {
        throw new Error(`${a}: the PNGs lack or disagree on their functor-bake-version / functor-bake-settings text chunks`);
      }
      const record = [
        `layout=${hashes[i]}`,
        `sources=${sources.join(" ")}`,
        `bakes=${BAKES.join(" ")}`,
        `bakeVersion=${version}`,
        `settings=${settings}`,
        "",
      ].join("\n");
      return [a, record];
    });
    for (const [a, record] of records) {
      mkdirSync(join(baked, a), { recursive: true });
      for (const f of ARTIFACTS) copyFileSync(join(temp, a, f), join(baked, a, f));
      writeFileSync(join(baked, a, "provenance.txt"), record);
    }
    console.log(`baked in ${seconds.toFixed(1)} s; wrote ${records.map(([a]) => `baked/${a}/provenance.txt`).join(", ")}`);
  }
} catch (e) {
  console.error(e.message);
  process.exitCode = 1;
} finally {
  rmSync(temp, { recursive: true, force: true });
}
