// Generate the island the block stands on: a 16-bit heightmap PNG,
// `islandLayout.fun` (the generated constants the game places it with) and
// the terrain's colormaps and normal map (see Colormaps). Node built-ins
// only; re-running reproduces every file byte for byte (with the same Node:
// its zlib does the deflate):
//
//   node examples/liminal-office/island.mjs                   # the committed island (mixed), every biome
//   node examples/liminal-office/island.mjs --preset alpine   # or rolling: overwrites every file
//   node examples/liminal-office/island.mjs --biome snow      # only that biome's maps (quicker, to tune a palette)
//
// 2049² samples over 2048 m: one sample per metre, so every terrain LOD
// vertex lands on a sample. `Physics.heightfield` keeps every other one
// (1025 per axis, 2 m cells).
//
// The island: a seabed, a beach at sea level, and inland a mix of regions
// picked by a warped low-frequency mask: plains and rolling hills (smooth
// gradient-noise fBm) and rocky mountains (ridged multifractal), with a
// ridge 300–450 m north of the town so the street sees peaks. The block
// stands on a flat pad (one u16 level) in the south of the island, calm
// plains around it and a smoothstep ramp. Out to `BAND` past the pad's edge
// the ground stays between the pad and `CAP` above it, so the block's
// collars (0–8 m over the street) hold every eye there.
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { encodePng } from "../../scripts/png.mjs";

// A/B presets: how much of the island is mountain, and how high it goes.
// `cover` shifts the region mask (plains < hills < mountains), `peak` scales
// the ridges, `massif` the bump north of the town.
const PRESETS = {
  rolling: { cover: -0.12, peak: 120, massif: 0.6 },
  mixed: { cover: 0, peak: 210, massif: 1 },
  alpine: { cover: 0.14, peak: 250, massif: 1 },
};
// Biomes: the colormaps' palette (sRGB, like `Color.rgb`), independent of
// the terrain's shape. `rock` is the grade where bare rock starts; `snowLine`
// the height snow starts on gentle ground (none above the peak: no snow);
// `lawn` a park's mown grass (snowed over in the snow biome; watered in the
// desert).
// island.fun pairs each with its detail texture.
const BIOMES = {
  grass: {
    lush: [0.33, 0.36, 0.21], dry: [0.42, 0.4, 0.27], hill: [0.4, 0.39, 0.25], gravel: [0.4, 0.37, 0.31],
    dirt: [0.39, 0.32, 0.24], road: [0.32, 0.31, 0.3], sand: [0.62, 0.57, 0.46], wet: [0.42, 0.39, 0.32],
    scree: [0.47, 0.46, 0.43], rock: [0.37, 0.35, 0.33], snow: [0.86, 0.87, 0.9], rockGrade: 0.75, snowLine: 185,
    lawn: [0.32, 0.38, 0.2],
  },
  snow: {
    lush: [0.67, 0.69, 0.73], dry: [0.63, 0.65, 0.69], hill: [0.69, 0.7, 0.74], gravel: [0.6, 0.61, 0.64],
    dirt: [0.45, 0.43, 0.41], road: [0.38, 0.37, 0.37], sand: [0.36, 0.35, 0.34], wet: [0.27, 0.27, 0.27],
    scree: [0.6, 0.61, 0.64], rock: [0.32, 0.31, 0.3], snow: [0.72, 0.73, 0.77], rockGrade: 0.65, snowLine: 120,
    lawn: [0.74, 0.76, 0.8],
  },
  // Cracked hardpan on the flats, rippled sand on the hills, warm gravel on
  // slopes and scree, red rock; no snow.
  desert: {
    lush: [0.66, 0.55, 0.42], dry: [0.71, 0.59, 0.43], hill: [0.73, 0.59, 0.42], gravel: [0.55, 0.43, 0.33],
    dirt: [0.56, 0.46, 0.35], road: [0.5, 0.44, 0.37], sand: [0.76, 0.69, 0.55], wet: [0.55, 0.49, 0.38],
    scree: [0.57, 0.45, 0.35], rock: [0.5, 0.37, 0.29], snow: [0.64, 0.54, 0.44], rockGrade: 0.75, snowLine: 1000,
    lawn: [0.36, 0.42, 0.22],
  },
};
// Run as a script: `[--preset <name>] [--biome <name>]`; without `--biome`
// every biome's maps are painted. Imported: `mixed`.
const RUN = process.argv[1] === fileURLToPath(import.meta.url);
const args = RUN ? process.argv.slice(2) : [];
const OPTIONS = { preset: "mixed", biome: "" };
const usage = () => new Error(`usage: island.mjs [--preset ${Object.keys(PRESETS).join("|")}] [--biome ${Object.keys(BIOMES).join("|")}]`);
for (let k = 0; k < args.length; k += 2) {
  const name = args[k].slice(2);
  if (!args[k].startsWith("--") || !Object.hasOwn(OPTIONS, name) || k + 1 >= args.length) throw usage();
  OPTIONS[name] = args[k + 1];
}
const PRESET_NAME = OPTIONS.preset;
const PAINTED = OPTIONS.biome === "" ? Object.keys(BIOMES) : [OPTIONS.biome];
if (!Object.hasOwn(PRESETS, PRESET_NAME) || !PAINTED.every((b) => Object.hasOwn(BIOMES, b))) throw usage();
const PRESET = PRESETS[PRESET_NAME];
const SEED = 2049;
const SAMPLES = 2049;
const SIZE = 2048; // m, square
const CELL = SIZE / (SAMPLES - 1); // 1 m
// Black and white; 330 m over 65535 levels is 5 mm a level. The terrain's
// height bands span this range.
const MIN_HEIGHT = -30;
const MAX_HEIGHT = 300;
const SEA_LEVEL = 0;

// The pad, in the block's space: the four copies' footprints (x, z from -53
// to 29) and a 6 m apron. Its south-west corner sits at `PAD_LOCAL` in the
// terrain's space (even, so the 2 m collider's samples fall on its edges).
const PAD = { x0: -59, z0: -59, x1: 35, z1: 35 };
const PAD_LOCAL = { x: 12, z: -478 };
// Where the terrain's centre sits in the block's space.
const CENTRE = { x: PAD.x0 - PAD_LOCAL.x, z: PAD.z0 - PAD_LOCAL.z };
const PAD_MIDDLE = { x: (PAD.x0 + PAD.x1) / 2 - CENTRE.x, z: (PAD.z0 + PAD.z1) / 2 - CENTRE.z };
// The massif's middle, north (+z, the street camera's view) of the pad's.
const MASSIF = { x: PAD_MIDDLE.x - 40, z: PAD_MIDDLE.z + 440, radius: 380 };
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

// An integer-lattice hash, a pure function of the seed.
const hash = (ix, iy) => {
  let h = Math.imul(ix, 0x27d4eb2d) ^ Math.imul(iy, 0x165667b1) ^ SEED;
  h = Math.imul(h ^ (h >>> 15), 0x2c1b3c6d);
  h = Math.imul(h ^ (h >>> 12), 0x297a2d39);
  return (h ^ (h >>> 15)) >>> 0;
};
const hash2 = (ix, iy) => hash(ix, iy) / 4294967296;
// Gradient (Perlin) noise: a unit gradient per lattice point, one of 32
// directions; about -1..1, 0 on the lattice.
const GRADIENTS = Array.from({ length: 32 }, (_, k) => [Math.cos((k * Math.PI) / 16), Math.sin((k * Math.PI) / 16)]);
const corner = (ix, iy, fx, fy) => {
  const [gx, gy] = GRADIENTS[hash(ix, iy) >>> 27];
  return gx * fx + gy * fy;
};
const fade = (t) => t * t * t * (t * (t * 6 - 15) + 10);
const perlin = (x, y) => {
  const ix = Math.floor(x);
  const iy = Math.floor(y);
  const fx = x - ix;
  const fy = y - iy;
  const ux = fade(fx);
  const uy = fade(fy);
  const a = lerp(corner(ix, iy, fx, fy), corner(ix + 1, iy, fx - 1, fy), ux);
  const b = lerp(corner(ix, iy + 1, fx, fy - 1), corner(ix + 1, iy + 1, fx - 1, fy - 1), ux);
  return 1.4 * lerp(a, b, uy);
};
// Smooth fBm, about -0.5..0.5. Each octave is shifted so they don't share
// lattice points.
const fbm = (x, y, octaves = 4, gain = 0.45) => {
  let sum = 0;
  let amp = 1;
  let total = 0;
  for (let o = 0, f = 1; o < octaves; o += 1, f *= 2.03, amp *= gain) {
    sum += perlin(x * f + 17.1 * o, y * f - 9.3 * o) * amp;
    total += amp;
  }
  return (0.5 * sum) / total;
};
// Ridged multifractal, about 0..1: sharp crests where the noise crosses 0,
// each octave weighted by the one before so the valleys stay smooth and the
// crests rocky.
const ridged = (x, y, octaves = 6) => {
  let sum = 0;
  let amp = 1;
  let total = 0;
  let weight = 1;
  for (let o = 0, f = 1; o < octaves; o += 1, f *= 2.07, amp *= 0.5) {
    let n = 1 - Math.abs(perlin(x * f - 31.7 * o, y * f + 5.9 * o));
    n = n * n * weight;
    weight = clamp(2 * n, 0, 1);
    sum += n * amp;
    total += amp;
  }
  return sum / total;
};

// The island in the terrain's space: how far inland (x, z) is (negative out
// at sea), from a coastline 750–880 m out from the centre.
const inland = (x, z) => {
  const a = Math.atan2(z, x);
  const coast = 810 + 280 * fbm(2.2 * Math.cos(a) + 11.3, 2.2 * Math.sin(a) + 3.7, 3);
  return coast - Math.hypot(x, z);
};
// Rising from the waterline at its steepest, levelling off at `run`.
const shelf = (rise, run, d) => {
  const t = 1 - clamp(d / run, 0, 1);
  return rise * (1 - t * t * t);
};
// The region at (x, z), already warped: plains below 0.1, rolling hills
// from there, mountains rising over them from 0.35. Low-frequency noise, the
// preset's cover and the massif north of the town; plains near the pad and
// the coast.
const region = (x, z, s) => {
  const toPad = Math.hypot(x - PAD_MIDDLE.x, z - PAD_MIDDLE.z);
  const toMassif = Math.hypot(x - MASSIF.x, z - MASSIF.z) / MASSIF.radius;
  const m = 0.3 + PRESET.cover + 2.4 * fbm(x / 800 + 3.1, z / 800 - 7.4, 3) + 0.7 * PRESET.massif * (1 - smoothstep(0, 1, toMassif));
  const away = smoothstep(100, 330, toPad);
  return lerp(-0.3, m * smoothstep(20, 300, s), away);
};
// Seabed to 26 m down; a beach that meets the sea at a 14% grade (a
// flatter one blurs where the sea plane crosses it); 16 m up over the first
// 350 m inland. Inland, the region mask blends gentle plains, rolling
// hills, and ridged mountains, all over a warped domain so the borders
// wander.
// `islandAt` also returns what the colormaps paint from: `s` (inland) and
// the hill and mountain weights.
const islandAt = (x, z) => {
  const s = inland(x, z);
  if (s < 0) return { h: SEA_LEVEL - shelf(26, 300, -s), s, hills: 0, mount: 0 };
  const wx = x + 140 * fbm(x / 500 + 1.7, z / 500 + 9.2, 3);
  const wz = z + 140 * fbm(x / 500 - 4.4, z / 500 - 2.8, 3);
  const r = region(wx, wz, s);
  const land = smoothstep(30, 300, s);
  const plains = 16 * fbm(wx / 450 + 5.2, wz / 450 - 1.7, 4);
  const hillWeight = smoothstep(0.1, 0.5, r);
  // Mountains rise from their foothills: a smooth uplift, then ridges on it.
  const mountWeight = smoothstep(0.35, 1.1, r);
  const hills = hillWeight > 0 ? hillWeight * 60 * (fbm(wx / 240 - 3.3, wz / 240 + 8.8, 5, 0.5) + 0.5) : 0;
  const mountains = mountWeight > 0 ? PRESET.peak * mountWeight * (0.3 * mountWeight + 0.7 * ridged(wx / 300 + 2.9, wz / 300 - 5.7)) : 0;
  const calm = lerp(0.25, 1, smoothstep(80, 320, Math.hypot(x - PAD_MIDDLE.x, z - PAD_MIDDLE.z)));
  return { h: SEA_LEVEL + shelf(16, 350, s) + land * (plains * calm + hills + mountains), s, hills: hillWeight, mount: mountWeight };
};
const island = (x, z) => islandAt(x, z).h;

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

// The samples: allocated by `main`, so importing `ground` costs nothing.
let levels;
// Row j is z = -1024 + j (south first), column i is x = -1024 + i.
const at = (i) => -SIZE / 2 + i * CELL;
const generate = () => {
  levels = new Uint16Array(SAMPLES * SAMPLES);
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
  const highest = decode(levels.reduce((a, b) => Math.max(a, b)));
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
  return { steepest, highest, relief: hi - lo, slope, yMin, yMax };
};

// ---- Props ----------------------------------------------------------------------
//
// A few trees in groves and rocks scattered over the island, placed from the
// seed and stood on `ground`. None within CLEARING of the pad's edge, where
// the ground is held near the pad (the band and its blend); no trees on the
// beach, none above TREELINE; none on steep ground. The caps hold the Quest budget: at most
// NEAR_CAP trees within MESH_DISTANCE of any point (the near tier), at most
// TREE_CAP trees (so cards) and ROCK_CAP rocks in all.
const CLEARING = BAND + BLEND;
const MESH_DISTANCE = 40;
const NEAR_CAP = 10;
const TREE_CAP = 150;
const ROCK_CAP = 100;
// Rocks farther than this from the eye aren't drawn.
const ROCK_DISTANCE = 150;
const GROVES = 16;
const GROVE_TREES = 7;
const GROVE_RADIUS = 20;
// Apart enough that no MESH_DISTANCE disc reaches two groves.
const GROVE_GAP = 2 * (MESH_DISTANCE + GROVE_RADIUS) + 10;
const SCATTERED_ROCKS = 22;
// So the town sees some: the first few groves and rocks are drawn from a
// ring around the pad, the rest from the whole island.
const TOWN_GROVES = 3;
const TOWN_ROCKS = 12;
// Rings around the pad's middle, from its corners' clearing outward.
// Rocks show only within ROCK_DISTANCE, so the town's come closer.
const TOWN_GROVE_RING = { from: 140, to: 300 };
const TOWN_ROCK_RING = { from: 140, to: 200 };
const TREE_GRADE = 0.2;
const ROCK_GRADE = 0.35;
// No tree stands higher than this; above it the mountains are bare rock.
const TREELINE = 110;

// [0, 1) from a candidate index and a stream, a pure function of the seed.
const rand = (k, stream) => hash2(k, 7919 * (stream + 1));
const grade = (x, z) => Math.hypot(ground(x + 1, z) - ground(x - 1, z), ground(x, z + 1) - ground(x, z - 1)) / 2;
const apart = (list, x, z, d) => list.every((p) => Math.hypot(p.x - x, p.z - z) >= d);
// The island's land (`inland` metres from the coast) past the clearing.
const open = (x, z, minInland) => inland(x, z) >= minInland && fromPad(x, z) >= CLEARING;
// Gentle enough and low enough for a tree.
const wooded = (x, z) => grade(x, z) <= TREE_GRADE && ground(x, z) <= TREELINE;

// Candidate `id`, uniform over `ring` around the pad's middle, or over the
// whole map without one.
const candidate = (id, ring) => {
  const a = 2 * Math.PI * rand(id, 0);
  const [c, from, to] = ring ? [PAD_MIDDLE, ring.from, ring.to] : [{ x: 0, z: 0 }, 0, SIZE / 2];
  const r = Math.sqrt(lerp(from * from, to * to, rand(id, 1)));
  return [c.x + r * Math.cos(a), c.z + r * Math.sin(a)];
};

// In the terrain's space.
const placeProps = () => {
  const groves = [];
  for (let k = 0; groves.length < GROVES && k < 4000; k += 1) {
    const [x, z] = candidate(k, groves.length < TOWN_GROVES ? TOWN_GROVE_RING : null);
    if (open(x, z, 60) && fromPad(x, z) >= CLEARING + GROVE_RADIUS && apart(groves, x, z, GROVE_GAP) && wooded(x, z)) groves.push({ x, z });
  }
  const trees = [];
  const rocks = [];
  groves.forEach((g, n) => {
    const mine = [];
    for (let k = 0; mine.length < GROVE_TREES && k < 40; k += 1) {
      const id = n * 1000 + k;
      const a = 2 * Math.PI * rand(id, 2);
      const r = GROVE_RADIUS * Math.sqrt(rand(id, 3));
      const x = g.x + r * Math.cos(a);
      const z = g.z + r * Math.sin(a);
      if (!open(x, z, 40) || !wooded(x, z) || !apart(mine, x, z, 6)) continue;
      mine.push({ x, z, yaw: 2 * Math.PI * rand(id, 4), size: lerp(1.1, 1.7, rand(id, 5)), variant: rand(id, 6) < 0.5 ? 0 : 1 });
    }
    trees.push(...mine);
  });
  // A rock is two or three boxes turned every way and a third buried
  // (island.fun), the second and third smaller and leaning on the first. One
  // beside each grove, the rest scattered (the beach included).
  // Box `n` of rock `id` draws from its own streams.
  const box = (id, n, x, z, scale) => {
    const r = (stream) => rand(id, 100 * n + stream);
    return {
      x, z, yaw: 2 * Math.PI * r(7), tilt: lerp(-0.7, 0.7, r(8)), roll: lerp(-0.7, 0.7, r(12)),
      sx: scale * lerp(0.9, 2.6, r(9)), sy: scale * lerp(0.9, 2.0, r(10)), sz: scale * lerp(0.8, 2.2, r(11)),
    };
  };
  const rock = (id, x, z) => {
    const first = box(id, 0, x, z, 1);
    const more = rand(id, 13) < 0.4 ? 2 : 1;
    return [first, ...Array.from({ length: more }, (_, k) => {
      const a = 2 * Math.PI * rand(id, 14 + k);
      const d = lerp(0.5, 1.0, rand(id, 16 + k)) * Math.max(first.sx, first.sz) / 2;
      return box(id, k + 1, x + d * Math.cos(a), z + d * Math.sin(a), lerp(0.4, 0.7, rand(id, 18 + k)));
    })];
  };
  // A box is a third buried (island.fun): the ground under its farthest
  // corner may fall at most 30% of its height.
  const sites = [];
  const rockFits = (boxes) => boxes.every((r) => {
    const g = grade(r.x, r.z);
    return open(r.x, r.z, 2) && ground(r.x, r.z) > SEA_LEVEL + 0.5 && g <= ROCK_GRADE && (g * Math.hypot(r.sx, r.sz)) / 2 <= 0.3 * r.sy &&
      apart(trees, r.x, r.z, 4);
  }) && apart(sites, boxes[0].x, boxes[0].z, 6);
  const place = (boxes) => {
    sites.push(boxes[0]);
    rocks.push(...boxes);
  };
  groves.forEach((g, n) => {
    for (let k = 0; k < 20; k += 1) {
      const id = 100000 + n * 1000 + k;
      const a = 2 * Math.PI * rand(id, 2);
      const r = rock(id, g.x + (GROVE_RADIUS + 6) * Math.cos(a), g.z + (GROVE_RADIUS + 6) * Math.sin(a));
      if (rockFits(r)) {
        place(r);
        break;
      }
    }
  });
  for (let k = 0, placed = 0; placed < SCATTERED_ROCKS && k < 4000; k += 1) {
    const id = 200000 + k;
    const r = rock(id, ...candidate(id, placed < TOWN_ROCKS ? TOWN_ROCK_RING : null));
    if (!rockFits(r)) continue;
    place(r);
    placed += 1;
  }
  const sited = (p) => ({ ...p, y: ground(p.x, p.z) });
  return { trees: trees.map(sited), rocks: rocks.map(sited), sites: sites.length, groves };
};

// Fails unless the caps hold, and every prop is on the land, off the
// clearing, and within 5 cm of the heightmap's samples under it.
const checkProps = ({ trees, rocks }) => {
  if (trees.length > TREE_CAP || rocks.length > ROCK_CAP) throw new Error(`${trees.length} trees, ${rocks.length} rocks: over the caps`);
  // Any MESH_DISTANCE disc holding a tree lies within 2 MESH_DISTANCE of it.
  for (const t of trees) {
    const near = trees.filter((u) => Math.hypot(u.x - t.x, u.z - t.z) <= 2 * MESH_DISTANCE).length;
    if (near > NEAR_CAP) throw new Error(`${near} trees within ${2 * MESH_DISTANCE} m of (${t.x}, ${t.z})`);
  }
  for (const p of [...trees, ...rocks]) {
    const i = (p.x + SIZE / 2) / CELL;
    const j = (p.z + SIZE / 2) / CELL;
    const [i0, j0] = [Math.floor(i), Math.floor(j)];
    const [fi, fj] = [i - i0, j - j0];
    const h = lerp(lerp(sample(i0, j0), sample(i0 + 1, j0), fi), lerp(sample(i0, j0 + 1), sample(i0 + 1, j0 + 1), fi), fj);
    if (Math.abs(h - p.y) > 0.05 || fromPad(p.x, p.z) < CLEARING || p.y <= SEA_LEVEL) throw new Error(`prop at (${p.x}, ${p.z}) is off the ground or in the clearing`);
  }
};

// ---- Parks ----------------------------------------------------------------------
//
// A plaza beside the town and two parks farther out: parks.fun builds them;
// this places them, paints their lawns (and the plaza's ground) and their
// lamps' pools. Each stands on ground flat enough that its pieces, sunk to
// its lowest point, never float: at most PARK_RELIEF (PLAZA_RELIEF) across it
// and MARGIN round it. Clear of the tracks, the props and each other, past
// PARK_PAD of the pad's edge, and turned a quarter at a time so its gate
// (its local south) faces the town. After the props, so they don't move.
const PARK_RELIEF = 0.25;
const PLAZA_RELIEF = 0.15;
const MARGIN = 2;
const PLAZA_RING = { from: 70, to: 120 };
const PARK_RING = { from: 110, to: 320 };
// From the tracks (and so the e2e's walk down the south one), trees and rocks.
const PARK_CLEAR = 8;
const PARK_GAP = 20;
const PARK_PAD = 36;
const PARK_KINDS = [
  { name: "island-plaza", kind: "plaza", ring: PLAZA_RING, relief: PLAZA_RELIEF, wet: true, monolith: false, seed: 3 },
  { name: "island-park-0", kind: "park", ring: PARK_RING, relief: PARK_RELIEF, wet: true, monolith: true, seed: 4 },
  { name: "island-park-1", kind: "park", ring: PARK_RING, relief: PARK_RELIEF, wet: false, monolith: false, seed: 5 },
];
// parks.fun's sizes, lamp spots (in a park's space) and lamp light.
const parkSpec = () => {
  const parks = source("parks.fun");
  const record = (name) => fields(line(parks, new RegExp(`^let ${name} = \\{(.*)\\}$`, "m")));
  const spots = (name) => [...line(parks, new RegExp(`^let ${name} = \\[(.*)\\]$`, "m")).matchAll(/\((-?[\d.]+), (-?[\d.]+)\)/g)]
    .map((m) => [Number(m[1]), Number(m[2])]);
  return {
    park: { ...record("parkSize"), lamps: spots("parkLampSpots") },
    plaza: { ...record("plazaSize"), lamps: spots("plazaLampSpots") },
    light: record("parkLight"),
  };
};
// (x, z) turned `deg` about y, as block.fun's `turn` (and `Scene.rotateY`).
const turn = (deg, x, z) => {
  const a = (deg * Math.PI) / 180;
  return [x * Math.cos(a) + z * Math.sin(a), z * Math.cos(a) - x * Math.sin(a)];
};
// How far outside site `p`'s rect (x, z) is, in the terrain's space: the
// larger of the two axis distances, negative inside.
const outside = (p, x, z) => {
  const [lx, lz] = turn(-p.yaw, x - p.x, z - p.z);
  return Math.max(Math.abs(lx) - p.w / 2, Math.abs(lz) - p.d / 2);
};
// Points 2 m apart over a site's rect grown by `m`, in the terrain's space.
const siteSamples = (p, m) => {
  const out = [];
  for (let lz = -p.d / 2 - m; lz <= p.d / 2 + m + 0.001; lz += 2) {
    for (let lx = -p.w / 2 - m; lx <= p.w / 2 + m + 0.001; lx += 2) {
      const [x, z] = turn(p.yaw, lx, lz);
      out.push([p.x + x, p.z + z]);
    }
  }
  return out;
};
const placeParks = (props, spec) => {
  const paths = tracks(props.groves);
  const toTrack = (x, z) => Math.min(...paths.flatMap((t) => t.slice(1).map((b, k) => toSegment(x, z, t[k], b))));
  const placed = [];
  for (const kind of PARK_KINDS) {
    const size = kind.kind === "plaza" ? spec.plaza : spec.park;
    const radius = Math.hypot(size.w, size.d) / 2;
    for (let k = 0; k < 4000; k += 1) {
      const id = 300000 + 10000 * placed.length + k;
      const [x, z] = candidate(id, kind.ring);
      // The quarter turn whose local south faces the pad's middle.
      const facing = (Math.atan2(x - PAD_MIDDLE.x, z - PAD_MIDDLE.z) * 180) / Math.PI;
      const yaw = (((Math.round(facing / 90) * 90) % 360) + 360) % 360;
      const p = { ...kind, x, z, yaw, w: size.w, d: size.d, size };
      if (inland(x, z) < 60 + radius || !placed.every((q) => Math.hypot(q.x - x, q.z - z) >= radius + q.radius + PARK_GAP)) continue;
      if (![...props.trees, ...props.rocks].every((t) => outside(p, t.x, t.z) >= PARK_CLEAR)) continue;
      const inner = siteSamples(p, 0);
      if (!inner.every(([sx, sz]) => fromPad(sx, sz) >= PARK_PAD && toTrack(sx, sz) >= PARK_CLEAR)) continue;
      const hs = siteSamples(p, MARGIN).map(([sx, sz]) => ground(sx, sz));
      if (Math.max(...hs) - Math.min(...hs) > kind.relief) continue;
      const under = inner.map(([sx, sz]) => ground(sx, sz));
      const [lo, hi] = [Math.min(...under), Math.max(...under)];
      placed.push({ ...p, radius, y: hi, sink: hi - lo + 0.05, relief: Math.max(...hs) - Math.min(...hs) });
      break;
    }
  }
  if (placed.length !== PARK_KINDS.length) throw new Error(`placed ${placed.length} of ${PARK_KINDS.length} parks: loosen the rings or the relief`);
  // Every lamp in the world (the block's space), as game.fun lights them.
  const lamps = placed.flatMap((p) => p.size.lamps.map(([lx, lz]) => {
    const [x, z] = turn(p.yaw, lx, lz);
    const { y, r, g, b, intensity, range } = spec.light;
    return { x: p.x + x + CENTRE.x, y: p.y - PAD_Y + GROUND_Y + y, z: p.z + z + CENTRE.z, r, g, b, intensity, range };
  }));
  return { parks: placed, lamps };
};
// What a texel's ground is under the parks: (lawn, paved) weights, over a
// texel's feather at the edge.
const parkWeights = (parks, x, z) => {
  let lawn = 0;
  let paved = 0;
  for (const p of parks) {
    const w = 1 - smoothstep(-STEP / 2, STEP / 2, outside(p, x, z));
    if (p.kind === "plaza") paved = Math.max(paved, w);
    else lawn = Math.max(lawn, w);
  }
  return [lawn, paved];
};

// ---- Colormaps -------------------------------------------------------------------
//
// `Terrain.colormap`'s day and night maps and `Terrain.normalMap`'s normals,
// COLORMAP² over the island: one texel per 2 m, its centre on an odd sample
// ((i + 0.5) / COLORMAP of the way across, as the shader's terrainUv maps it).
// Albedo from the biome's palette by region, coast, grade, height and the
// town; then lit: day = albedo × (ambient + sun·N·L) × occlusion, night the
// same under the moon plus the street lights. No cast shadows.
const COLORMAP = 1024;
const STEP = SIZE / COLORMAP; // 2 m, two samples

// The bake's light, authored as the frame's lights are (sRGB colour ×
// intensity; `direction` is the way the light travels). islandLayout.fun
// carries it to hours.fun, so props and buildings match the ground. By day
// a high sun (60°), so slopes facing away still read under the flashlight.
const LIGHT = {
  day: { ambient: [0.7, 0.72, 0.78], sun: [1.0, 0.94, 0.82], intensity: 1.5, direction: [0.42, -0.87, 0.27] },
  night: { ambient: [0.09, 0.08, 0.14], sun: [0.35, 0.38, 0.55], intensity: 0.6, direction: [0.3, -0.8, 0.5] },
};
// island.fun's `groundY` (tests.fun checks it): the pad's height in the world.
export const GROUND_Y = -0.05;

const toLinear = (c) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
const toSrgb = (c) => (c <= 0.0031308 ? c * 12.92 : 1.055 * c ** (1 / 2.4) - 0.055);
const lin = (rgb) => rgb.map(toLinear);
const mix3 = (a, b, t) => [lerp(a[0], b[0], t), lerp(a[1], b[1], t), lerp(a[2], b[2], t)];
const unit = ([x, y, z]) => {
  const l = Math.hypot(x, y, z);
  return [x / l, y / l, z / l];
};
// Linear light from an authored colour × intensity, as the engine packs it.
const radiance = (rgb, k) => lin(rgb).map((v) => v * k);

// The street lights, read from exterior.fun (lobbyLight, sodiumLight),
// officeLayout.fun (lampHeads) and block.fun (streetWidth), and placed on
// every copy as block.fun places them; tests.fun checks the result against
// game.fun's.
const source = (name) => readFileSync(fileURLToPath(new URL(name, import.meta.url)), "utf8");
const line = (text, re) => {
  const m = text.match(re);
  if (!m) throw new Error(`island.mjs can't find ${re}: update its parser`);
  return m[1];
};
const fields = (text) => Object.fromEntries([...text.matchAll(/(\w+): (-?[\d.]+)/g)].map(([, k, v]) => [k, Number(v)]));
const streetLights = () => {
  const exterior = source("exterior.fun");
  const lobby = fields(line(exterior, /^let lobbyLight = \{(.*)\}$/m));
  const lamp = fields(line(exterior, /^let sodiumLight = \{(.*)\}$/m));
  const heads = [...line(source("officeLayout.fun"), /^let lampHeads = \[(.*)\]$/m).matchAll(/v\((-?[\d.]+), (-?[\d.]+), (-?[\d.]+)\)/g)]
    .map((m) => m.slice(1).map(Number));
  const centre = -Number(line(source("block.fun"), /^let streetWidth = (-?[\d.]+)$/m)) / 2;
  const lights = [];
  for (let k = 0; k < 4; k += 1) {
    const [cx, cz] = turn(90 * k, centre, centre);
    const world = (x, y, z) => {
      const [tx, tz] = turn(90 * k, x, z);
      return { x: tx + centre - cx, y, z: tz + centre - cz };
    };
    const { r, g, b, intensity, range } = lobby;
    lights.push({ ...world(lobby.x, lobby.y, lobby.z), r, g, b, intensity, range });
    for (const [x, y, z] of heads) {
      lights.push({ ...world(x, y - lamp.drop, z), r: lamp.r, g: lamp.g, b: lamp.b, intensity: lamp.intensity, range: lamp.range });
    }
  }
  return lights;
};

// Dirt tracks from the pad's edge out to the town groves and down to the
// south beach: polylines (8 m steps) that wander up to 6 m either side.
const tracks = (groves) => {
  const targets = groves.slice(0, TOWN_GROVES).map((g) => [g.x, g.z, GROVE_RADIUS]);
  let south = PAD_MIDDLE.z;
  while (inland(PAD_MIDDLE.x, south) > 15) south -= 4;
  targets.push([PAD_MIDDLE.x, south, 0]);
  // No two leave within 30° of each other.
  const heading = ([x, z]) => Math.atan2(z - PAD_MIDDLE.z, x - PAD_MIDDLE.x);
  const apartFrom = (t, k) => targets.slice(0, k).every((u) => Math.abs(Math.atan2(Math.sin(heading(t) - heading(u)), Math.cos(heading(t) - heading(u)))) > Math.PI / 6);
  return targets.filter(apartFrom).map(([tx, tz, stop], n) => {
    const length = Math.hypot(tx - PAD_MIDDLE.x, tz - PAD_MIDDLE.z);
    const [ux, uz] = [(tx - PAD_MIDDLE.x) / length, (tz - PAD_MIDDLE.z) / length];
    const points = [];
    for (let d = 40; d <= length - stop; d += 8) {
      const side = 6 * perlin(d / 45 + 7.3 * n, 3.1 * n) * smoothstep(40, 80, d);
      points.push([PAD_MIDDLE.x + ux * d - uz * side, PAD_MIDDLE.z + uz * d + ux * side]);
    }
    return points;
  });
};
const toSegment = (x, z, [ax, az], [bx, bz]) => {
  const [dx, dz] = [bx - ax, bz - az];
  const t = clamp(((x - ax) * dx + (z - az) * dz) / (dx * dx + dz * dz), 0, 1);
  return Math.hypot(x - ax - t * dx, z - az - t * dz);
};

// Soft occlusion under every tree and rock: a darkening that fades out at
// `reach` metres from the prop, multiplied into both maps.
const propShade = ({ trees, rocks }) => {
  const shade = new Float32Array(COLORMAP * COLORMAP).fill(1);
  const texel = (v) => (v + SIZE / 2) / STEP - 0.5;
  const blob = (x, z, reach, depth) => {
    for (let j = Math.max(0, Math.floor(texel(z - reach))); j <= Math.min(COLORMAP - 1, Math.ceil(texel(z + reach))); j += 1) {
      for (let i = Math.max(0, Math.floor(texel(x - reach))); i <= Math.min(COLORMAP - 1, Math.ceil(texel(x + reach))); i += 1) {
        const d = Math.hypot(-SIZE / 2 + (i + 0.5) * STEP - x, -SIZE / 2 + (j + 0.5) * STEP - z) / reach;
        if (d < 1) shade[j * COLORMAP + i] *= 1 - depth * (1 - d * d) ** 2;
      }
    }
  };
  for (const t of trees) blob(t.x, t.z, 3.2 * t.size, 0.45);
  for (const r of rocks) blob(r.x, r.z, 0.7 * Math.max(r.sx, r.sz) + 1, 0.35);
  return shade;
};

// Horizon occlusion: the mean openness over 8 directions, marched out to
// ~350 m in growing steps; valleys and the feet of slopes darken.
const horizon = (si, sj, h0) => {
  let open = 0;
  for (let d = 0; d < 8; d += 1) {
    const [dx, dz] = [Math.cos((d * Math.PI) / 4), Math.sin((d * Math.PI) / 4)];
    let tangent = 0;
    for (let k = 0, dist = 3; k < 10; k += 1, dist *= 1.7) {
      const i = clamp(Math.round(si + (dx * dist) / CELL), 0, SAMPLES - 1);
      const j = clamp(Math.round(sj + (dz * dist) / CELL), 0, SAMPLES - 1);
      tangent = Math.max(tangent, (sample(i, j) - h0) / dist);
    }
    open += 1 - tangent / Math.hypot(1, tangent);
  }
  const ao = open / 8;
  return 1 + (ao * ao - 1) * 0.6;
};

// The biome's albedo (linear) at a texel, from the ground there: `part` is
// `islandAt`'s, `track` the distance to the nearest dirt track, the last
// the parks' (lawn, paved) weights.
const albedoAt = (b, x, z, h, grade, part, track, [lawn, paved]) => {
  const pad = fromPad(x, z);
  // Macro variation: broad patches of lusher and drier ground, finer mottling.
  const macro = 0.5 + fbm(x / 160 + 41.3, z / 160 - 12.9, 3);
  const mottle = fbm(x / 30 - 8.1, z / 30 + 27.4, 2);
  let c = mix3(b.lush, b.dry, smoothstep(0.2, 0.8, macro + 0.25 * mottle));
  c = mix3(c, b.hill, 0.6 * smoothstep(0.05, 0.5, part.hills) * smoothstep(0.35, 0.65, 0.5 + mottle));
  c = mix3(c, b.gravel, smoothstep(0.3, 0.55, grade + 0.2 * mottle));
  // Above the treeline scree; steep faces bare rock; snow on gentler ground
  // from the snow line.
  c = mix3(c, b.scree, smoothstep(TREELINE - 10, TREELINE + 30, h + 25 * mottle));
  c = mix3(c, b.rock, smoothstep(b.rockGrade - 0.2, b.rockGrade + 0.15, grade + 0.15 * part.mount * mottle));
  c = mix3(c, b.snow, smoothstep(b.snowLine, b.snowLine + 15, h + 12 * mottle) * (1 - smoothstep(0.6, 1.0, grade)));
  // The beach: wet sand at the waterline, dry above (and the seabed).
  const beach = (1 - smoothstep(18, 45, part.s + 30 * mottle)) * (1 - smoothstep(2, 5, h));
  c = mix3(c, mix3(b.wet, b.sand, smoothstep(0.1, 1.2, h)), beach);
  // The town: dirt around the pad and along the tracks, the road on it.
  const dirt = Math.max(1 - smoothstep(4, 18 + 10 * (mottle + 0.5), pad), 1 - smoothstep(1.4, 3 + 2 * mottle, track));
  c = mix3(c, b.dirt, dirt);
  c = mix3(c, b.road, 1 - smoothstep(0, 2, pad));
  // The parks: mown lawn, the plaza's paving.
  c = mix3(c, b.lawn, lawn);
  c = mix3(c, b.road, paved);
  const vary = 0.9 + 0.2 * (0.5 + mottle);
  return lin(c).map((v) => v * vary);
};

// Each biome's day and night maps (sRGB RGB8) and the normal map
// (terrain-local, xyz × 0.5 + 0.5, as `Terrain.normalMap` reads it). The
// light at a texel is the same for every biome; only the albedo differs.
const paint = (biomes, props, lights, parks) => {
  const maps = biomes.map((name) => ({ name, b: BIOMES[name], day: Buffer.alloc(COLORMAP * COLORMAP * 3), night: Buffer.alloc(COLORMAP * COLORMAP * 3) }));
  const normals = Buffer.alloc(COLORMAP * COLORMAP * 3);
  const sky = ["day", "night"].map((k) => ({
    ambient: lin(LIGHT[k].ambient), sun: radiance(LIGHT[k].sun, LIGHT[k].intensity), toward: unit(LIGHT[k].direction.map((v) => -v)),
  }));
  const lamps = lights.map((l) => {
    const [x, z] = toTerrain(l.x, l.z);
    return { x, z, y: l.y - GROUND_Y + PAD_Y, range: l.range, light: radiance([l.r, l.g, l.b], l.intensity) };
  });
  const paths = tracks(props.groves);
  const shade = propShade(props);
  for (let j = 0; j < COLORMAP; j += 1) {
    for (let i = 0; i < COLORMAP; i += 1) {
      const [si, sj] = [2 * i + 1, 2 * j + 1];
      const [x, z] = [at(si), at(sj)];
      const h = sample(si, sj);
      const dx = (sample(si + 1, sj) - sample(si - 1, sj)) / (2 * CELL);
      const dz = (sample(si, sj + 1) - sample(si, sj - 1)) / (2 * CELL);
      const n = unit([-dx, 1, -dz]);
      let track = Infinity;
      for (const p of paths) for (let k = 0; k + 1 < p.length; k += 1) track = Math.min(track, toSegment(x, z, p[k], p[k + 1]));
      const part = islandAt(x, z);
      const park = parkWeights(parks, x, z);
      const occlusion = horizon(si, sj, h) * shade[j * COLORMAP + i];
      const [dayLight, nightLight] = sky.map((l) => {
        const ndl = Math.max(0, n[0] * l.toward[0] + n[1] * l.toward[1] + n[2] * l.toward[2]);
        return [0, 1, 2].map((c) => (l.ambient[c] + l.sun[c] * ndl) * occlusion);
      });
      // The street lights, at night only: the engine's falloff, (1 − d²/r²)².
      for (const l of lamps) {
        const v = [l.x - x, l.y - h, l.z - z];
        const d = Math.hypot(...v);
        if (d >= l.range) continue;
        const f = (1 - (d * d) / (l.range * l.range)) ** 2 * Math.max(0, (n[0] * v[0] + n[1] * v[1] + n[2] * v[2]) / d);
        for (let c = 0; c < 3; c += 1) nightLight[c] += l.light[c] * f * shade[j * COLORMAP + i];
      }
      const o = (j * COLORMAP + i) * 3;
      for (const m of maps) {
        const albedo = albedoAt(m.b, x, z, h, Math.hypot(dx, dz), part, track, park);
        for (let c = 0; c < 3; c += 1) {
          m.day[o + c] = Math.round(clamp(toSrgb(albedo[c] * dayLight[c]), 0, 1) * 255);
          m.night[o + c] = Math.round(clamp(toSrgb(albedo[c] * nightLight[c]), 0, 1) * 255);
        }
      }
      for (let c = 0; c < 3; c += 1) normals[o + c] = Math.round((n[c] * 0.5 + 0.5) * 255);
    }
  }
  return { maps, normals };
};

const num = (v, digits = 3) => {
  const s = (Math.round(v * 10 ** digits) / 10 ** digits).toString();
  return s.includes(".") ? s : `${s}.0`;
};

const main = () => {
  generate();
  const s = survey();
  const props = placeProps();
  checkProps(props);
  const lights = streetLights();
  const parks = placeParks(props, parkSpec());
  const pixels = Buffer.alloc(SAMPLES * SAMPLES * 2);
  levels.forEach((l, k) => pixels.writeUInt16BE(l, k * 2));
  const png = encodePng(SAMPLES, SAMPLES, 16, 0, pixels);
  writeFileSync(fileURLToPath(new URL("island.png", import.meta.url)), png);

  const fun = `// islandLayout.fun — GENERATED by island.mjs (preset ${PRESET_NAME}, seed ${SEED}); do not edit.
// Rerun the script after changing the island; it rewrites this, island.png and the maps.
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
// The props: none within \`clearing\` of the pad's edge. At most \`nearCap\`
// trees within \`meshDistance\` of any point, \`treeCap\` trees and \`rockCap\`
// rocks in all; rocks show within \`rockDistance\`. No tree above \`treeline\`.
let props = { clearing: ${num(CLEARING)}, treeline: ${num(TREELINE)}, meshDistance: ${num(MESH_DISTANCE)}, nearCap: ${num(NEAR_CAP)}, treeCap: ${num(TREE_CAP)}, rockCap: ${num(ROCK_CAP)}, rockDistance: ${num(ROCK_DISTANCE)} }
// Each stands on the ground at (x, z): y is its height there. Yaw and tilt
// are radians; a tree's variant picks its model.
let trees = [
${props.trees.map((t, i) => `  { id: ${num(i)}, x: ${num(t.x + CENTRE.x)}, y: ${num(t.y)}, z: ${num(t.z + CENTRE.z)}, yaw: ${num(t.yaw)}, size: ${num(t.size)}, variant: ${num(t.variant)} },`).join("\n")}
]
// A rock is a box sx × sy × sz, tilted (about x), rolled (about z), then
// turned (yaw); a cluster of boxes makes one boulder.
let rocks = [
${props.rocks.map((r, i) => `  { id: ${num(i)}, x: ${num(r.x + CENTRE.x)}, y: ${num(r.y)}, z: ${num(r.z + CENTRE.z)}, yaw: ${num(r.yaw)}, tilt: ${num(r.tilt)}, roll: ${num(r.roll)}, sx: ${num(r.sx)}, sy: ${num(r.sy)}, sz: ${num(r.sz)} },`).join("\n")}
]
// The colormaps' bake (island-<biome>-day/-night.png, island-normal.png):
// the pad's height in the world, the street lights painted into the night
// map (exterior.fun's, on every copy, in the world), and the sun or moon
// and ambient each map was lit with, as \`Light.directional\` and
// \`Light.ambient\` take them.
let groundY = ${num(GROUND_Y)}
let lamps = [
${lights.map((l) => `  { x: ${num(l.x)}, y: ${num(l.y)}, z: ${num(l.z)}, r: ${num(l.r)}, g: ${num(l.g)}, b: ${num(l.b)}, intensity: ${num(l.intensity)}, range: ${num(l.range)} },`).join("\n")}
]
// The parks (parks.fun builds them): a plaza by the town and two parks,
// each on flat ground, its middle at (x, z), \`y\` the
// highest ground under it and every upright box \`sink\` into it; turned
// \`yaw\` degrees, its gate to the town. Their lamps, in the world, are
// painted into the night map.
let parks = [
${parks.parks.map((p) => `  { name: "${p.name}", kind: "${p.kind}", x: ${num(p.x + CENTRE.x)}, y: ${num(p.y)}, z: ${num(p.z + CENTRE.z)}, yaw: ${num(p.yaw)}, sink: ${num(p.sink)}, wet: ${p.wet}, monolith: ${p.monolith}, seed: ${num(p.seed)} },`).join("\n")}
]
let parkLamps = [
${parks.lamps.map((l) => `  { x: ${num(l.x)}, y: ${num(l.y)}, z: ${num(l.z)}, r: ${num(l.r)}, g: ${num(l.g)}, b: ${num(l.b)}, intensity: ${num(l.intensity)}, range: ${num(l.range)} },`).join("\n")}
]
let light = {
${["day", "night"].map((k) => {
    const { ambient: a, sun: c, intensity, direction: d } = LIGHT[k];
    const rgb = ([r, g, b]) => `{ r: ${num(r)}, g: ${num(g)}, b: ${num(b)} }`;
    return `  ${k}: { ambient: ${rgb(a)}, sun: ${rgb(c)}, intensity: ${num(intensity)}, direction: { x: ${num(d[0])}, y: ${num(d[1])}, z: ${num(d[2])} } },`;
  }).join("\n")}
}
`;
  writeFileSync(fileURLToPath(new URL("islandLayout.fun", import.meta.url)), fun);

  const { maps, normals } = paint(PAINTED, props, [...lights, ...parks.lamps], parks.parks);
  const sizes = [...maps.flatMap((m) => [[`island-${m.name}-day.png`, m.day], [`island-${m.name}-night.png`, m.night]]), ["island-normal.png", normals]]
    .map(([name, pixels]) => {
      const bytes = encodePng(COLORMAP, COLORMAP, 8, 2, pixels);
      writeFileSync(fileURLToPath(new URL(name, import.meta.url)), bytes);
      return `${name} ${bytes.length}`;
    });

  console.log(
    `wrote ${SAMPLES}² heightmap (${png.length} bytes); steepest grade ${(s.steepest * 100).toFixed(1)}%; ` +
      `under the pad ${s.relief.toFixed(2)} m relief, ${(s.slope * 100).toFixed(1)}% grade before flattening; ` +
      `highest ${s.highest.toFixed(1)} m; pad ${PAD_Y.toFixed(3)} m, band ${s.yMin.toFixed(3)}–${s.yMax.toFixed(3)} m; ` +
      `${props.trees.length} trees, ${props.sites} rocks of ${props.rocks.length} boxes; ` +
      `parks ${parks.parks.map((p) => `${p.name} (${num(p.x + CENTRE.x, 1)}, ${num(p.z + CENTRE.z, 1)}) ${num(p.relief, 2)} m`).join(", ")}; ${COLORMAP}² maps: ${sizes.join(", ")} bytes`,
  );
};

// Run as a script; imported (by the e2e), only the ground function.
if (process.argv[1] === fileURLToPath(import.meta.url)) main();
