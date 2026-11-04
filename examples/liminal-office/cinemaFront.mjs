// Generate the cinema front's textures (cinemaFront.fun): `cinema-atlas.png`,
// its marquee letter boards, lit soffit, posters (the boards and posters
// faded too, for the day), blade sign and booth signs, with `cinemaAtlas.fun`, each cell's UV rect; and
// `cinema-terrazzo.png`, the forecourt's tileable terrazzo. Node built-ins
// only; re-running reproduces the files byte for byte (with the same Node:
// its zlib does the deflate):
//
//   node examples/liminal-office/cinemaFront.mjs
//
// Text is the street atlas's 5 × 7 pixel font. Every title is made up.
import { writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { encodePng } from "../../scripts/png.mjs";

// Deterministic hash noise in [0, 1).
const hash = (x, y, seed) => {
  let h = (Math.imul(x, 374761393) + Math.imul(y, 668265263) + Math.imul(seed, 1442695041)) | 0;
  h = Math.imul(h ^ (h >>> 13), 1274126177);
  return ((h ^ (h >>> 16)) >>> 0) / 4294967296;
};

// 5 × 7 glyphs, rows top first (streetscape.mjs's).
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
  K: ["10001", "10010", "10100", "11000", "10100", "10010", "10001"],
  L: ["10000", "10000", "10000", "10000", "10000", "10000", "11111"],
  M: ["10001", "11011", "10101", "10101", "10001", "10001", "10001"],
  N: ["10001", "10001", "11001", "10101", "10011", "10001", "10001"],
  O: ["01110", "10001", "10001", "10001", "10001", "10001", "01110"],
  P: ["11110", "10001", "10001", "11110", "10000", "10000", "10000"],
  R: ["11110", "10001", "10001", "11110", "10100", "10010", "10001"],
  S: ["01111", "10000", "10000", "01110", "00001", "00001", "11110"],
  T: ["11111", "00100", "00100", "00100", "00100", "00100", "00100"],
  U: ["10001", "10001", "10001", "10001", "10001", "10001", "01110"],
  W: ["10001", "10001", "10001", "10101", "10101", "10101", "01010"],
  X: ["10001", "10001", "01010", "00100", "01010", "10001", "10001"],
  Y: ["10001", "10001", "01010", "00100", "00100", "00100", "00100"],
  0: ["01110", "10001", "10011", "10101", "11001", "10001", "01110"],
  1: ["00100", "01100", "00100", "00100", "00100", "00100", "01110"],
  2: ["01110", "10001", "00001", "00010", "00100", "01000", "11111"],
  3: ["11111", "00010", "00100", "00010", "00001", "10001", "01110"],
  4: ["00010", "00110", "01010", "10010", "11111", "00010", "00010"],
  5: ["11111", "10000", "11110", "00001", "00001", "10001", "01110"],
  7: ["11111", "00001", "00010", "00100", "01000", "01000", "01000"],
  9: ["01110", "10001", "10001", "01111", "00001", "00010", "01100"],
  ":": ["00000", "01100", "01100", "00000", "01100", "01100", "00000"],
  " ": ["00000", "00000", "00000", "00000", "00000", "00000", "00000"],
};
const ADVANCE = 6;
const textWidth = (s, k) => (s.length * ADVANCE - 1) * k;

// A cell's canvas: `w` × `h` RGB in 0–1.
const canvas = (w, h, fill) => {
  const px = Array.from({ length: w * h }, () => [...fill]);
  const at = (x, y) => px[Math.min(h - 1, Math.max(0, y)) * w + Math.min(w - 1, Math.max(0, x))];
  const set = (x, y, c) => {
    if (x >= 0 && y >= 0 && x < w && y < h) px[y * w + x] = [...c];
  };
  const rect = (x0, y0, x1, y1, c) => {
    for (let y = Math.max(0, y0); y < Math.min(h, y1); y++) for (let x = Math.max(0, x0); x < Math.min(w, x1); x++) set(x, y, c);
  };
  // `s` centred on row `y` at scale `k`; `style(i)` gives letter i's
  // colour and drop in texels, or null for a letter that has fallen off.
  const centred = (s, y, k, style) => {
    const x = Math.floor((w - textWidth(s, k)) / 2);
    [...s].forEach((ch, i) => {
      const g = FONT[ch];
      if (!g) throw new Error(`no glyph ${ch}`);
      const st = style(i);
      if (!st) return;
      g.forEach((row, gy) => [...row].forEach((bit, gx) => {
        if (bit === "1") rect(x + (i * ADVANCE + gx) * k, y + st.drop + gy * k, x + (i * ADVANCE + gx + 1) * k, y + st.drop + (gy + 1) * k, st.color);
      }));
    });
  };
  const frame = (c, t = 1) => {
    rect(0, 0, w, t, c);
    rect(0, h - t, w, h, c);
    rect(0, 0, t, h, c);
    rect(w - t, 0, w, h, c);
  };
  return { w, h, px, at, set, rect, centred, frame };
};
const mix = (a, b, t) => a.map((v, i) => v + (b[i] - v) * t);

const INK = [0.08, 0.07, 0.08];
const RED = [0.7, 0.1, 0.12];
const CREAM = [0.97, 0.93, 0.82];

// ---- The marquee -----------------------------------------------------------------

// A letter board at 32 texels a metre: a cream panel lit from behind by
// tubes (a brighter band every 1.2 m, one tube failing), the letter tracks,
// and two rows of 44 cm letters, a few hanging crooked and a few gone.
const board = (w, rows, seed) => () => {
  const c = canvas(w, 40, CREAM);
  for (let y = 0; y < c.h; y++) for (let x = 0; x < c.w; x++) {
    const tube = 0.93 + 0.07 * Math.cos((2 * Math.PI * x) / 38);
    const failing = x > w * 0.68 && x < w * 0.68 + 30 ? 0.78 : 1;
    c.set(x, y, CREAM.map((v) => v * tube * failing * (0.97 + 0.03 * hash(x, y, seed))));
  }
  for (const y of [0, 19, 20, 39]) c.rect(0, y, w, y + 1, [0.58, 0.55, 0.5]);
  rows.forEach(([s, red], r) => c.centred(s, 3 + 19 * r, 2, (i) => {
    const h = hash(i, r, seed);
    if (s[i] !== " " && h < 0.04) return null;
    return { color: red.includes(i) ? RED : INK, drop: h > 0.93 ? 1 : 0 };
  }));
  return c;
};
// Indices `a` to `b` - 1.
const span = (a, b) => Array.from({ length: b - a }, (_, i) => a + i);

// The marquee's soffit, lit: warm panels between a grid of dark rails.
const soffit = () => {
  const c = canvas(128, 24, [1, 0.93, 0.74]);
  for (let y = 0; y < c.h; y++) for (let x = 0; x < c.w; x++) {
    if (x % 16 === 0 || y % 8 === 0) c.set(x, y, [0.5, 0.42, 0.32]);
    else c.set(x, y, mix([1, 0.95, 0.8], [0.92, 0.8, 0.58], Math.abs((y % 8) - 4) / 4));
  }
  return c;
};

// ---- Posters ---------------------------------------------------------------------

// A one-sheet: a design over a dark frame, a strip of blocks for a title and
// three lines of credits (nothing legible).
const poster = (seed, paint) => () => {
  const c = canvas(32, 52, [0.06, 0.05, 0.07]);
  paint(c);
  c.frame([0.06, 0.05, 0.07], 1);
  c.rect(1, 38, 31, 51, [0.06, 0.05, 0.07]);
  for (let k = 0; k < 3; k++) c.rect(4 + 4 * k, 40, 7 + 4 * k, 43, [0.95, 0.9, 0.78]);
  for (let k = 0; k < 3; k++) c.rect(4, 45 + 2 * k, 10 + Math.round(16 * hash(k, seed, 3)), 46 + 2 * k, [0.62, 0.6, 0.56]);
  return c;
};
const gradient = (c, top, bottom, h = 38) => {
  for (let y = 0; y < h; y++) c.rect(0, y, c.w, y + 1, mix(top, bottom, y / h));
};
const POSTERS = [
  // A sun setting through bands.
  ["poster0", poster(1, (c) => {
    gradient(c, [0.45, 0.1, 0.4], [0.95, 0.45, 0.2]);
    for (let y = 0; y < 38; y++) for (let x = 0; x < 32; x++) {
      if (Math.hypot(x - 15.5, y - 22) < 9 && (y < 22 || y % 3 !== 0)) c.set(x, y, [1, 0.85, 0.3]);
    }
    c.rect(0, 31, 32, 38, [0.1, 0.05, 0.12]);
  })],
  // A lit doorway and its long shadow.
  ["poster1", poster(2, (c) => {
    gradient(c, [0.1, 0.4, 0.42], [0.05, 0.22, 0.26]);
    c.rect(12, 8, 20, 26, [0.95, 0.94, 0.86]);
    for (let y = 26; y < 38; y++) c.rect(12 + (y - 26), y, 20 + 2 * (y - 26), y + 1, [0.03, 0.12, 0.14]);
  })],
  // Stars over a pink peak.
  ["poster2", poster(3, (c) => {
    gradient(c, [0.04, 0.05, 0.16], [0.12, 0.08, 0.28]);
    for (let y = 0; y < 38; y++) for (let x = 0; x < 32; x++) if (hash(x, y, 31) < 0.05) c.set(x, y, [0.85, 0.85, 0.95]);
    for (let y = 18; y < 38; y++) c.rect(16 - Math.round((y - 18) * 0.7), y, 16 + Math.round((y - 18) * 0.7), y + 1, [1, 0.3, 0.72]);
  })],
  // Rings.
  ["poster3", poster(4, (c) => {
    gradient(c, [0.9, 0.72, 0.2], [0.85, 0.55, 0.15]);
    for (let y = 0; y < 38; y++) for (let x = 0; x < 32; x++) if (Math.floor(Math.hypot(x - 15.5, y - 18) / 2.5) % 2 === 1 && Math.hypot(x - 15.5, y - 18) < 14) c.set(x, y, [0.1, 0.06, 0.08]);
  })],
  // A corridor to a vanishing point.
  ["poster4", poster(5, (c) => {
    gradient(c, [0.32, 0.12, 0.5], [0.15, 0.05, 0.25]);
    for (let y = 0; y < 38; y++) for (let x = 0; x < 32; x++) {
      const dx = x - 15.5;
      const dy = y - 16;
      if (Math.abs(dx) < 2 && Math.abs(dy) < 3) c.set(x, y, [0.95, 0.9, 0.6]);
      else if (Math.abs(Math.abs(dx) - Math.abs(dy) * 0.9) < 0.6) c.set(x, y, [0.75, 0.55, 0.95]);
    }
  })],
  // Waves.
  ["poster5", poster(6, (c) => {
    gradient(c, [0.05, 0.14, 0.3], [0.02, 0.06, 0.14]);
    for (let k = 0; k < 4; k++) for (let x = 0; x < 32; x++) {
      const y = Math.round(12 + 6 * k + 2.5 * Math.sin(x / 3 + k));
      c.rect(x, y, x + 1, y + 2, mix([0.3, 0.95, 0.95], [0.1, 0.4, 0.7], k / 3));
    }
  })],
];
// The forecourt's A-frame boards: two lines over a design.
const aframe = (a, b, paint) => () => {
  const c = canvas(44, 64, [0.06, 0.05, 0.07]);
  paint(c);
  c.frame([0.2, 0.16, 0.12], 2);
  c.rect(2, 2, 42, 22, [0.06, 0.05, 0.07]);
  c.centred(a, 4, 1, () => ({ color: CREAM, drop: 0 }));
  c.centred(b, 13, 1, () => ({ color: [1, 0.4, 0.75], drop: 0 }));
  return c;
};

// ---- The blade sign and the booth --------------------------------------------------

// The blade's face: CINEMA down a maroon panel inside a gilt line, its N's
// tube dead.
const blade = () => {
  const c = canvas(24, 104, [0.34, 0.07, 0.12]);
  c.frame([0.34, 0.07, 0.12], 2);
  c.rect(2, 2, 22, 3, [0.85, 0.7, 0.4]);
  c.rect(2, 101, 22, 102, [0.85, 0.7, 0.4]);
  c.rect(2, 2, 3, 102, [0.85, 0.7, 0.4]);
  c.rect(21, 2, 22, 102, [0.85, 0.7, 0.4]);
  [..."CINEMA"].forEach((ch, i) => {
    const glyph = canvas(24, 16, [0, 0, 0]);
    glyph.centred(ch, 1, 2, () => ({ color: [1, 1, 1], drop: 0 }));
    for (let y = 0; y < 16; y++) for (let x = 0; x < 24; x++) {
      if (glyph.at(x, y)[0] > 0) c.set(x, 5 + 16 * i + y, i === 2 ? [0.5, 0.36, 0.3] : [1, 0.92, 0.7]);
    }
  });
  return c;
};
const tickets = () => {
  const c = canvas(48, 11, CREAM);
  c.centred("TICKETS", 2, 1, () => ({ color: RED, drop: 0 }));
  return c;
};
// The booth's speaker: a chrome plate, a ring of holes.
const grille = () => {
  const c = canvas(16, 16, [0.7, 0.7, 0.72]);
  c.frame([0.45, 0.45, 0.48], 1);
  for (let y = 0; y < 16; y++) for (let x = 0; x < 16; x++) {
    if (Math.hypot(x - 7.5, y - 7.5) < 6 && (x + y) % 2 === 0) c.set(x, y, [0.12, 0.12, 0.13]);
  }
  return c;
};

// A cell as the day shows it, unlit behind its glass: sun-faded, toward
// a pale grey.
const faded = (make) => () => {
  const c = make();
  c.px.forEach((p, i) => {
    const lum = 0.3 * p[0] + 0.59 * p[1] + 0.11 * p[2];
    c.px[i] = mix(mix(p, [lum, lum, lum], 0.45), [0.8, 0.78, 0.76], 0.3);
  });
  return c;
};

const BOARD_FRONT = board(680, [["NOW SHOWING  ROOM 203  7:15 9:40", span(0, 11)], ["WAITING AREA  STAIR B  LATE SHOW SAT", span(23, 36)]], 7);
const BOARD_SIDE = board(116, [["ROOM 203", []], ["7:15 9:40", span(0, 9)]], 9);
const CELLS = [
  ["boardFront", BOARD_FRONT],
  ["boardSide", BOARD_SIDE],
  ["soffit", soffit],
  ["boardFrontDay", faded(BOARD_FRONT)],
  ["boardSideDay", faded(BOARD_SIDE)],
  ...POSTERS,
  ...POSTERS.map(([name, make]) => [`${name}Day`, faded(make)]),
  ["aframeNow", aframe("NOW", "SHOWING", (c) => gradient(c, [0.1, 0.4, 0.42], [0.05, 0.22, 0.26], 64))],
  ["aframeSoon", aframe("COMING", "SOON", (c) => gradient(c, [0.45, 0.1, 0.4], [0.95, 0.45, 0.2], 64))],
  ["blade", blade],
  ["tickets", tickets],
  ["grille", grille],
];

// ---- Packing ---------------------------------------------------------------------

// Shelves left to right, each cell padded by `PAD` texels of its own edge
// (so mip levels don't bleed a neighbour in).
const WIDTH = 1024;
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

const byte = (v) => Math.max(0, Math.min(255, Math.round(v * 255)));
const pixels = new Uint8Array(WIDTH * HEIGHT * 3);
for (const d of drawn) {
  for (let py = -PAD; py < d.c.h + PAD; py++) {
    for (let px = -PAD; px < d.c.w + PAD; px++) {
      pixels.set(d.c.at(px, py).map(byte), ((d.y + py) * WIDTH + d.x + px) * 3);
    }
  }
}
const here = (file) => fileURLToPath(new URL(file, import.meta.url));
writeFileSync(here("cinema-atlas.png"), encodePng(WIDTH, HEIGHT, 8, 2, pixels));

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
  return `let ${d.name}: StreetAtlas.Cell = { u0: ${num(u0)}, v0: ${num(v0)}, u1: ${num(u1)}, v1: ${num(v1)} }`;
});
writeFileSync(here("cinemaAtlas.fun"), [
  "// Generated by cinemaFront.mjs: each cell of cinema-atlas.png as a UV rect",
  "// (glTF's convention: 0, 0 is the image's top-left). Don't edit; rerun it.",
  "",
  ...lines,
  "",
].join("\n"));

// ---- Terrazzo --------------------------------------------------------------------

// One 1.2 m repeat at 128 texels: a pale matrix, mottled, set with chips of
// pink, teal, charcoal and ochre stone, a brass strip along two edges (so
// the repeats meet in a grid).
const T = 128;
const CHIPS = [[0.86, 0.56, 0.6], [0.4, 0.64, 0.64], [0.28, 0.27, 0.3], [0.64, 0.63, 0.62], [0.8, 0.67, 0.46]];
const terrazzo = new Uint8Array(T * T * 3);
for (let ty = 0; ty < T; ty++) {
  for (let tx = 0; tx < T; tx++) {
    let c = [0.9, 0.88, 0.84].map((v) => v * (0.96 + 0.05 * hash(tx >> 3, ty >> 3, 41) + 0.03 * hash(tx, ty, 43)));
    // A chip in some 3 × 3 cells, 1 or 2 texels across.
    const cx = Math.floor(tx / 3);
    const cy = Math.floor(ty / 3);
    if (hash(cx, cy, 47) < 0.32) {
      const size = hash(cx, cy, 53) < 0.4 ? 2 : 1;
      if (tx % 3 < size && ty % 3 < size) c = CHIPS[Math.floor(hash(cx, cy, 59) * CHIPS.length)];
    }
    if (tx === 0 || ty === 0) c = [0.74, 0.6, 0.36];
    terrazzo.set(c.map(byte), (ty * T + tx) * 3);
  }
}
writeFileSync(here("cinema-terrazzo.png"), encodePng(T, T, 8, 2, terrazzo));
console.log(`cinema-atlas.png ${WIDTH}×${HEIGHT}, ${drawn.length} cells; cinema-terrazzo.png ${T}×${T}`);
