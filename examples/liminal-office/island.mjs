// Generate the island the block stands on: a 16-bit heightmap PNG and
// `islandLayout.fun`, the generated constants the game places it with. Node
// built-ins only; re-running reproduces both files byte for byte (with the
// same Node: its zlib does the deflate):
//
//   node examples/liminal-office/island.mjs
//
// 2049² samples over 2048 m: one sample per metre, so every terrain LOD
// vertex lands on a sample. `Physics.heightfield` keeps every other one
// (1025 per axis, 2 m cells).
//
// The island: a seabed, a beach at sea level and gentle hills inland, with a
// noisy coastline. The block stands on a flat pad (one u16 level) in the
// south of the island, a smoothstep ramp around it. Out to `BAND` past the
// pad's edge the ground stays between the pad and `CAP` above it, so the
// block's collars (0–8 m over the street) hold every eye there.
import { writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { encodePng } from "../../scripts/png.mjs";

const SEED = 2049;
const SAMPLES = 2049;
const SIZE = 2048; // m, square
const CELL = SIZE / (SAMPLES - 1); // 1 m
// Black and white. The terrain's height bands span this range.
const MIN_HEIGHT = -24;
const MAX_HEIGHT = 72;
const SEA_LEVEL = 0;

// The pad, in the block's space: the four copies' footprints (x, z from -53
// to 29) and a 6 m apron. Its south-west corner sits at `PAD_LOCAL` in the
// terrain's space (even, so the 2 m collider's samples fall on its edges).
const PAD = { x0: -59, z0: -59, x1: 35, z1: 35 };
const PAD_LOCAL = { x: 12, z: -478 };
// Where the terrain's centre sits in the block's space.
const CENTRE = { x: PAD.x0 - PAD_LOCAL.x, z: PAD.z0 - PAD_LOCAL.z };
const PAD_MIDDLE = { x: (PAD.x0 + PAD.x1) / 2 - CENTRE.x, z: (PAD.z0 + PAD.z1) / 2 - CENTRE.z };
// The ramp from the pad to the ground around it.
const RAMP = 16;
// Out to BAND past the pad's edge the ground lies in [pad, pad + CAP); it
// returns to the island's own over the next BLEND.
const BAND = 30;
const CAP = 3;
const BLEND = 60;

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
// About 0..1; low gain keeps each finer octave's slope below the one before.
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

// The island in the terrain's space: how far inland (x, z) is (negative out
// at sea), from a coastline 740–880 m out from the centre.
const inland = (x, z) => {
  const a = Math.atan2(z, x);
  const coast = 810 + 140 * (fbm(2.2 * Math.cos(a) + 11.3, 2.2 * Math.sin(a) + 3.7) - 0.5);
  return coast - Math.hypot(x, z);
};
// Rising from the waterline at its steepest, levelling off at `run`.
const shelf = (rise, run, d) => {
  const t = 1 - clamp(d / run, 0, 1);
  return rise * (1 - t * t * t);
};
// Seabed to 22 m down; a beach that meets the sea at a 14% grade (a
// flatter one blurs where the sea plane crosses it); 16 m up over the first
// 350 m inland, a broad rise to the north, and hills inland of the beach,
// calmer around the pad.
const island = (x, z) => {
  const s = inland(x, z);
  if (s < 0) return SEA_LEVEL - shelf(22, 300, -s);
  const hills = 24 * (fbm(x / 420 + 5.2, z / 420 - 1.7) - 0.5) + 8 * (fbm(x / 160 - 3.3, z / 160 + 8.8) - 0.5);
  const calm = lerp(0.25, 1, smoothstep(80, 320, Math.hypot(x - PAD_MIDDLE.x, z - PAD_MIDDLE.z)));
  return SEA_LEVEL + shelf(16, 350, s) + 14 * smoothstep(-300, 500, z) * smoothstep(100, 500, s) + hills * calm * smoothstep(30, 300, s);
};

// Distance from (x, z) to the pad's rect, in the terrain's space (0 on it).
const fromPad = (x, z) => {
  const dx = Math.max(PAD.x0 - CENTRE.x - x, 0, x - (PAD.x1 - CENTRE.x));
  const dz = Math.max(PAD.z0 - CENTRE.z - z, 0, z - (PAD.z1 - CENTRE.z));
  return Math.hypot(dx, dz);
};

const level = (h) => Math.round(((h - MIN_HEIGHT) / (MAX_HEIGHT - MIN_HEIGHT)) * 65535);
// What the runtime reads back at a level.
const decode = (l) => MIN_HEIGHT + (l / 65535) * (MAX_HEIGHT - MIN_HEIGHT);
// The pad's level: the island's own ground at its middle.
const PAD_LEVEL = level(island(PAD_MIDDLE.x, PAD_MIDDLE.z));
const PAD_Y = decode(PAD_LEVEL);

// The ground, in the terrain's space: the island, held in [pad, pad + CAP)
// out to BAND (a soft floor and cap: no kinks), then flattened onto the pad
// through the ramp.
export const ground = (x, z) => {
  const d = fromPad(x, z);
  const h = island(x, z);
  if (d >= BAND + BLEND) return h;
  const u = h - PAD_Y;
  const above = (u + Math.sqrt(u * u + 1)) / 2; // > 0, ≈ max(u, 0)
  const held = PAD_Y + (CAP * above) / (above + CAP);
  const near = lerp(held, h, smoothstep(BAND, BAND + BLEND, d));
  return lerp(PAD_Y, near, smoothstep(0, RAMP, d));
};
// The terrain's space from the block's.
export const toTerrain = (x, z) => [x - CENTRE.x, z - CENTRE.z];
export const padY = PAD_Y;

const levels = new Uint16Array(SAMPLES * SAMPLES);
// Row j is z = -1024 + j (south first), column i is x = -1024 + i.
const at = (i) => -SIZE / 2 + i * CELL;
const generate = () => {
  for (let j = 0; j < SAMPLES; j += 1) {
    for (let i = 0; i < SAMPLES; i += 1) {
      const x = at(i);
      const z = at(j);
      const h = ground(x, z);
      if (h < MIN_HEIGHT || h > MAX_HEIGHT) throw new Error(`ground ${h} m at (${x}, ${z}) is outside the declared range`);
      levels[j * SAMPLES + i] = fromPad(x, z) === 0 ? PAD_LEVEL : level(h);
    }
  }
};
const sample = (i, j) => decode(levels[j * SAMPLES + i]);
const column = (x) => Math.round((x + SIZE / 2) / CELL);

// Checks on the samples: the border is under the sea; the steepest grade of
// any triangle; the island's own relief and slope under the pad and ramp
// (before flattening); and the band's height range.
const survey = () => {
  for (let k = 0; k < SAMPLES; k += 1) {
    for (const [i, j] of [[k, 0], [k, SAMPLES - 1], [0, k], [SAMPLES - 1, k]]) {
      if (sample(i, j) > SEA_LEVEL - 2) throw new Error(`the map's edge is above the sea at (${at(i)}, ${at(j)})`);
    }
  }
  const grade = (i, j) => {
    const h = sample(i, j);
    const h11 = sample(i + 1, j + 1);
    return Math.max(Math.hypot(sample(i + 1, j) - h, sample(i, j + 1) - h), Math.hypot(h11 - sample(i, j + 1), h11 - sample(i + 1, j))) / CELL;
  };
  let steepest = 0;
  for (let j = 0; j + 1 < SAMPLES; j += 1) for (let i = 0; i + 1 < SAMPLES; i += 1) steepest = Math.max(steepest, grade(i, j));
  const [x0, z0] = toTerrain(PAD.x0 - RAMP, PAD.z0 - RAMP);
  const [x1, z1] = toTerrain(PAD.x1 + RAMP, PAD.z1 + RAMP);
  let lo = Infinity;
  let hi = -Infinity;
  let slope = 0;
  for (let z = z0; z <= z1; z += 1) {
    for (let x = x0; x <= x1; x += 1) {
      const h = island(x, z);
      lo = Math.min(lo, h);
      hi = Math.max(hi, h);
      slope = Math.max(slope, Math.hypot(island(x + 1, z) - h, island(x, z + 1) - h));
    }
  }
  if (hi - lo > 3 || slope > 0.15) throw new Error(`the pad's ground is too rough: ${(hi - lo).toFixed(2)} m relief, ${(slope * 100).toFixed(1)}% grade`);
  const [bx0, bz0] = toTerrain(PAD.x0 - BAND, PAD.z0 - BAND);
  const [bx1, bz1] = toTerrain(PAD.x1 + BAND, PAD.z1 + BAND);
  let yMin = Infinity;
  let yMax = -Infinity;
  for (let j = column(bz0); j <= column(bz1); j += 1) {
    for (let i = column(bx0); i <= column(bx1); i += 1) {
      if (fromPad(at(i), at(j)) > BAND) continue;
      yMin = Math.min(yMin, sample(i, j));
      yMax = Math.max(yMax, sample(i, j));
    }
  }
  return { steepest, relief: hi - lo, slope, yMin, yMax };
};

const num = (v, digits = 3) => {
  const s = (Math.round(v * 10 ** digits) / 10 ** digits).toString();
  return s.includes(".") ? s : `${s}.0`;
};

const main = () => {
  generate();
  const s = survey();
  const pixels = Buffer.alloc(SAMPLES * SAMPLES * 2);
  levels.forEach((l, k) => pixels.writeUInt16BE(l, k * 2));
  const png = encodePng(SAMPLES, SAMPLES, 16, 0, pixels);
  writeFileSync(fileURLToPath(new URL("island.png", import.meta.url)), png);

  const fun = `// islandLayout.fun — GENERATED by island.mjs (seed ${SEED}); do not edit.
// Rerun the script after changing the island; it rewrites this and island.png.
// Heights are the terrain's own (its samples' range); x and z are the block's.

let size = ${num(SIZE)} // m, square
let samples = ${num(SAMPLES)} // per side, ${num(CELL)} m apart
let minHeight = ${num(MIN_HEIGHT)}
let maxHeight = ${num(MAX_HEIGHT)}
let seaLevel = ${num(SEA_LEVEL)}
// Where the terrain's centre sits.
let centre = { x: ${num(CENTRE.x)}, z: ${num(CENTRE.z)} }
// The flat pad (every sample one level, height y), and the ramp around it.
let pad = { x0: ${num(PAD.x0)}, z0: ${num(PAD.z0)}, x1: ${num(PAD.x1)}, z1: ${num(PAD.z1)}, y: ${num(PAD_Y, 6)} }
let ramp = ${num(RAMP)}
// The ground's range out to \`reach\` past the pad's edge.
let band = { reach: ${num(BAND)}, yMin: ${num(s.yMin, 6)}, yMax: ${num(s.yMax, 6)} }
`;
  writeFileSync(fileURLToPath(new URL("islandLayout.fun", import.meta.url)), fun);

  console.log(
    `wrote ${SAMPLES}² heightmap (${png.length} bytes); steepest grade ${(s.steepest * 100).toFixed(1)}%; ` +
      `under the pad ${s.relief.toFixed(2)} m relief, ${(s.slope * 100).toFixed(1)}% grade before flattening; ` +
      `pad ${PAD_Y.toFixed(3)} m, band ${s.yMin.toFixed(3)}–${s.yMax.toFixed(3)} m`,
  );
};

// Run as a script; imported (by the e2e), only the ground function.
if (process.argv[1] === fileURLToPath(import.meta.url)) main();
