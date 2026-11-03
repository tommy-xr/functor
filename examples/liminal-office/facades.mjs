// Generate the exterior finishes' textures (finishes.fun): one tileable
// `facade-<name>.png` per finish, near-white so each archetype's tint gives
// its colour. Node built-ins only; re-running reproduces every file byte for
// byte (with the same Node: its zlib does the deflate):
//
//   node examples/liminal-office/facades.mjs
//
// Each image is one repeat of its finish (`tile` metres in finishes.fun),
// top row up. Row 0 lies on a storey line where the finish has one.
import { writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { encodePng } from "../../scripts/png.mjs";

// Deterministic hash noise in [0, 1).
const hash = (x, y, seed) => {
  let h = (Math.imul(x, 374761393) + Math.imul(y, 668265263) + Math.imul(seed, 1442695041)) | 0;
  h = Math.imul(h ^ (h >>> 13), 1274126177);
  return ((h ^ (h >>> 16)) >>> 0) / 4294967296;
};
const smooth = (t) => t * t * (3 - 2 * t);
// Value noise of `cells` cells across an `n`-texel image, wrapping, so it tiles.
const valueNoise = (x, y, n, cells, seed) => {
  const fx = (x / n) * cells;
  const fy = (y / n) * cells;
  const x0 = Math.floor(fx);
  const y0 = Math.floor(fy);
  const at = (i, j) => hash(((i % cells) + cells) % cells, ((j % cells) + cells) % cells, seed);
  const tx = smooth(fx - x0);
  const ty = smooth(fy - y0);
  const a = at(x0, y0) + (at(x0 + 1, y0) - at(x0, y0)) * tx;
  const b = at(x0, y0 + 1) + (at(x0 + 1, y0 + 1) - at(x0, y0 + 1)) * tx;
  return a + (b - a) * ty;
};
// Wrapping fBm: octaves of value noise from `cells` cells up.
const fbm = (x, y, n, cells, seed, octaves = 4) => {
  let sum = 0;
  let amp = 0.5;
  let norm = 0;
  for (let o = 0; o < octaves; o += 1) {
    sum += amp * valueNoise(x, y, n, cells << o, seed + o);
    norm += amp;
    amp /= 2;
  }
  return sum / norm;
};

// Each finish: its size in texels and a texel's grey level (0–1, before
// the tint) or [r, g, b].
const FINISHES = {
  // Office: storey-high precast panels, 1.65 m wide (two per 3.3 m repeat),
  // with dark reveals, a lit lip under each horizontal one, mottling, a
  // little grit and streaks running down from the reveals.
  precast: {
    size: 256,
    texel: (x, y, n) => {
      const panel = Math.floor(x / (n / 2));
      const px = x % (n / 2);
      if (y < 3 || px < 3) return 0.42;
      if (y < 5) return 0.98;
      const streak = hash(x >> 2, 0, 11) < 0.18 ? 0.05 * Math.max(0, 1 - y / (n * 0.7)) : 0;
      const mottle = 0.06 * (fbm(x, y, n, 4, 3) - 0.5);
      return 0.86 + 0.03 * (hash(panel, 0, 5) - 0.5) + mottle + 0.05 * (hash(x, y, 7) - 0.5) - streak;
    },
  },
  // Tower: a curtain wall's metal ribs every 0.41 m (eight per repeat), a
  // spandrel band 0.6 m deep over each storey line with a seam along it,
  // and dark glass between.
  curtain: {
    size: 256,
    texel: (x, y, n) => {
      const rib = x % (n / 8);
      const band = y < 24 || y >= n - 24;
      if (rib < 3) return 0.82 - (rib === 2 ? 0.12 : 0);
      if (y === 0 || y === 1) return 0.38;
      if (band) return 0.7 + 0.04 * (fbm(x, y, n, 8, 21) - 0.5);
      if (y === 24 || y === n - 25) return 0.3;
      return 0.36 + 0.06 * (fbm(x, y, n, 2, 23) - 0.5) + 0.03 * (y / n);
    },
  },
  // Cinema: painted stucco, a fine trowelled grain over a soft mottle.
  stucco: {
    size: 128,
    texel: (x, y, n) => 0.88 + 0.08 * (fbm(x, y, n, 16, 31) - 0.5) + 0.07 * (hash(x, y, 37) - 0.5),
  },
  // The cinema's base: 15 cm glazed tiles (four per 0.6 m repeat) in grout.
  tiles: {
    size: 64,
    texel: (x, y, n) => {
      const t = n / 4;
      if (x % t < 1 || y % t < 1) return 0.55;
      const glaze = 0.06 * (hash(Math.floor(x / t), Math.floor(y / t), 41) - 0.5);
      return 0.86 + glaze + 0.02 * (hash(x, y, 43) - 0.5) + (y % t === 1 ? 0.06 : 0);
    },
  },
  // Sidewalks: 1.5 m concrete slabs (four per 3 m repeat) in sawn joints,
  // each slab a shade apart, with grit and a few gum-dark spots.
  paving: {
    size: 128,
    texel: (x, y, n) => {
      const slab = n / 2;
      if (x % slab < 2 || y % slab < 2) return 0.5;
      const tone = 0.05 * (hash(Math.floor(x / slab), Math.floor(y / slab), 61) - 0.5);
      const spot = hash(x >> 1, y >> 1, 67) < 0.004 ? 0.25 : 0;
      return 0.86 + tone + 0.06 * (fbm(x, y, n, 4, 63) - 0.5) + 0.06 * (hash(x, y, 65) - 0.5) - spot;
    },
  },
  // Flat roofs: membrane in 1 m strips (four per 4 m repeat) with lapped
  // seams, gravel grit and ponding stains.
  roofing: {
    size: 128,
    texel: (x, y, n) => {
      const strip = n / 4;
      if (y % strip < 1) return 0.62;
      if (y % strip < 2) return 0.95;
      const stain = fbm(x, y, n, 4, 51) < 0.38 ? 0.08 : 0;
      return 0.84 + 0.1 * (hash(x, y, 53) - 0.5) - stain;
    },
  },
  // Mall: split-face concrete block, 40 × 20 cm (four by eight per 1.6 m
  // repeat) in running bond: mortar joints, a rough split face, a little
  // tone between blocks and a lit lip under each bed joint.
  splitface: {
    size: 128,
    texel: (x, y, n) => {
      const w = n / 4;
      const h = n / 8;
      const course = Math.floor(y / h);
      const u = (x + (course % 2) * (w / 2)) % n;
      if (y % h < 2 || u % w < 2) return 0.58;
      const tone = 0.06 * (hash(Math.floor(u / w), course, 61) - 0.5);
      const split = 0.14 * (fbm(x, y, n, 32, 63) - 0.5) + 0.08 * (hash(x, y, 67) - 0.5);
      return 0.84 + tone + split + (y % h === 2 ? 0.05 : 0);
    },
  },
  // The mall's fascia: ribbed metal, a standing seam every 0.5 m (two per
  // 1 m repeat) lit along one edge, brushed streaks between.
  fascia: {
    size: 64,
    texel: (x, y, n) => {
      const seam = x % (n / 2);
      if (seam < 1) return 0.5;
      if (seam < 2) return 0.96;
      return 0.8 + 0.04 * (fbm(x, y, n, 4, 71) - 0.5) + 0.03 * (hash(0, y, 73) - 0.5);
    },
  },
  // Houses: clapboard, 15 cm boards (eight per 1.2 m repeat), each in the
  // shadow of the one lapped over it at its top and paling down to its
  // edge, with a faint grain.
  clapboard: {
    size: 128,
    texel: (x, y, n) => {
      const h = n / 8;
      const r = y % h;
      if (r < 2) return 0.55;
      return 0.86 + 0.08 * (r / h) + 0.03 * (fbm(x, y, n, 8, 81) - 0.5) + 0.02 * (hash(x >> 4, y, 83) - 0.5);
    },
  },
  // Houses' gables: asphalt shingles, 14 cm courses (eight per 1.12 m
  // repeat) of 28 cm tabs staggered by half a tab: slots between the tabs,
  // a shadow along each course's edge, a tone per tab and granules.
  shingles: {
    size: 128,
    texel: (x, y, n) => {
      const h = n / 8;
      const w = n / 4;
      const course = Math.floor(y / h);
      const u = (x + (course % 2) * (w / 2)) % n;
      const r = y % h;
      if (r >= h - 2) return 0.45;
      if (u % w < 2 && r > h / 3) return 0.42;
      const tone = 0.08 * (hash(Math.floor(u / w), course, 91) - 0.5);
      return 0.8 + tone + 0.12 * (hash(x, y, 93) - 0.5);
    },
  },
};

const gray = (v) => Math.max(0, Math.min(255, Math.round(v * 255)));
for (const [name, { size, texel }] of Object.entries(FINISHES)) {
  const pixels = new Uint8Array(size * size * 3);
  for (let y = 0; y < size; y += 1) {
    for (let x = 0; x < size; x += 1) {
      const v = texel(x, y, size);
      const [r, g, b] = Array.isArray(v) ? v : [v, v, v];
      pixels.set([gray(r), gray(g), gray(b)], (y * size + x) * 3);
    }
  }
  writeFileSync(fileURLToPath(new URL(`facade-${name}.png`, import.meta.url)), encodePng(size, size, 8, 2, pixels));
}
