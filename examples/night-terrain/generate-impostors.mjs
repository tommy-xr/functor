// Bake the fir impostor cards: `impostor-fir-{a,b,c}-{near,far}.glb`.
//
//   npm run build:cli:debug   # once: the bake renders through the functor CLI
//   node examples/night-terrain/generate-impostors.mjs
//
// For each of the three Poly Haven firs (`fir_sapling_medium`, CC0) this
// fetches the FULL-detail model (~400–700k triangles, needles and all),
// renders it side-on through a long lens against a key colour with the
// headless `--capture-frame` path, and turns the capture into crossed vertical
// cards. The renderer has no alpha cutout or per-texel blending, so the cards
// cut the silhouette out in GEOMETRY instead: the capture is gridded, and only
// cells the tree covers become quads. The background texels are recoloured to
// the foliage's mean so the stair-stepped edges never show the key colour.
//
// The bake draws the model unlit (static glTF models are), so the capture IS
// the foliage's albedo; the cards carry the lighting skin and are shaded in
// game, with normals bent out from the trunk so every plane reads as part of
// one lit crown rather than a flat board.
//
// The output is deterministic for a given CLI build and checked in; the
// full-detail models are fetched to a temp dir and discarded.
import { execFileSync } from "node:child_process";
import { mkdtemp, writeFile, readFile, rm } from "node:fs/promises";
import { deflateSync, inflateSync } from "node:zlib";
import { fileURLToPath } from "node:url";
import os from "node:os";
import path from "node:path";
import { appendView, skinForLighting, writeGlb } from "../../scripts/glb.mjs";
import { fetchPolyHavenGlb } from "../../scripts/polyhaven-gltf.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
// `FUNCTOR` overrides the CLI; otherwise the debug build in the cargo target dir.
const FUNCTOR =
  process.env.FUNCTOR ??
  path.join(process.env.CARGO_TARGET_DIR ?? path.join(here, "../../target"), "debug/functor");

// Scene-root index in `fir_sapling_medium`, and the tree's height in metres.
const TREES = [
  { name: "a", node: 0, height: 8.26 },
  { name: "b", node: 1, height: 6.99 },
  { name: "c", node: 2, height: 5.14 },
];
const TEXTURE = { width: 384, height: 512 }; // the card's 3:4 aspect
// Near cards: three planes, fine cells. Far cards: two planes, coarse cells.
const CARDS = [
  { suffix: "near", planes: 3, cols: 72, rows: 96 },
  { suffix: "far", planes: 2, cols: 36, rows: 48 },
];
const KEY = [255, 0, 255];
const FOV_DEGREES = 4.0; // a long lens: near-orthographic, so the card is to scale

// ── PNG ─────────────────────────────────────────────────────────────────────

const crcTable = new Uint32Array(256).map((_, n) => {
  let c = n;
  for (let k = 0; k < 8; k += 1) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
const crc32 = (bytes) => {
  let crc = 0xffffffff;
  for (const b of bytes) crc = crcTable[(crc ^ b) & 0xff] ^ (crc >>> 8);
  return (crc ^ 0xffffffff) >>> 0;
};

// 8-bit RGB or RGBA, non-interlaced: what the capture path writes.
function decodePng(buf) {
  let off = 8;
  let width = 0, height = 0, channels = 0;
  const idat = [];
  while (off < buf.length) {
    const len = buf.readUInt32BE(off);
    const type = buf.toString("ascii", off + 4, off + 8);
    const data = buf.subarray(off + 8, off + 8 + len);
    if (type === "IHDR") {
      width = data.readUInt32BE(0);
      height = data.readUInt32BE(4);
      if (data[8] !== 8 || data[12] !== 0) throw new Error("expected 8-bit, non-interlaced PNG");
      channels = { 2: 3, 6: 4 }[data[9]];
      if (!channels) throw new Error(`unsupported PNG colour type ${data[9]}`);
    } else if (type === "IDAT") idat.push(data);
    off += 12 + len;
  }
  const raw = inflateSync(Buffer.concat(idat));
  const stride = width * channels;
  const out = Buffer.alloc(stride * height);
  for (let y = 0; y < height; y += 1) {
    const filter = raw[y * (stride + 1)];
    for (let i = 0; i < stride; i += 1) {
      const x = raw[y * (stride + 1) + 1 + i];
      const a = i >= channels ? out[y * stride + i - channels] : 0;
      const b = y > 0 ? out[(y - 1) * stride + i] : 0;
      const c = i >= channels && y > 0 ? out[(y - 1) * stride + i - channels] : 0;
      const p = a + b - c;
      const pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c);
      const paeth = pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
      out[y * stride + i] =
        (x + [0, a, b, (a + b) >> 1, paeth][filter]) & 0xff;
    }
  }
  return { width, height, channels, pixels: out };
}

function encodeRgbPng(width, height, rgb) {
  const stride = width * 3;
  const raw = Buffer.alloc((stride + 1) * height);
  for (let y = 0; y < height; y += 1) {
    raw[y * (stride + 1)] = 2; // Up
    for (let i = 0; i < stride; i += 1) {
      const above = y > 0 ? rgb[(y - 1) * stride + i] : 0;
      raw[y * (stride + 1) + 1 + i] = (rgb[y * stride + i] - above) & 0xff;
    }
  }
  const chunk = (name, data) => {
    const body = Buffer.concat([Buffer.from(name, "ascii"), data]);
    const out = Buffer.alloc(12 + data.length);
    out.writeUInt32BE(data.length, 0);
    body.copy(out, 4);
    out.writeUInt32BE(crc32(body), 8 + data.length);
    return out;
  };
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8;
  ihdr[9] = 2; // RGB
  return Buffer.concat([
    Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

// ── Bake ────────────────────────────────────────────────────────────────────

const bakeGame = (height) => `// Generated by generate-impostors.mjs: one tree, side-on, on a key colour.
let height = ${(height * 1.03).toFixed(3)}
let distance = height / 2.0 / Math.tan(${FOV_DEGREES / 2} * Math.pi / 180.0)
let init = {}
let tick = (model, dt, tts) => model
let draw = (model, tts) =>
  Frame.create(
    Camera3D.firstPerson(Vec3.make(0.0, height / 2.0, 0.0 - distance),
                         Angle.radians(0.0), Angle.radians(0.0), Angle.degrees(${FOV_DEGREES}))
    |> Camera3D.clip(distance - 20.0, distance + 20.0),
    Scene.model(Asset.model("tree.glb")))
  |> Frame.withClearColor(Color.rgb(1.0, 0.0, 1.0))
`;

// Foreground = far enough from the key colour (which has no green).
const isTree = (r, g, b) => g > 60 || Math.abs(r - KEY[0]) + Math.abs(b - KEY[2]) > 150;

async function bake(tree) {
  const dir = await mkdtemp(path.join(os.tmpdir(), `functor-impostor-${tree.name}-`));
  try {
    const triangles = await fetchPolyHavenGlb({
      id: "fir_sapling_medium",
      node: tree.node,
      out: path.join(dir, "tree.glb"),
    });
    await writeFile(path.join(dir, "functor.json"), '{"language": "functor-lang", "entry": "game.fun"}\n');
    await writeFile(path.join(dir, "game.fun"), bakeGame(tree.height));
    const shot = path.join(dir, "bake.png");
    execFileSync(FUNCTOR, [
      "-d", dir, "run", "native", "--",
      "--fixed-time", "1.0", "--capture-time", "3.0",
      "--capture-size", `${TEXTURE.width}x${TEXTURE.height}`,
      "--capture-frame", shot,
    ], { stdio: "ignore" });
    console.log(`baked fir ${tree.name} from ${triangles} triangles`);
    return decodePng(await readFile(shot));
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
}

// ── Cards ───────────────────────────────────────────────────────────────────

// Row runs of covered cells, as [col0, col1) per row.
function coverage(image, cols, rows) {
  const { width, height, channels, pixels } = image;
  const runs = [];
  for (let r = 0; r < rows; r += 1) {
    let start = -1;
    for (let c = 0; c <= cols; c += 1) {
      let on = false;
      if (c < cols) {
        let hit = 0, total = 0;
        for (let y = Math.floor((r * height) / rows); y < Math.floor(((r + 1) * height) / rows); y += 1) {
          for (let x = Math.floor((c * width) / cols); x < Math.floor(((c + 1) * width) / cols); x += 1) {
            const i = (y * width + x) * channels;
            hit += isTree(pixels[i], pixels[i + 1], pixels[i + 2]) ? 1 : 0;
            total += 1;
          }
        }
        on = hit / total >= 0.3;
      }
      if (on && start < 0) start = c;
      if (!on && start >= 0) {
        runs.push({ row: r, c0: start, c1: c });
        start = -1;
      }
    }
  }
  return runs;
}

// The capture with its background recoloured to the foliage's mean colour.
function cardTexture(image) {
  const { width, height, channels, pixels } = image;
  const rgb = Buffer.alloc(width * height * 3);
  const sum = [0, 0, 0];
  let n = 0;
  for (let i = 0; i < width * height; i += 1) {
    const [r, g, b] = [pixels[i * channels], pixels[i * channels + 1], pixels[i * channels + 2]];
    if (isTree(r, g, b)) {
      sum[0] += r; sum[1] += g; sum[2] += b; n += 1;
    }
  }
  const mean = sum.map((s) => Math.round(s / Math.max(n, 1)));
  for (let i = 0; i < width * height; i += 1) {
    const px = [pixels[i * channels], pixels[i * channels + 1], pixels[i * channels + 2]];
    const c = isTree(...px) ? px : mean;
    rgb.set(c, i * 3);
  }
  return encodeRgbPng(width, height, rgb);
}

// Crossed vertical planes through the Y axis, each carrying the covered runs.
function cardGlb(runs, card, height, png) {
  const H = height * 1.03;
  const W = H * (TEXTURE.width / TEXTURE.height);
  const pos = [], nor = [], uv = [], idx = [];
  for (let p = 0; p < card.planes; p += 1) {
    const angle = (Math.PI * p) / card.planes;
    const [ax, az] = [Math.cos(angle), -Math.sin(angle)];
    for (const { row, c0, c1 } of runs) {
      const base = pos.length / 3;
      const [u0, u1] = [c0 / card.cols, c1 / card.cols];
      const [v0, v1] = [row / card.rows, (row + 1) / card.rows];
      for (const [u, v] of [[u0, v1], [u1, v1], [u1, v0], [u0, v0]]) {
        // The bake looks down +Z, so screen-left is +X: u runs from +X to -X
        // for plane 0 to overlay the real tree rather than its mirror image.
        const s = (0.5 - u) * W;
        pos.push(ax * s, (1 - v) * H, az * s);
        // Out from the trunk and up, as if the crown were a solid cone: the
        // planes shade together, and no face is ever turned fully away.
        const out = (2 * s) / W;
        const n = [ax * out, 0.6, az * out];
        const len = Math.hypot(...n);
        nor.push(n[0] / len, n[1] / len, n[2] / len);
        uv.push(u, v);
      }
      idx.push(base, base + 1, base + 2, base, base + 2, base + 3);
    }
  }
  const json = {
    asset: { version: "2.0", generator: "night-terrain generate-impostors.mjs" },
    scene: 0,
    scenes: [{ nodes: [0] }],
    nodes: [{ mesh: 0 }],
    meshes: [{ primitives: [{ attributes: { POSITION: 0, NORMAL: 1, TEXCOORD_0: 2 }, indices: 3, material: 0 }] }],
    materials: [{
      pbrMetallicRoughness: { baseColorTexture: { index: 0 }, metallicFactor: 0, roughnessFactor: 1 },
      doubleSided: true,
    }],
    textures: [{ source: 0 }],
    images: [],
    accessors: [],
  };
  let bin = Buffer.alloc(0);
  let view;
  const accessor = (data, componentType, type, count, target, extra = {}) => {
    [bin, view] = appendView(json, bin, data, target);
    json.accessors.push({ bufferView: view, componentType, count, type, ...extra });
  };
  accessor(Buffer.from(new Float32Array(pos).buffer), 5126, "VEC3", pos.length / 3, 34962,
    { min: [-W / 2, 0, -W / 2], max: [W / 2, H, W / 2] });
  accessor(Buffer.from(new Float32Array(nor).buffer), 5126, "VEC3", nor.length / 3, 34962);
  accessor(Buffer.from(new Float32Array(uv).buffer), 5126, "VEC2", uv.length / 2, 34962);
  accessor(Buffer.from(new Uint32Array(idx).buffer), 5125, "SCALAR", idx.length, 34963);
  [bin, view] = appendView(json, bin, png);
  json.images.push({ bufferView: view, mimeType: "image/png" });
  return { glb: skinForLighting(writeGlb(json, bin)), triangles: idx.length / 3 };
}

for (const tree of TREES) {
  const image = await bake(tree);
  const png = cardTexture(image);
  for (const card of CARDS) {
    const { glb, triangles } = cardGlb(coverage(image, card.cols, card.rows), card, tree.height, png);
    const file = `impostor-fir-${tree.name}-${card.suffix}.glb`;
    await writeFile(path.join(here, file), glb);
    console.log(`wrote ${file}: ${triangles} triangles, ${(glb.length / 1024).toFixed(0)} KB`);
  }
}
