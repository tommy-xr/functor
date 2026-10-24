// Deterministically generate the sample's 16-bit grayscale heightmap using
// only Node built-ins. Re-run with:
//
//   node examples/night-terrain/generate-heightmap.mjs
//
// Three biomes from seeded, domain-warped Perlin fBm, laid out so the
// renderer's height/slope bands land on them: a flat valley floor in the
// south-west (the low band), rolling hills across the middle (the high band),
// and a ridged mountain range to the north whose steep faces take the rock
// band and whose peaks clear the snow line. 1024² over 4 km is about one
// sample per four metres — the most `Physics.heightfield` collides against.
import { deflateSync } from "node:zlib";
import { writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const size = 1024;
const SEED = 0x6e16_7a1e;

const lerp = (a, b, t) => a + (b - a) * t;
const smoothstep = (edge0, edge1, value) => {
  const t = Math.max(0, Math.min(1, (value - edge0) / (edge1 - edge0)));
  return t * t * (3 - 2 * t);
};

// Seeded permutation for classic 2D Perlin noise: a pure function of SEED, so
// re-running the script reproduces the committed PNG byte for byte.
const mulberry32 = (a) => () => {
  a = (a + 0x6d2b79f5) | 0;
  let t = Math.imul(a ^ (a >>> 15), 1 | a);
  t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
  return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
};
const random = mulberry32(SEED);
const perm = new Uint8Array(512);
{
  const p = Array.from({ length: 256 }, (_, i) => i);
  for (let i = 255; i > 0; i -= 1) {
    const j = Math.floor(random() * (i + 1));
    [p[i], p[j]] = [p[j], p[i]];
  }
  for (let i = 0; i < 512; i += 1) perm[i] = p[i & 255];
}
const GRADIENTS = [
  [1, 0], [-1, 0], [0, 1], [0, -1],
  [Math.SQRT1_2, Math.SQRT1_2], [-Math.SQRT1_2, Math.SQRT1_2],
  [Math.SQRT1_2, -Math.SQRT1_2], [-Math.SQRT1_2, -Math.SQRT1_2],
];
const fade = (t) => t * t * t * (t * (t * 6 - 15) + 10);
const grad = (ix, iy, dx, dy) => {
  const g = GRADIENTS[perm[(ix & 255) + perm[iy & 255]] & 7];
  return g[0] * dx + g[1] * dy;
};
// Roughly -0.7..0.7.
const perlin = (x, y) => {
  const ix = Math.floor(x);
  const iy = Math.floor(y);
  const fx = x - ix;
  const fy = y - iy;
  const u = fade(fx);
  const v = fade(fy);
  return lerp(
    lerp(grad(ix, iy, fx, fy), grad(ix + 1, iy, fx - 1, fy), u),
    lerp(grad(ix, iy + 1, fx, fy - 1), grad(ix + 1, iy + 1, fx - 1, fy - 1), u),
    v,
  );
};

// Signed fBm, roughly -0.5..0.5.
const fbm = (x, y, octaves) => {
  let sum = 0;
  let amplitude = 1;
  let total = 0;
  for (let o = 0; o < octaves; o += 1) {
    const f = 2.03 ** o;
    sum += perlin(x * f + o * 17.3, y * f - o * 9.1) * amplitude;
    total += amplitude;
    amplitude *= 0.5;
  }
  return sum / total;
};

// Ridged multifractal: 1 - |noise| sharpens each octave into a crest, and
// weighting by the previous octave keeps detail on the ridges. 0..1.
const ridged = (x, y, octaves) => {
  let sum = 0;
  let amplitude = 0.5;
  let total = 0;
  let weight = 1;
  for (let o = 0; o < octaves; o += 1) {
    const f = 2.07 ** o;
    let signal = 1 - Math.abs(perlin(x * f + o * 5.7, y * f + o * 3.3) * 1.4);
    signal *= signal * weight;
    weight = Math.max(0, Math.min(1, signal * 2));
    sum += signal * amplitude;
    total += amplitude;
    amplitude *= 0.5;
  }
  return sum / total;
};

const height = new Float64Array(size * size);
let lo = Infinity;
let hi = -Infinity;
for (let y = 0; y < size; y += 1) {
  for (let x = 0; x < size; x += 1) {
    // +X east, +Z north (image rows run with world Z).
    const nx = (x / (size - 1)) * 2 - 1;
    const nz = (y / (size - 1)) * 2 - 1;
    // Domain warp bends ridges and shorelines into curves, not a lattice.
    const wx = nx + fbm(nx * 1.4 + 9.1, nz * 1.4 - 3.7, 4) * 0.45;
    const wz = nz + fbm(nx * 1.4 - 5.3, nz * 1.4 + 6.2, 4) * 0.45;

    // Biome masks over the warped plane: mountains to the north, the valley
    // floor to the south-west, hills between.
    const mountains = smoothstep(0.05, 0.6, wz + wx * 0.25);
    const valley = smoothstep(0.0, 0.7, -(wx + wz) * 0.7 - 0.05) * (1 - mountains);

    const hills = 0.24 + fbm(wx * 3.0 + 1.7, wz * 3.0 - 2.3, 6) * 0.34;
    const floor = 0.035 + fbm(wx * 5.0, wz * 5.0, 4) * 0.02;
    const range = ridged(wx * 2.6 + 4.4, wz * 2.6 - 1.2, 8) * 0.8;

    const h = lerp(hills, floor, valley) + mountains * range;
    height[y * size + x] = h;
    if (h < lo) lo = h;
    if (h > hi) hi = h;
  }
}

// Rescale to the full 16-bit range so `Terrain.heightmap`'s min/max map onto
// the actual extremes.
const raw = Buffer.alloc((size * 2 + 1) * size);
for (let y = 0; y < size; y += 1) {
  const row = y * (size * 2 + 1);
  raw[row] = 0; // PNG "None" filter
  for (let x = 0; x < size; x += 1) {
    const normalized = (height[y * size + x] - lo) / (hi - lo);
    raw.writeUInt16BE(Math.round(normalized * 65535), row + 1 + x * 2);
  }
}

const crcTable = new Uint32Array(256);
for (let n = 0; n < 256; n += 1) {
  let c = n;
  for (let k = 0; k < 8; k += 1) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  crcTable[n] = c >>> 0;
}
const crc32 = (bytes) => {
  let crc = 0xffffffff;
  for (const byte of bytes) crc = crcTable[(crc ^ byte) & 0xff] ^ (crc >>> 8);
  return (crc ^ 0xffffffff) >>> 0;
};
const chunk = (name, data) => {
  const body = Buffer.concat([Buffer.from(name, "ascii"), data]);
  const result = Buffer.alloc(12 + data.length);
  result.writeUInt32BE(data.length, 0);
  body.copy(result, 4);
  result.writeUInt32BE(crc32(body), 8 + data.length);
  return result;
};

const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(size, 0);
ihdr.writeUInt32BE(size, 4);
ihdr[8] = 16; // bit depth
ihdr[9] = 0; // grayscale

const png = Buffer.concat([
  Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
  chunk("IHDR", ihdr),
  chunk("IDAT", deflateSync(raw, { level: 9 })),
  chunk("IEND", Buffer.alloc(0)),
]);
writeFileSync(fileURLToPath(new URL("night-heightmap.png", import.meta.url)), png);
console.log(`wrote ${size}x${size} 16-bit heightmap (${png.length} bytes)`);
