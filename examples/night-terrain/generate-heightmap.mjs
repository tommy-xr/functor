// Deterministically generate the sample's 16-bit grayscale heightmap using
// only Node built-ins. Re-run with:
//
//   node examples/night-terrain/generate-heightmap.mjs
//
// Rolling, domain-warped fBm hills: open ground to cross on foot, with enough
// relief to hide what is over the next rise. 1024² over 4 km is about one
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

// Integer-lattice value noise; a pure function of SEED, so re-running the
// script reproduces the committed PNG byte for byte.
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
  return lerp(
    lerp(hash2(ix, iy), hash2(ix + 1, iy), ux),
    lerp(hash2(ix, iy + 1), hash2(ix + 1, iy + 1), ux),
    uy,
  );
};

const fbm = (x, y, octaves) => {
  let sum = 0;
  let amplitude = 1;
  let total = 0;
  for (let o = 0; o < octaves; o += 1) {
    const f = 2.03 ** o;
    sum += valueNoise(x * f, y * f) * amplitude;
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
    const nx = (x / (size - 1)) * 2 - 1;
    const nz = (y / (size - 1)) * 2 - 1;
    // Domain warp bends the hills into curved ridges rather than a lattice.
    const wx = nx + (fbm(nx * 1.3 + 9.1, nz * 1.3 - 3.7, 4) - 0.5) * 0.6;
    const wz = nz + (fbm(nx * 1.3 - 5.3, nz * 1.3 + 6.2, 4) - 0.5) * 0.6;
    const hills = fbm(wx * 3.2 + 1.7, wz * 3.2 - 2.3, 7);
    // Squaring flattens the lowlands into open fields between the hills.
    const shaped = hills * hills;
    // Raise the rim so the map ends in a ring of high ground, not a cliff.
    const edge = Math.max(Math.abs(nx), Math.abs(nz));
    const h = shaped + smoothstep(0.8, 1.0, edge) * 0.35;
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
