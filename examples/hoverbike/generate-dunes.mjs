// Generate the dune course: a 16-bit heightmap PNG and `dunesLayout.fun`,
// the generated constants the game places things with. Node built-ins only;
// re-running reproduces both files byte for byte:
//
//   node examples/hoverbike/generate-dunes.mjs
//
// 1025² samples over 1 km is ~0.98 m per cell, the finest collision
// `Physics.heightfield` keeps (1025 samples per axis), and about the bike's
// own emitter footprint (0.9 × 1.5 m). Coarser cells facet the ground at
// the bike's scale: the 4 km terrain's ~4 m cells turn a smooth slope into
// grade steps that kick the hover springs.
//
// The ground is seeded dunes, flattened along a straight corridor that runs
// north from the start pad, plus one crest across it:
//
//   start pad (100, -380) → straight north → crest at z = -180 (5 m, gentle
//   both ways) → flat runout → the block at z = 40 → open dunes to turn in.
//
// The script prints the steepest grade (under 9%): climbing at 24 m/s, the
// hover springs ride springDamping × climb rate / springRate low (ride.fun).
// Byte-identical output assumes the same Node (its zlib does the deflate).
import { writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { encodePng } from "../../scripts/png.mjs";

const SEED = 42;
const SAMPLES = 1025;
const SIZE = 1000; // m, square, centred on the origin
const CELL = SIZE / (SAMPLES - 1);
// A tight range, so the terrain's height bands span the dunes and the crest
// top: 15 m over 16 bits is a quarter-millimetre step.
const MIN_HEIGHT = -5; // m: black
const MAX_HEIGHT = 10; // m: white

const PAD = { x: 100, z: -380 };
const CORRIDOR = { x: 100, z0: -440, z1: 60, flat: 22, blend: 70 };
const CREST = { z: -180, height: 5, sigma: 45, flatX: 150, fadeX: 400 };
const BLOCK = { x: 100, z: 40 };

const clamp = (v, lo, hi) => Math.max(lo, Math.min(hi, v));
const smoothstep = (a, b, v) => {
  const t = clamp((v - a) / (b - a), 0, 1);
  return t * t * (3 - 2 * t);
};
const lerp = (a, b, t) => a + (b - a) * t;

// Integer-lattice value noise, a pure function of the seed.
const hash2 = (ix, iy) => {
  let h = Math.imul(ix, 0x27d4eb2d) ^ Math.imul(iy, 0x165667b1) ^ SEED;
  h = Math.imul(h ^ (h >>> 15), 0x2c1b3c6d);
  h = Math.imul(h ^ (h >>> 12), 0x297a2d39);
  return ((h ^ (h >>> 15)) >>> 0) / 4294967296;
};
const valueNoise = (x, y) => {
  const ix = Math.floor(x);
  const iy = Math.floor(y);
  const fx = x - ix;
  const fy = y - iy;
  const ux = fx * fx * (3 - 2 * fx);
  const uy = fy * fy * (3 - 2 * fy);
  return lerp(lerp(hash2(ix, iy), hash2(ix + 1, iy), ux), lerp(hash2(ix, iy + 1), hash2(ix + 1, iy + 1), ux), uy);
};
// Low gain keeps each finer octave's slope below the one before.
const fbm = (x, y) => {
  let sum = 0;
  let amp = 1;
  let total = 0;
  for (let o = 0, f = 1; o < 3; o += 1, f *= 2.03, amp *= 0.35) {
    sum += valueNoise(x * f, y * f) * amp;
    total += amp;
  }
  return sum / total;
};

// Dunes ±2.5 m, ridges stretched east–west (wavelengths 300 m along x,
// 180 m along z).
const dunes = (x, z) => 5 * (fbm(x / 300 + 17.3, z / 180 - 4.1) - 0.5);

// 0 on the corridor (and so the pad), 1 in the open dunes.
const openness = (x, z) => {
  const dz = Math.max(CORRIDOR.z0 - z, 0, z - CORRIDOR.z1);
  return smoothstep(CORRIDOR.flat, CORRIDOR.blend, Math.hypot(x - CORRIDOR.x, dz));
};

// The crest: a Gaussian ridge across the corridor, full height within
// `flatX` of it and fading out by `fadeX`.
const crest = (x, z) => {
  const across = (z - CREST.z) / CREST.sigma;
  return CREST.height * Math.exp(-0.5 * across * across) * (1 - smoothstep(CREST.flatX, CREST.fadeX, Math.abs(x - CORRIDOR.x)));
};

const ground = (x, z) => dunes(x, z) * openness(x, z) + crest(x, z);

// Row j is z = -500 + j·CELL (south first), column i is x = -500 + i·CELL.
const levels = new Uint16Array(SAMPLES * SAMPLES);
for (let j = 0; j < SAMPLES; j += 1) {
  for (let i = 0; i < SAMPLES; i += 1) {
    const h = ground(-SIZE / 2 + i * CELL, -SIZE / 2 + j * CELL);
    if (h < MIN_HEIGHT || h > MAX_HEIGHT) throw new Error(`ground ${h} m at (${i}, ${j}) is outside the declared range`);
    levels[j * SAMPLES + i] = Math.round(((h - MIN_HEIGHT) / (MAX_HEIGHT - MIN_HEIGHT)) * 65535);
  }
}
// What the runtime reads back at a sample.
const decoded = (i, j) => MIN_HEIGHT + (levels[j * SAMPLES + i] / 65535) * (MAX_HEIGHT - MIN_HEIGHT);
const sampleAt = (x, z) => decoded(Math.round((x + SIZE / 2) / CELL), Math.round((z + SIZE / 2) / CELL));

// Steepest grade of any collision triangle (the heightfield splits each cell
// along its diagonal), over the map and along the corridor's centre line.
const grade = (i, j) => {
  const h = decoded(i, j);
  const lower = Math.hypot(decoded(i + 1, j) - h, decoded(i, j + 1) - h);
  const h11 = decoded(i + 1, j + 1);
  const upper = Math.hypot(h11 - decoded(i, j + 1), h11 - decoded(i + 1, j));
  return Math.max(lower, upper) / CELL;
};
const routeColumn = Math.round((CORRIDOR.x + SIZE / 2) / CELL);
let steepest = 0;
let steepestRoute = 0;
for (let j = 0; j + 1 < SAMPLES; j += 1) {
  for (let i = 0; i + 1 < SAMPLES; i += 1) steepest = Math.max(steepest, grade(i, j));
  steepestRoute = Math.max(steepestRoute, grade(routeColumn, j));
}

const pixels = Buffer.alloc(SAMPLES * SAMPLES * 2);
levels.forEach((level, k) => pixels.writeUInt16BE(level, k * 2));
const png = encodePng(SAMPLES, SAMPLES, 16, 0, pixels);
writeFileSync(fileURLToPath(new URL("dunes.png", import.meta.url)), png);

// ── dunesLayout.fun ─────────────────────────────────────────────────────────
// Route posts: pairs flanking the corridor every 40 m, then a sparse ring
// around the turn beyond the block. Each sits on the decoded ground.
const posts = [];
for (let z = -360; z <= 0; z += 40) posts.push({ x: CORRIDOR.x + 12, z }, { x: CORRIDOR.x - 12, z });
// The ring's first post would sit on the lane's centre line; it starts one on.
for (let a = 1; a < 8; a += 1) {
  const t = (a / 8) * Math.PI;
  posts.push({ x: Math.round(CORRIDOR.x - 70 + 70 * Math.cos(t)), z: Math.round(110 + 50 * Math.sin(t)) });
}
const num = (v) => {
  const s = (Math.round(v * 1000) / 1000).toString();
  return s.includes(".") ? s : `${s}.0`;
};
const fun = `// dunesLayout.fun — GENERATED by generate-dunes.mjs (seed ${SEED}); do not edit.
// Rerun the script after changing the course; it rewrites this and dunes.png.

let size = ${num(SIZE)} // m, square, centred on the origin
let minHeight = ${num(MIN_HEIGHT)}
let maxHeight = ${num(MAX_HEIGHT)}
// The start pad and the corridor north from it are flat at ground 0.
let pad = { x: ${num(PAD.x)}, z: ${num(PAD.z)} }
let crestZ = ${num(CREST.z)}
let block = { x: ${num(BLOCK.x)}, z: ${num(BLOCK.z)}, ground: ${num(sampleAt(BLOCK.x, BLOCK.z))} }
// Route posts on the ground: x, ground height, z.
let posts = [
${posts.map((p) => `  { x: ${num(p.x)}, y: ${num(sampleAt(p.x, p.z))}, z: ${num(p.z)} },`).join("\n")}
]
`;
writeFileSync(fileURLToPath(new URL("dunesLayout.fun", import.meta.url)), fun);

console.log(
  `wrote ${SAMPLES}² heightmap (${png.length} bytes, ${CELL.toFixed(4)} m cells); ` +
    `steepest grade ${(steepest * 100).toFixed(1)}% (route ${(steepestRoute * 100).toFixed(1)}%)`,
);
