// Generate the street atlas (streetscape.fun): `street-atlas.png`, every
// sign, window dressing, vending front and kerb-ramp pad the streetscape
// kit lays on decal quads, and `streetAtlas.fun`, each cell's UV rect.
// Node built-ins only; re-running reproduces both files byte for byte (with
// the same Node: its zlib does the deflate):
//
//   node examples/liminal-office/streetscape.mjs
//
// Text is a 5 × 7 pixel font. Window cells come in two shapes, a band's
// (2.2 × 1.3 m) and a tower bay's (1.2 × 1.4 m), by day and by night.
import { writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { encodePng } from "../../scripts/png.mjs";

// Deterministic hash noise in [0, 1).
const hash = (x, y, seed) => {
  let h = (Math.imul(x, 374761393) + Math.imul(y, 668265263) + Math.imul(seed, 1442695041)) | 0;
  h = Math.imul(h ^ (h >>> 13), 1274126177);
  return ((h ^ (h >>> 16)) >>> 0) / 4294967296;
};

// ---- The font --------------------------------------------------------------------

// 5 × 7 glyphs, rows top first.
const FONT = {
  A: ["01110", "10001", "10001", "11111", "10001", "10001", "10001"],
  B: ["11110", "10001", "10001", "11110", "10001", "10001", "11110"],
  C: ["01110", "10001", "10000", "10000", "10000", "10001", "01110"],
  D: ["11110", "10001", "10001", "10001", "10001", "10001", "11110"],
  E: ["11111", "10000", "10000", "11110", "10000", "10000", "11111"],
  F: ["11111", "10000", "10000", "11110", "10000", "10000", "10000"],
  G: ["01110", "10001", "10000", "10111", "10001", "10001", "01111"],
  H: ["10001", "10001", "10001", "11111", "10001", "10001", "10001"],
  I: ["01110", "00100", "00100", "00100", "00100", "00100", "01110"],
  J: ["00111", "00010", "00010", "00010", "00010", "10010", "01100"],
  K: ["10001", "10010", "10100", "11000", "10100", "10010", "10001"],
  L: ["10000", "10000", "10000", "10000", "10000", "10000", "11111"],
  M: ["10001", "11011", "10101", "10101", "10001", "10001", "10001"],
  N: ["10001", "10001", "11001", "10101", "10011", "10001", "10001"],
  O: ["01110", "10001", "10001", "10001", "10001", "10001", "01110"],
  P: ["11110", "10001", "10001", "11110", "10000", "10000", "10000"],
  Q: ["01110", "10001", "10001", "10001", "10101", "10010", "01101"],
  R: ["11110", "10001", "10001", "11110", "10100", "10010", "10001"],
  S: ["01111", "10000", "10000", "01110", "00001", "00001", "11110"],
  T: ["11111", "00100", "00100", "00100", "00100", "00100", "00100"],
  U: ["10001", "10001", "10001", "10001", "10001", "10001", "01110"],
  V: ["10001", "10001", "10001", "10001", "10001", "01010", "00100"],
  W: ["10001", "10001", "10001", "10101", "10101", "10101", "01010"],
  X: ["10001", "10001", "01010", "00100", "01010", "10001", "10001"],
  Y: ["10001", "10001", "01010", "00100", "00100", "00100", "00100"],
  Z: ["11111", "00001", "00010", "00100", "01000", "10000", "11111"],
  0: ["01110", "10001", "10011", "10101", "11001", "10001", "01110"],
  1: ["00100", "01100", "00100", "00100", "00100", "00100", "01110"],
  2: ["01110", "10001", "00001", "00010", "00100", "01000", "11111"],
  3: ["11111", "00010", "00100", "00010", "00001", "10001", "01110"],
  4: ["00010", "00110", "01010", "10010", "11111", "00010", "00010"],
  5: ["11111", "10000", "11110", "00001", "00001", "10001", "01110"],
  6: ["00110", "01000", "10000", "11110", "10001", "10001", "01110"],
  7: ["11111", "00001", "00010", "00100", "01000", "01000", "01000"],
  8: ["01110", "10001", "10001", "01110", "10001", "10001", "01110"],
  9: ["01110", "10001", "10001", "01111", "00001", "00010", "01100"],
  "-": ["00000", "00000", "00000", "11111", "00000", "00000", "00000"],
  ".": ["00000", "00000", "00000", "00000", "00000", "01100", "01100"],
  "/": ["00001", "00010", "00010", "00100", "01000", "01000", "10000"],
  ":": ["00000", "01100", "01100", "00000", "01100", "01100", "00000"],
  "&": ["01100", "10010", "10100", "01000", "10101", "10010", "01101"],
  " ": ["00000", "00000", "00000", "00000", "00000", "00000", "00000"],
};
const ADVANCE = 6;
const textWidth = (s, k = 1) => (s.length * ADVANCE - 1) * k;

// ---- Cells -----------------------------------------------------------------------

// A cell's canvas: `w` × `h` RGB in 0–1.
const canvas = (w, h, fill) => {
  const px = Array.from({ length: w * h }, () => [...fill]);
  const at = (x, y) => (x >= 0 && y >= 0 && x < w && y < h ? px[y * w + x] : null);
  const set = (x, y, c) => {
    if (x >= 0 && y >= 0 && x < w && y < h) px[y * w + x] = [...c];
  };
  const rect = (x0, y0, x1, y1, c) => {
    for (let y = Math.max(0, y0); y < Math.min(h, y1); y++) for (let x = Math.max(0, x0); x < Math.min(w, x1); x++) set(x, y, c);
  };
  // `s` from (x, y) at scale `k`; `dead(i)` letters in `off` instead.
  const text = (s, x, y, c, k = 1, dead = () => false, off = c) => {
    [...s].forEach((ch, i) => {
      const g = FONT[ch];
      if (!g) throw new Error(`no glyph ${ch}`);
      const color = dead(i) ? off : c;
      g.forEach((row, gy) => [...row].forEach((bit, gx) => {
        if (bit === "1") rect(x + (i * ADVANCE + gx) * k, y + gy * k, x + (i * ADVANCE + gx + 1) * k, y + (gy + 1) * k, color);
      }));
    });
  };
  const centred = (s, y, c, k = 1, dead, off) => text(s, Math.floor((w - textWidth(s, k)) / 2), y, c, k, dead, off);
  const frame = (c, t = 1) => {
    rect(0, 0, w, t, c);
    rect(0, h - t, w, h, c);
    rect(0, 0, t, h, c);
    rect(w - t, 0, w, h, c);
  };
  return { w, h, px, at, set, rect, text, centred, frame };
};
const mul = (c, k) => c.map((v) => v * k);
const mix = (a, b, t) => a.map((v, i) => v + (b[i] - v) * t);

const WHITE = [0.92, 0.92, 0.9];
const RED = [0.72, 0.1, 0.1];
const GREEN = [0.1, 0.42, 0.24];
const BLUE = [0.12, 0.24, 0.55];
const INK = [0.1, 0.1, 0.11];
const YELLOW = [0.86, 0.72, 0.16];

// Windows. Glass: a dark interior, a little lighter at the top (the sky in
// it by day). Venetian blinds come down to `drop` of the height, slats
// every 2 texels; vertical blinds fill it, a slat missing; a sheet of paper
// taped up; FOR LEASE in red on white.
const glassDay = (c) => {
  for (let y = 0; y < c.h; y++) c.rect(0, y, c.w, y + 1, mix([0.3, 0.34, 0.4], [0.17, 0.19, 0.24], y / c.h));
  // A streak of sky across it.
  for (let y = 0; y < c.h; y++) for (let x = 0; x < c.w; x++) if (Math.abs(x - 0.7 * c.w + 0.6 * y) < 2) c.set(x, y, mix(c.at(x, y), [0.5, 0.55, 0.62], 0.35));
};
const glassNight = (c) => {
  for (let y = 0; y < c.h; y++) c.rect(0, y, c.w, y + 1, mix([0.05, 0.055, 0.07], [0.025, 0.025, 0.035], y / c.h));
};
const blinds = (c, drop, slat, gap) => {
  const bottom = Math.round(c.h * drop);
  for (let y = 0; y < bottom; y++) c.rect(0, y, c.w, y + 1, y % 2 === 0 ? slat : gap);
  c.rect(0, bottom - 1, c.w, bottom + 1, mul(slat, 0.8));
  // The cord down one side.
  c.rect(3, bottom, 4, Math.min(c.h, bottom + 6), mul(slat, 0.7));
};
const verticals = (c, slat, gap) => {
  const missing = Math.floor(c.w / 4 / 2);
  for (let x = 0; x < c.w; x++) {
    const k = Math.floor(x / 4);
    if (k === missing) continue;
    c.rect(x, 0, x + 1, c.h, x % 4 === 3 ? gap : slat);
  }
};
const paper = (c, light) => {
  const x0 = Math.floor(c.w * 0.58);
  const y0 = Math.floor(c.h * 0.2);
  c.rect(x0, y0, x0 + 9, y0 + 12, light);
  for (let k = 0; k < 4; k++) c.rect(x0 + 1, y0 + 2 + 2 * k, x0 + 2 + Math.round(6 * hash(k, c.w, 3)), y0 + 3 + 2 * k, mul(light, 0.45));
};
const leaseSign = (c, k) => {
  const w = 31;
  const x0 = Math.floor((c.w - w) / 2);
  const y0 = Math.floor(c.h * 0.3);
  c.rect(x0, y0, x0 + w, y0 + 20, mul(WHITE, k));
  c.rect(x0, y0, x0 + w, y0 + 10, mul(RED, k));
  c.text("FOR", x0 + 7, y0 + 2, mul(WHITE, k));
  c.text("LEASE", x0 + 1, y0 + 12, mul(RED, k));
};
const lit = (c, warm) => {
  const base = warm ? [0.95, 0.78, 0.46] : [0.78, 0.86, 0.74];
  for (let y = 0; y < c.h; y++) c.rect(0, y, c.w, y + 1, mix(base, mul(base, 0.72), y / c.h));
  // A panel's glare along the top.
  c.rect(2, 1, c.w - 2, 3, mix(base, [1, 1, 1], 0.6));
};
const windowCells = (shape, w, h) => {
  const slatDay = [0.72, 0.7, 0.64];
  const gapDay = [0.56, 0.55, 0.5];
  const slatNight = [0.1, 0.1, 0.11];
  const gapNight = [0.07, 0.07, 0.08];
  const day = (draw) => () => {
    const c = canvas(w, h, [0, 0, 0]);
    glassDay(c);
    draw(c);
    return c;
  };
  const night = (draw) => () => {
    const c = canvas(w, h, [0, 0, 0]);
    glassNight(c);
    draw(c);
    return c;
  };
  return [
    [`${shape}Dark`, day(() => {})],
    [`${shape}Blinds1`, day((c) => blinds(c, 0.3, slatDay, gapDay))],
    [`${shape}Blinds2`, day((c) => blinds(c, 0.55, slatDay, gapDay))],
    [`${shape}Blinds3`, day((c) => blinds(c, 0.85, slatDay, gapDay))],
    [`${shape}Verticals`, day((c) => verticals(c, [0.66, 0.66, 0.62], [0.42, 0.42, 0.4]))],
    [`${shape}Paper`, day((c) => (blinds(c, 0.2, slatDay, gapDay), paper(c, [0.88, 0.88, 0.84])))],
    [`${shape}Lease`, day((c) => leaseSign(c, 1))],
    [`${shape}DarkNight`, night(() => {})],
    [`${shape}BlindsNight`, night((c) => blinds(c, 0.55, slatNight, gapNight))],
    [`${shape}VerticalsNight`, night((c) => verticals(c, [0.09, 0.09, 0.1], [0.05, 0.05, 0.06]))],
    [`${shape}LeaseNight`, night((c) => leaseSign(c, 0.3))],
    [`${shape}LitWarm`, () => {
      const c = canvas(w, h, [0, 0, 0]);
      lit(c, true);
      blinds(c, 0.4, [0.62, 0.5, 0.3], [0.8, 0.65, 0.4]);
      return c;
    }],
    [`${shape}LitCool`, () => {
      const c = canvas(w, h, [0, 0, 0]);
      lit(c, false);
      verticals(c, [0.6, 0.66, 0.58], [0.74, 0.82, 0.7]);
      return c;
    }],
  ];
};

// A pole sign's plate: a frame, lines of text, a red P struck through.
const poleSign = (lines, color) => () => {
  const c = canvas(48, 56, WHITE);
  c.frame(color, 2);
  lines.forEach((s, i) => c.centred(s, 5 + 9 * i, color));
  return c;
};
const noParking = () => {
  const c = canvas(48, 56, WHITE);
  c.frame(RED, 2);
  // The struck-through P in a ring.
  for (let y = 0; y < 22; y++) for (let x = 0; x < 22; x++) {
    const d = Math.hypot(x - 10.5, y - 10.5);
    if (d < 10.5 && d > 8.2) c.set(13 + x, 4 + y, RED);
  }
  c.text("P", 22, 11, INK);
  for (let k = 0; k < 15; k++) c.rect(17 + k, 22 - k, 19 + k, 24 - k, RED);
  c.centred("NO", 29, RED);
  c.centred("PARKING", 38, RED);
  return c;
};
// A plaque: one line, light on dark.
const plaque = (s, w, fg, bg) => () => {
  const c = canvas(w, 13, bg);
  c.centred(s, 3, fg);
  return c;
};
// The tenant directory: a header, then a suite number and a name line on
// each row; some suites vacant.
const directory = (title, first) => () => {
  const c = canvas(48, 96, [0.12, 0.13, 0.15]);
  c.frame([0.42, 0.42, 0.4], 1);
  c.centred(title, 4, WHITE);
  c.rect(4, 13, 44, 14, [0.5, 0.5, 0.48]);
  for (let r = 0; r < 8; r++) {
    const y = 18 + 9 * r;
    c.text(String(first + 10 * r), 3, y, [0.8, 0.78, 0.7]);
    if (hash(r, first, 9) < 0.7) c.rect(23, y + 2, 23 + 6 + Math.round(16 * hash(r, first, 11)), y + 5, [0.62, 0.62, 0.58]);
  }
  return c;
};
// A lot row marker: a letter on blue.
const rowMarker = (s) => () => {
  const c = canvas(16, 16, BLUE);
  c.frame(WHITE, 1);
  c.centred(s, 5, WHITE);
  return c;
};
// A sign box's face by night: lit letters on a cream face, `dead` ones out.
const boxSign = (s, w, dead) => () => {
  const c = canvas(w, 15, [0.9, 0.86, 0.74]);
  c.frame([0.3, 0.28, 0.26], 1);
  c.centred(s, 4, [0.78, 0.12, 0.1], 1, (i) => dead.includes(i), [0.62, 0.52, 0.48]);
  return c;
};
// A vending machine's front: a lit header (letters dead), four rows of cans
// behind glass, a pay panel and a dark tray.
const vending = () => {
  const c = canvas(40, 80, [0.08, 0.1, 0.16]);
  c.rect(0, 0, 40, 12, [0.78, 0.86, 0.92]);
  c.centred("COLD", 3, [0.12, 0.3, 0.62], 1, (i) => i === 1, [0.6, 0.68, 0.74]);
  const cans = [[0.78, 0.14, 0.12], [0.16, 0.5, 0.2], [0.9, 0.62, 0.12], [0.14, 0.3, 0.7], [0.86, 0.86, 0.82]];
  for (let r = 0; r < 4; r++) for (let k = 0; k < 5; k++) {
    const x = 3 + 5 * k;
    const y = 16 + 13 * r;
    c.rect(x, y, x + 3, y + 9, cans[Math.floor(hash(r, k, 13) * cans.length)]);
    c.rect(x, y + 10, x + 3, y + 11, [0.5, 0.52, 0.5]);
  }
  c.rect(29, 16, 38, 60, [0.3, 0.32, 0.36]);
  c.rect(31, 20, 36, 24, [0.15, 0.8, 0.3]);
  for (let k = 0; k < 4; k++) c.rect(31, 28 + 5 * k, 36, 31 + 5 * k, [0.62, 0.62, 0.64]);
  c.rect(4, 66, 26, 74, [0.02, 0.02, 0.03]);
  return c;
};
// A payphone's sign, white on blue.
const phone = () => {
  const c = canvas(32, 11, BLUE);
  c.centred("PHONE", 2, WHITE);
  return c;
};
// A kerb ramp: concrete like the paving, its lower 40% (by the road) a
// yellow tactile strip of raised dots.
const tactile = () => {
  const c = canvas(16, 16, [0.5, 0.5, 0.48]);
  for (let y = 0; y < 16; y++) for (let x = 0; x < 16; x++) {
    if (y < 10) c.set(x, y, mul([0.5, 0.5, 0.48], 0.94 + 0.12 * hash(x, y, 71)));
    else c.set(x, y, Math.hypot((x % 4) - 1.5, (y % 3) - 1) < 1.0 ? [0.56, 0.46, 0.14] : [0.74, 0.62, 0.2]);
  }
  return c;
};
// The vacant lot's board.
const available = () => {
  const c = canvas(64, 40, WHITE);
  c.frame(GREEN, 2);
  c.rect(2, 2, 62, 14, GREEN);
  c.centred("AVAILABLE", 4, WHITE);
  c.centred("2.5 ACRES", 18, INK);
  c.centred("555-0199", 28, GREEN);
  return c;
};

const CELLS = [
  ...windowCells("band", 64, 38),
  ...windowCells("bay", 32, 38),
  ["noParking", noParking],
  ["fireLane", poleSign(["FIRE", "LANE", "NO", "PARKING"], RED)],
  ["visitor", poleSign(["VISITOR", "PARKING", "ONLY", "8-6"], BLUE)],
  ["suite100", plaque("SUITES 100-170", 96, WHITE, [0.14, 0.14, 0.16])],
  ["suite200", plaque("LEVEL 2 200-270", 96, WHITE, [0.14, 0.14, 0.16])],
  ["lobby", plaque("LOBBY", 48, WHITE, [0.14, 0.14, 0.16])],
  ["number1200", plaque("1200", 32, [0.86, 0.84, 0.78], [0.2, 0.2, 0.22])],
  ["number400", plaque("400", 32, [0.86, 0.84, 0.78], [0.2, 0.2, 0.22])],
  ["directory", directory("TENANTS", 100)],
  ["directoryTower", directory("FLOORS", 200)],
  ["rowA", rowMarker("A")],
  ["rowB", rowMarker("B")],
  ["rowC", rowMarker("C")],
  ["rowD", rowMarker("D")],
  ["rowP", rowMarker("P")],
  ["boxOffice", boxSign("BUSINESS CENTRE", 96, [1, 10])],
  ["boxLot", boxSign("FREE PARKING", 80, [2, 8])],
  ["vending", vending],
  ["phone", phone],
  ["tactile", tactile],
  ["available", available],
];

// ---- Packing ---------------------------------------------------------------------

// Shelves left to right, each cell padded by `PAD` texels of its own edge
// (so mip levels don't bleed a neighbour in).
const WIDTH = 512;
const PAD = 3;
const drawn = CELLS.map(([name, make]) => ({ name, c: make() }));
let x = 0;
let y = 0;
let shelf = 0;
for (const d of drawn) {
  const w = d.c.w + 2 * PAD;
  const h = d.c.h + 2 * PAD;
  if (x + w > WIDTH) {
    x = 0;
    y += shelf;
    shelf = 0;
  }
  d.x = x + PAD;
  d.y = y + PAD;
  x += w;
  shelf = Math.max(shelf, h);
}
let HEIGHT = 1;
while (HEIGHT < y + shelf) HEIGHT *= 2;

const pixels = new Uint8Array(WIDTH * HEIGHT * 3);
const byte = (v) => Math.max(0, Math.min(255, Math.round(v * 255)));
for (const d of drawn) {
  for (let py = -PAD; py < d.c.h + PAD; py++) {
    for (let px = -PAD; px < d.c.w + PAD; px++) {
      const c = d.c.at(Math.min(d.c.w - 1, Math.max(0, px)), Math.min(d.c.h - 1, Math.max(0, py)));
      pixels.set(c.map(byte), ((d.y + py) * WIDTH + d.x + px) * 3);
    }
  }
}
const here = (file) => fileURLToPath(new URL(file, import.meta.url));
writeFileSync(here("street-atlas.png"), encodePng(WIDTH, HEIGHT, 8, 2, pixels));

// Each cell's rect, half a texel in from its edges.
const num = (v) => {
  const s = String(Number(v.toFixed(6)));
  return s.includes(".") ? s : `${s}.0`;
};
const lines = drawn.map((d) => {
  const u0 = (d.x + 0.5) / WIDTH;
  const v0 = (d.y + 0.5) / HEIGHT;
  const u1 = (d.x + d.c.w - 0.5) / WIDTH;
  const v1 = (d.y + d.c.h - 0.5) / HEIGHT;
  return `let ${d.name}: Cell = { u0: ${num(u0)}, v0: ${num(v0)}, u1: ${num(u1)}, v1: ${num(v1)} }`;
});
writeFileSync(here("streetAtlas.fun"), [
  "// Generated by streetscape.mjs: each cell of street-atlas.png as a UV rect",
  "// (glTF's convention: 0, 0 is the image's top-left). Don't edit; rerun it.",
  "",
  "type Cell = { u0: float, v0: float, u1: float, v1: float }",
  "",
  ...lines,
  "",
].join("\n"));
console.log(`street-atlas.png ${WIDTH}×${HEIGHT}, ${drawn.length} cells`);
