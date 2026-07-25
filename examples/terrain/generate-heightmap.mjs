// Deterministically generate the sample's 16-bit grayscale PNG using only
// Node built-ins. Re-run with:
//
//   node examples/terrain/generate-heightmap.mjs
//
// The field is ridged/domain-warped fBm followed by a droplet hydraulic-erosion
// pass. That combination — rather than a sum of sine waves — is what gives the
// terrain a spectrum: ridgelines to catch light, concavities for shading, and
// slope discontinuities for the rock band to snap to. Every procedural signal
// downstream (band placement, normals, future baked occlusion) is derived from
// this field, so its frequency content bounds all of them.
//
// The checked-in 1024² map keeps the example download compact. For a shipping
// 4 km world, author a 4096² source (roughly one height sample per metre); the
// Terrain API and renderer do not change. Physics still caps collision at
// 1025 samples per axis (about four-metre spacing across 4 km); see
// `Physics.heightfield`.
import { deflateSync } from "node:zlib";
import { writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const size = 1024;
const raw = Buffer.alloc((size * 2 + 1) * size);

const clamp = (value, lo, hi) => Math.max(lo, Math.min(hi, value));
const smoothstep = (edge0, edge1, value) => {
  const t = clamp((value - edge0) / (edge1 - edge0), 0, 1);
  return t * t * (3 - 2 * t);
};
const lerp = (a, b, t) => a + (b - a) * t;

// Everything random here is a pure function of a fixed seed, so re-running the
// script reproduces the committed PNG byte for byte.
const SEED = 0x5eed_1a3f;
const mulberry32 = (a) => () => {
  a = (a + 0x6d2b79f5) | 0;
  let t = Math.imul(a ^ (a >>> 15), 1 | a);
  t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
  return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
};

// Integer-lattice value noise. Cheap, and with enough octaves it is
// indistinguishable from gradient noise at terrain scale.
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

const fbm = (x, y, octaves, lacunarity = 2.03, gain = 0.5) => {
  let sum = 0;
  let amplitude = 1;
  let total = 0;
  let fx = x;
  let fy = y;
  for (let o = 0; o < octaves; o += 1) {
    sum += valueNoise(fx, fy) * amplitude;
    total += amplitude;
    amplitude *= gain;
    fx *= lacunarity;
    fy *= lacunarity;
  }
  return sum / total;
};

// Ridged multifractal: 1 - |signal| sharpens each octave into a crest, and
// weighting by the previous octave keeps detail on the ridges instead of
// spraying it uniformly. This is what produces a jagged silhouette.
const ridged = (x, y, octaves, lacunarity = 2.07, gain = 0.5) => {
  let sum = 0;
  let amplitude = 0.5;
  let total = 0;
  let weight = 1;
  let fx = x;
  let fy = y;
  for (let o = 0; o < octaves; o += 1) {
    let signal = 1 - Math.abs(valueNoise(fx, fy) * 2 - 1);
    signal *= signal;
    signal *= weight;
    weight = clamp(signal * 2.2, 0, 1);
    sum += signal * amplitude;
    total += amplitude;
    amplitude *= gain;
    fx *= lacunarity;
    fy *= lacunarity;
  }
  return sum / total;
};

const height = new Float32Array(size * size);

for (let y = 0; y < size; y += 1) {
  for (let x = 0; x < size; x += 1) {
    const nx = (x / (size - 1)) * 2 - 1;
    const nz = (y / (size - 1)) * 2 - 1;

    // Domain warp: displacing the sample point with its own noise bends the
    // ridges into curved chains instead of a uniform lattice of bumps.
    const warpX = fbm(nx * 1.7 + 11.3, nz * 1.7 - 4.1, 4) - 0.5;
    const warpZ = fbm(nx * 1.7 - 7.9, nz * 1.7 + 2.7, 4) - 0.5;
    const wx = nx + warpX * 0.55;
    const wz = nz + warpZ * 0.55;

    // Continental base: broad landmass shape.
    const continent = fbm(wx * 1.1 + 3.0, wz * 1.1 - 1.5, 7) * 0.42;

    // A mountain belt running across the map, with ridges only inside it.
    const beltAxis = (wx + 0.15) * 1.15 - (wz - 0.05) * 0.55;
    const belt = Math.exp(-beltAxis * beltAxis * 2.6);
    const peaks = ridged(wx * 2.4 + 5.5, wz * 2.4 + 8.2, 8) * belt * 0.62;

    // Fine roughness everywhere, so no slope is perfectly smooth.
    const roughness = (fbm(wx * 9.0, wz * 9.0, 5) - 0.5) * 0.05;

    // Coast: fall away near the edges so the map reads as an island rather
    // than a cropped rectangle, and the water plane has a shoreline to meet.
    const edge = Math.max(Math.abs(nx), Math.abs(nz));
    const coast = smoothstep(0.62, 1.0, edge);

    const h = (0.20 + continent + peaks + roughness) * (1 - coast * 0.86);
    height[y * size + x] = h;
  }
}

// ── Hydraulic erosion ────────────────────────────────────────────────────────
// Droplets that pick up sediment on descent and drop it where they slow down.
// This is what turns fBm from "noise" into "terrain": it carves connected
// drainage channels and deposits fans, which noise alone never produces.
const DROPLETS = 260_000;
const MAX_STEPS = 34;
const INERTIA = 0.06;
const CAPACITY = 3.4;
const MIN_SLOPE = 0.008;
const ERODE_RATE = 0.32;
const DEPOSIT_RATE = 0.22;
const EVAPORATE = 0.02;
const GRAVITY = 5.0;
const RADIUS = 3;

// Precomputed falloff so a droplet erodes a small disc rather than one texel,
// which would just add single-pixel noise.
const brush = [];
for (let dy = -RADIUS; dy <= RADIUS; dy += 1) {
  for (let dx = -RADIUS; dx <= RADIUS; dx += 1) {
    const d = Math.hypot(dx, dy);
    if (d <= RADIUS) brush.push({ dx, dy, w: 1 - d / RADIUS });
  }
}
const brushTotal = brush.reduce((sum, b) => sum + b.w, 0);

const sampleHeight = (x, y) => {
  const ix = Math.floor(x);
  const iy = Math.floor(y);
  const fx = x - ix;
  const fy = y - iy;
  const i = iy * size + ix;
  const h00 = height[i];
  const h10 = height[i + 1];
  const h01 = height[i + size];
  const h11 = height[i + size + 1];
  return {
    height:
      h00 * (1 - fx) * (1 - fy) +
      h10 * fx * (1 - fy) +
      h01 * (1 - fx) * fy +
      h11 * fx * fy,
    gradX: (h10 - h00) * (1 - fy) + (h11 - h01) * fy,
    gradY: (h01 - h00) * (1 - fx) + (h11 - h10) * fx,
  };
};

const deposit = (x, y, amount) => {
  const ix = Math.floor(x);
  const iy = Math.floor(y);
  const fx = x - ix;
  const fy = y - iy;
  const i = iy * size + ix;
  height[i] += amount * (1 - fx) * (1 - fy);
  height[i + 1] += amount * fx * (1 - fy);
  height[i + size] += amount * (1 - fx) * fy;
  height[i + size + 1] += amount * fx * fy;
};

const random = mulberry32(SEED);
for (let drop = 0; drop < DROPLETS; drop += 1) {
  let x = random() * (size - 2 * RADIUS - 2) + RADIUS + 1;
  let y = random() * (size - 2 * RADIUS - 2) + RADIUS + 1;
  let dirX = 0;
  let dirY = 0;
  let speed = 1;
  let water = 1;
  let sediment = 0;

  for (let step = 0; step < MAX_STEPS; step += 1) {
    const here = sampleHeight(x, y);
    dirX = dirX * INERTIA - here.gradX * (1 - INERTIA);
    dirY = dirY * INERTIA - here.gradY * (1 - INERTIA);
    const len = Math.hypot(dirX, dirY);
    if (len < 1e-6) break;
    dirX /= len;
    dirY /= len;
    const nextX = x + dirX;
    const nextY = y + dirY;
    if (
      nextX < RADIUS + 1 ||
      nextX >= size - RADIUS - 2 ||
      nextY < RADIUS + 1 ||
      nextY >= size - RADIUS - 2
    ) {
      break;
    }

    const next = sampleHeight(nextX, nextY);
    const delta = next.height - here.height;
    const capacity = Math.max(-delta, MIN_SLOPE) * speed * water * CAPACITY;

    if (sediment > capacity || delta > 0) {
      // Uphill: drop enough to fill the pit, never more than carried.
      const drop_ = delta > 0
        ? Math.min(delta, sediment)
        : (sediment - capacity) * DEPOSIT_RATE;
      sediment -= drop_;
      deposit(x, y, drop_);
    } else {
      const take = Math.min((capacity - sediment) * ERODE_RATE, -delta);
      let removed = 0;
      const ix = Math.round(x);
      const iy = Math.round(y);
      for (const b of brush) {
        const idx = (iy + b.dy) * size + (ix + b.dx);
        const amount = (take * b.w) / brushTotal;
        height[idx] -= amount;
        removed += amount;
      }
      sediment += removed;
    }

    speed = Math.sqrt(Math.max(0, speed * speed + -delta * GRAVITY));
    water *= 1 - EVAPORATE;
    x = nextX;
    y = nextY;
  }
}

// ── Encode ───────────────────────────────────────────────────────────────────
// Rescale to use the full 16-bit range so `Terrain.heightmap`'s min/max map
// onto the actual extremes rather than a compressed middle.
let lo = Infinity;
let hi = -Infinity;
for (const h of height) {
  if (h < lo) lo = h;
  if (h > hi) hi = h;
}
const span = Math.max(hi - lo, 1e-6);

for (let y = 0; y < size; y += 1) {
  const row = y * (size * 2 + 1);
  raw[row] = 0; // PNG "None" filter
  for (let x = 0; x < size; x += 1) {
    const normalized = clamp((height[y * size + x] - lo) / span, 0, 1);
    raw.writeUInt16BE(Math.round(normalized * 65535), row + 1 + x * 2);
  }
}

const crcTable = new Uint32Array(256);
for (let n = 0; n < 256; n += 1) {
  let c = n;
  for (let k = 0; k < 8; k += 1) {
    c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  }
  crcTable[n] = c >>> 0;
}

const crc32 = (bytes) => {
  let crc = 0xffffffff;
  for (const byte of bytes) {
    crc = crcTable[(crc ^ byte) & 0xff] ^ (crc >>> 8);
  }
  return (crc ^ 0xffffffff) >>> 0;
};

const chunk = (name, data) => {
  const type = Buffer.from(name, "ascii");
  const body = Buffer.concat([type, data]);
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
ihdr[10] = 0; // compression
ihdr[11] = 0; // filter
ihdr[12] = 0; // no interlace

const png = Buffer.concat([
  Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
  chunk("IHDR", ihdr),
  chunk("IDAT", deflateSync(raw, { level: 9 })),
  chunk("IEND", Buffer.alloc(0)),
]);
writeFileSync(fileURLToPath(new URL("heightmap.png", import.meta.url)), png);
console.log(`wrote ${size}x${size} 16-bit heightmap (${png.length} bytes)`);
