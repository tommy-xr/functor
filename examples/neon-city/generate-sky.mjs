// Deterministically generate neon-city's six skybox faces using only Node
// built-ins. Re-run with:
//
//   node examples/neon-city/generate-sky.mjs
//
// The sky is a function of view direction, so the faces meet seamlessly: a
// near-black zenith, a violet-to-magenta horizon glow, and a striped synthwave
// sun low in +Z. Below the horizon the sky is mirrored and dimmed — that half
// shows through the game's translucent wet street as the sky's reflection.
// The skybox is never fogged, so the sun lives here rather than in the scene.
import { deflateSync } from "node:zlib";
import { writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const size = 512;

// Keep in sync with `fogColor` in game.fun: the horizon is exactly the fog
// color, so fully fogged buildings dissolve into the sky.
const horizon = [0.42, 0.12, 0.46];
const glow = [0.95, 0.22, 0.55];
const mid = [0.16, 0.04, 0.26];
const zenith = [0.02, 0.01, 0.05];
const sunTop = [1.0, 0.86, 0.32];
const sunBottom = [1.0, 0.16, 0.55];

// Sun: angular radius and centre elevation, in radians, centred on +Z.
const sunRadius = 0.26;
const sunElevation = 0.12;
// How much dimmer the mirrored (below-horizon) half is.
const reflectGain = 0.5;

const clamp = (v, lo, hi) => Math.max(lo, Math.min(hi, v));
const smoothstep = (a, b, v) => {
  const t = clamp((v - a) / (b - a), 0, 1);
  return t * t * (3 - 2 * t);
};
const mix = (a, b, t) => a.map((x, i) => x + (b[i] - x) * t);

// Sky color for a unit direction above the horizon (elevation >= 0).
const skyAbove = (elevation, toSun) => {
  const band = mix(horizon, mid, smoothstep(0.0, 0.35, elevation));
  const sky = mix(band, zenith, smoothstep(0.3, 1.2, elevation));
  // A warm glow just above the horizon toward the sun. It fades out AT the
  // horizon, so elevation 0 is exactly the fog color in every direction.
  const glowAmount =
    Math.exp(-elevation * 9) * Math.pow(Math.max(toSun, 0), 6) * smoothstep(0.0, 0.06, elevation);
  return mix(sky, glow, clamp(glowAmount, 0, 1) * 0.8);
};

// The sun disc with synthwave stripes cut through its lower half. `dx`/`dy`
// are angular offsets from its centre.
const sun = (dx, dy) => {
  const r = Math.hypot(dx, dy);
  if (r > sunRadius) return null;
  const v = dy / sunRadius; // -1 bottom .. 1 top
  if (v < 0.1) {
    // Stripe gaps widen toward the bottom.
    const phase = ((0.1 - v) * 5.5) % 1;
    const gap = 0.12 + (0.1 - v) * 0.5;
    if (phase < gap) return null;
  }
  return mix(sunBottom, sunTop, (v + 1) / 2);
};

const shade = (x, y, z) => {
  const len = Math.hypot(x, y, z);
  const [dx, dy, dz] = [x / len, y / len, z / len];
  const elevation = Math.asin(Math.abs(dy));
  const azimuth = Math.atan2(dx, dz);
  let color = skyAbove(elevation, dz);
  const disc = sun(azimuth, elevation - sunElevation);
  if (disc && dz > 0) color = disc;
  if (dy < 0) {
    // Mirrored half: full strength at grazing angles (where wet streets
    // reflect most), fading darker with depth below the horizon.
    const fade = 1 - (1 - reflectGain) * smoothstep(0.0, 0.5, elevation);
    color = mix(zenith, color, fade);
  } else if (dy > 0.25) {
    // Sparse faint stars, a pure function of direction.
    const cell =
      Math.floor(dx * 300) * 7919 + Math.floor(dy * 300) * 104729 + Math.floor(dz * 300) * 1299709;
    const h = Math.abs(Math.sin(cell) * 43758.5453) % 1;
    if (h > 0.9985) color = mix(color, [0.9, 0.85, 1.0], 0.6);
  }
  return color;
};

// Direction for a face texel, in the OpenGL cubemap convention (+X, -X, +Y,
// -Y, +Z, -Z), with row 0 at the top of each side face.
const faces = {
  px: (sc, tc) => [1, -tc, -sc],
  nx: (sc, tc) => [-1, -tc, sc],
  py: (sc, tc) => [sc, 1, tc],
  ny: (sc, tc) => [sc, -1, -tc],
  pz: (sc, tc) => [sc, -tc, 1],
  nz: (sc, tc) => [-sc, -tc, -1],
};

const crcTable = new Uint32Array(256);
for (let n = 0; n < 256; n++) {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  crcTable[n] = c >>> 0;
}
const crc32 = (bytes) => {
  let crc = 0xffffffff;
  for (const byte of bytes) crc = crcTable[(crc ^ byte) & 0xff] ^ (crc >>> 8);
  return (crc ^ 0xffffffff) >>> 0;
};
const chunk = (type, data) => {
  const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const result = Buffer.alloc(12 + data.length);
  result.writeUInt32BE(data.length, 0);
  body.copy(result, 4);
  result.writeUInt32BE(crc32(body), 8 + data.length);
  return result;
};
const png = (raw) => {
  const header = Buffer.alloc(13);
  header.writeUInt32BE(size, 0);
  header.writeUInt32BE(size, 4);
  header[8] = 8; // bit depth
  header[9] = 2; // RGB
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", header),
    chunk("IDAT", deflateSync(raw, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
};

const dir = fileURLToPath(new URL(".", import.meta.url));
for (const [name, direction] of Object.entries(faces)) {
  const raw = Buffer.alloc((size * 3 + 1) * size);
  for (let py = 0; py < size; py++) {
    const row = py * (size * 3 + 1);
    raw[row] = 0; // filter: none
    for (let px = 0; px < size; px++) {
      const sc = ((px + 0.5) / size) * 2 - 1;
      const tc = ((py + 0.5) / size) * 2 - 1;
      const [r, g, b] = shade(...direction(sc, tc));
      const o = row + 1 + px * 3;
      raw[o] = Math.round(clamp(r, 0, 1) * 255);
      raw[o + 1] = Math.round(clamp(g, 0, 1) * 255);
      raw[o + 2] = Math.round(clamp(b, 0, 1) * 255);
    }
  }
  writeFileSync(`${dir}sky_${name}.png`, png(raw));
}
