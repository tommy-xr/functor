// Bake the day and night colormaps for `Terrain.colormap` from heightmap.png,
// using only Node built-ins. Re-run after regenerating the heightmap:
//
//   node examples/terrain/generate-colormaps.mjs
//
// The colormap terrain is unlit: the lit path's height/slope bands, sun,
// ambient and occlusion are computed here once and stored (no specular). Day = albedo × (ambient + sun·N·L) × horizon AO; night is the same
// albedo under dim moonlight. Output is deterministic.
import { inflateSync } from "node:zlib";
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { encodePng } from "../../scripts/png.mjs";

// Must match game.fun's `Terrain.heightmap(…, worldSize, worldSize, -80.0, 520.0)`.
const WORLD = 4000;
const MIN_H = -80;
const MAX_H = 520;
const WATER = 28; // game.fun's water plane
const OUT = 1024;

const here = (name) => fileURLToPath(new URL(name, import.meta.url));
const clamp = (v, lo, hi) => Math.max(lo, Math.min(hi, v));
const smoothstep = (a, b, v) => {
  const t = clamp((v - a) / (b - a), 0, 1);
  return t * t * (3 - 2 * t);
};
const mix = (a, b, t) => a.map((x, i) => x + (b[i] - x) * t);
// Authored colors are sRGB, like `Color.rgb`; light in linear.
const toLinear = (c) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
const toSrgb = (c) => (c <= 0.0031308 ? c * 12.92 : 1.055 * c ** (1 / 2.4) - 0.055);
const lin = (rgb) => rgb.map(toLinear);
const normalize = ([x, y, z]) => {
  const l = Math.hypot(x, y, z);
  return [x / l, y / l, z / l];
};

// ── Decode the 16-bit grayscale heightmap (scripts/png.mjs decodes 8-bit only) ──────────────────────────────────────
const readPng16 = (path) => {
  const png = readFileSync(path);
  let pos = 8;
  let width = 0;
  let height = 0;
  const idat = [];
  while (pos < png.length) {
    const len = png.readUInt32BE(pos);
    const type = png.toString("ascii", pos + 4, pos + 8);
    const data = png.subarray(pos + 8, pos + 8 + len);
    if (type === "IHDR") {
      width = data.readUInt32BE(0);
      height = data.readUInt32BE(4);
      if (data[8] !== 16 || data[9] !== 0 || data[12] !== 0) {
        throw new Error("expected a non-interlaced 16-bit grayscale PNG");
      }
    } else if (type === "IDAT") {
      idat.push(data);
    }
    pos += 12 + len;
  }
  const raw = inflateSync(Buffer.concat(idat));
  const stride = width * 2;
  const rows = Buffer.alloc(stride * height);
  for (let y = 0; y < height; y += 1) {
    const filter = raw[y * (stride + 1)];
    const src = raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1));
    const out = rows.subarray(y * stride, (y + 1) * stride);
    const prev = y > 0 ? rows.subarray((y - 1) * stride, y * stride) : null;
    for (let i = 0; i < stride; i += 1) {
      const a = i >= 2 ? out[i - 2] : 0;
      const b = prev ? prev[i] : 0;
      const c = prev && i >= 2 ? prev[i - 2] : 0;
      let pred = 0;
      if (filter === 1) pred = a;
      else if (filter === 2) pred = b;
      else if (filter === 3) pred = (a + b) >> 1;
      else if (filter === 4) {
        const p = a + b - c;
        const pa = Math.abs(p - a);
        const pb = Math.abs(p - b);
        const pc = Math.abs(p - c);
        pred = pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
      }
      out[i] = (src[i] + pred) & 0xff;
    }
  }
  const metres = new Float32Array(width * height);
  for (let i = 0; i < width * height; i += 1) {
    metres[i] = MIN_H + (rows.readUInt16BE(i * 2) / 65535) * (MAX_H - MIN_H);
  }
  return { width, height, metres };
};

const map = readPng16(here("heightmap.png"));
const N = map.width;
const spacing = WORLD / (N - 1); // metres between samples

// Bilinear height at a fractional sample position (clamped to the map).
const heightAt = (px, py) => {
  px = clamp(px, 0, N - 1);
  py = clamp(py, 0, N - 1);
  const x0 = Math.min(Math.floor(px), N - 2);
  const y0 = Math.min(Math.floor(py), N - 2);
  const fx = px - x0;
  const fy = py - y0;
  const m = map.metres;
  const h00 = m[y0 * N + x0];
  const h10 = m[y0 * N + x0 + 1];
  const h01 = m[(y0 + 1) * N + x0];
  const h11 = m[(y0 + 1) * N + x0 + 1];
  return (h00 + (h10 - h00) * fx) * (1 - fy) + (h01 + (h11 - h01) * fx) * fy;
};

// Cheap value noise for albedo variation, so flat bands don't read as paint.
const hash2 = (ix, iy) => {
  let h = Math.imul(ix, 0x27d4eb2d) ^ Math.imul(iy, 0x165667b1) ^ 0x1c0102;
  h = Math.imul(h ^ (h >>> 15), 0x2c1b3c6d);
  h = Math.imul(h ^ (h >>> 12), 0x297a2d39);
  return ((h ^ (h >>> 15)) >>> 0) / 4294967296;
};
const valueNoise = (x, y) => {
  const ix = Math.floor(x);
  const iy = Math.floor(y);
  const ux = (x - ix) * (x - ix) * (3 - 2 * (x - ix));
  const uy = (y - iy) * (y - iy) * (3 - 2 * (y - iy));
  const a = hash2(ix, iy) + (hash2(ix + 1, iy) - hash2(ix, iy)) * ux;
  const b = hash2(ix, iy + 1) + (hash2(ix + 1, iy + 1) - hash2(ix, iy + 1)) * ux;
  return a + (b - a) * uy;
};

// Palette: the lit example's `Terrain.layered` colors, plus a beach band.
const LOW = [0.12, 0.3, 0.12];
const HIGH = [0.34, 0.49, 0.2];
const ROCK = [0.28, 0.25, 0.23];
const SNOW = [0.86, 0.9, 0.91];
const SAND = [0.62, 0.56, 0.4];
const SNOW_HEIGHT = 340;

// Lights, matching game.fun's frame lights so lit props agree with the bake.
const DAY = {
  ambient: lin([0.2, 0.23, 0.28]),
  dir: normalize([-0.42, 1.0, -0.28]),
  sun: lin([1.0, 0.84, 0.64]).map((c) => c * 1.15),
};
const NIGHT = {
  ambient: lin([0.06, 0.07, 0.12]),
  dir: normalize([0.35, 0.8, 0.45]),
  sun: lin([0.5, 0.6, 0.85]).map((c) => c * 0.5),
};

// Horizon AO: the mean sky openness over 8 directions, marched outward in
// geometric steps. Open ground → 1, a valley floor → darker. Squared to
// spread the narrow band real terrain occupies, then applied at partial
// strength so flat ground doesn't read as painted-on shadow.
const AO_DIRS = 8;
const AO_STEPS = 10;
const AO_STRENGTH = 0.6;
const horizonAo = (px, py, h0) => {
  let open = 0;
  for (let d = 0; d < AO_DIRS; d += 1) {
    const angle = (d / AO_DIRS) * Math.PI * 2;
    const dx = Math.cos(angle);
    const dy = Math.sin(angle);
    let maxSlope = 0;
    let dist = 1.5; // samples
    for (let s = 0; s < AO_STEPS; s += 1) {
      const h = heightAt(px + dx * dist, py + dy * dist);
      maxSlope = Math.max(maxSlope, (h - h0) / (dist * spacing));
      dist *= 1.7;
    }
    open += 1 - maxSlope / Math.hypot(1, maxSlope); // 1 − sin(horizon)
  }
  const ao = open / AO_DIRS;
  return 1 + (ao * ao - 1) * AO_STRENGTH;
};

const day = Buffer.alloc(OUT * OUT * 3);
const night = Buffer.alloc(OUT * OUT * 3);
for (let j = 0; j < OUT; j += 1) {
  for (let i = 0; i < OUT; i += 1) {
    // Texel centre → heightmap sample space, as the shader's terrainUv maps it.
    const px = ((i + 0.5) / OUT) * (N - 1);
    const py = ((j + 0.5) / OUT) * (N - 1);
    const h = heightAt(px, py);
    const dhdx = (heightAt(px + 1, py) - heightAt(px - 1, py)) / (2 * spacing);
    const dhdz = (heightAt(px, py + 1) - heightAt(px, py - 1)) / (2 * spacing);
    const n = normalize([-dhdx, 1, -dhdz]);

    // Height/slope bands, as `Terrain.layered` weights them.
    const t = clamp((h - MIN_H) / (MAX_H - MIN_H), 0, 1);
    const rock = smoothstep(0.3, 0.67, 1 - n[1]);
    const snow = smoothstep(SNOW_HEIGHT, SNOW_HEIGHT + 48, h) * (1 - rock * 0.72);
    const beach = 1 - smoothstep(WATER + 1, WATER + 7, h);
    let albedo = mix(LOW, HIGH, smoothstep(0.15, 0.72, t));
    albedo = mix(albedo, SAND, beach * (1 - rock));
    albedo = mix(albedo, ROCK, rock);
    albedo = mix(albedo, SNOW, snow);
    const wx = (i / OUT) * 96;
    const wy = (j / OUT) * 96;
    const vary = 0.88 + 0.24 * (0.6 * valueNoise(wx, wy) + 0.4 * valueNoise(wx * 3.7, wy * 3.7));
    albedo = lin(albedo).map((c) => c * vary);

    const ao = horizonAo(px, py, h);
    for (const [light, out] of [
      [DAY, day],
      [NIGHT, night],
    ]) {
      const ndl = Math.max(0, n[0] * light.dir[0] + n[1] * light.dir[1] + n[2] * light.dir[2]);
      const o = (j * OUT + i) * 3;
      for (let c = 0; c < 3; c += 1) {
        const lit = albedo[c] * (light.ambient[c] + light.sun[c] * ndl) * ao;
        out[o + c] = Math.round(clamp(toSrgb(lit), 0, 1) * 255);
      }
    }
  }
}

// ── Encode RGB8 PNGs ─────────────────────────────────────────────────────────
for (const [name, pixels] of [
  ["colormap-day.png", day],
  ["colormap-night.png", night],
]) {
  const png = encodePng(OUT, OUT, 8, 2, pixels);
  writeFileSync(here(name), png);
  console.log(`wrote ${name}: ${OUT}x${OUT} RGB (${png.length} bytes)`);
}
