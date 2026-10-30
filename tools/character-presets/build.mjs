// Build the Quaternius character presets: one .glb per body × outfit × LOD,
// every part on ONE skin, covered body regions removed, base-colour textures
// downscaled and deduped, and the idle/walk/run clips retargeted in.
//
// Usage: node tools/character-presets/build.mjs --src <dir> [--out examples/outfits]
//   <dir> holds the three free packs unzipped side by side (see README.md).
//   Every source file is checked against SOURCE_SHA256 before use.
import { Document } from "@gltf-transform/core";
import { MeshoptSimplifier } from "meshoptimizer";
import { createHash } from "node:crypto";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { parseArgs } from "node:util";
import { decodePng, encodePng } from "../../scripts/png.mjs";
import { io, readSource } from "./read.mjs";

const UBC = "Universal Base Characters[Standard]/Base Characters/Godot - UE";
const OUTFITS_DIR = "Modular Character Outfits - Fantasy[Standard]/Exports/glTF (Godot-Unreal)/Outfits";
const UAL1 = "Universal Animation Library[Standard]/Unreal-Godot/UAL1_Standard.glb";

const BODIES = {
  male: `${UBC}/Superhero_Male_FullBody.gltf`,
  female: `${UBC}/Superhero_Female_FullBody.gltf`,
};
// `base` is the body alone; an outfit keeps only the body's head (the pack's
// readme: "only the head of the model is required").
const OUTFITS = {
  base: null,
  ranger: { male: `${OUTFITS_DIR}/Male_Ranger.gltf`, female: `${OUTFITS_DIR}/Female_Ranger.gltf` },
  peasant: { male: `${OUTFITS_DIR}/Male_Peasant.gltf`, female: `${OUTFITS_DIR}/Female_Peasant.gltf` },
};
// Output clip name → UAL1 clip (the no-root-motion file).
const CLIPS = { Idle: "Idle_Loop", Walk: "Walk_Loop", Run: "Jog_Fwd_Loop" };
// Triangle ratio and max texture edge per LOD.
const LODS = [
  { ratio: 1, texture: 1024 },
  { ratio: 0.12, texture: 512 },
  { ratio: 0.03, texture: 256 },
];
// A body triangle survives under an outfit if any corner has at least this
// much weight on the head joints: the cut sits just under the outfit's collar.
const HEAD_JOINTS = new Set(["Head", "neck_01"]);
const HEAD_WEIGHT = 0.5;

// Pinned inputs (archive SHA-256s are in examples/outfits/ASSETS.md).
const SOURCE_SHA256 = {
  "Universal Animation Library[Standard]/Unreal-Godot/UAL1_Standard.glb": "69591853d817488edaa8fd9bf8fc1d821eaeaf789f8627b3cd23b41c4ed67997",
  "Universal Base Characters[Standard]/Base Characters/Godot - UE/Superhero_Male_FullBody.gltf": "e7fcea214ecf8855afbf910b50de6f9c7d1decfb71ca28bad8a4481452dafeb4",
  "Universal Base Characters[Standard]/Base Characters/Godot - UE/Superhero_Male_FullBody.bin": "459003f9745853ae562a85506a2b94dd56515c1f37728f9fa3d2ce1a3e4cd92f",
  "Universal Base Characters[Standard]/Base Characters/Godot - UE/T_Hair_1_BaseColor.png": "bc7aa863bd22ab0a995cd838cceb4d3a5186ee54ee2fb0108fad85d20c057e6e",
  "Universal Base Characters[Standard]/Base Characters/Godot - UE/T_Eye_Brown.png": "d08e3356a83211bc6ca21fe3a8e39f4b5c1a3b8f85457fc2c0fb57be09935025",
  "Universal Base Characters[Standard]/Base Characters/Godot - UE/T_Superhero_Male_Dark.png": "df680cbc1967a61e85998bfafb8f7bc0a36dd604bb3266486c4b06b1a184bb45",
  "Universal Base Characters[Standard]/Base Characters/Godot - UE/Superhero_Female_FullBody.gltf": "adedf28000a0716f689b009a70314506fc62f827498f77ba852acb5610f3f3f4",
  "Universal Base Characters[Standard]/Base Characters/Godot - UE/Superhero_Female_FullBody.bin": "3a8220a485b33d05d879115a50697728b45a151781106033afb8b8c243fca208",
  "Universal Base Characters[Standard]/Base Characters/Godot - UE/T_Hair_2_BaseColor.png": "f9f4f2fb3eeeefb0b6f0fdef38b0770fd6607640c1c680b8139dd76697e9f9b3",
  "Universal Base Characters[Standard]/Base Characters/Godot - UE/T_Superhero_Female_Dark_BaseColor.png": "d366843ee7d9e9f37ec87a0e1c98c7f2e45573c08eeb80a51a6d9344e68cf288",
  "Modular Character Outfits - Fantasy[Standard]/Exports/glTF (Godot-Unreal)/Outfits/Male_Ranger.gltf": "e1033a212332d84d98c19bc4927c621635478d0aa15d13c79da68d3ba9e0196f",
  "Modular Character Outfits - Fantasy[Standard]/Exports/glTF (Godot-Unreal)/Outfits/Male_Ranger.bin": "5bbb0d42cb324c6c27ce00417e68a876d92978f07d161f83590f0d7c0570eff6",
  "Modular Character Outfits - Fantasy[Standard]/Exports/glTF (Godot-Unreal)/Outfits/T_Ranger_BaseColor.png": "846ef2170b04ee584ad83bb8a1e86dab57a9a43b1193204505912c274d949925",
  "Modular Character Outfits - Fantasy[Standard]/Exports/glTF (Godot-Unreal)/Outfits/T_Regular_Male_Dark_BaseColor.png": "b4dcc0f4139aab548d7c070c4655bd2080ef16caeaaabee6c4aeb693c524e802",
  "Modular Character Outfits - Fantasy[Standard]/Exports/glTF (Godot-Unreal)/Outfits/Female_Ranger.gltf": "9b98b2ae6af3b4a1ec22bf8928b939e0751f08b25b636de82bfa7364fb34d3e1",
  "Modular Character Outfits - Fantasy[Standard]/Exports/glTF (Godot-Unreal)/Outfits/Female_Ranger.bin": "d015964ba0a26eee9c9d2ea6f07aa660bba88aa536cf8bb6b67944b9e97c46df",
  "Modular Character Outfits - Fantasy[Standard]/Exports/glTF (Godot-Unreal)/Outfits/T_Regular_Female_Dark_BaseColor.png": "3b6ab973f9f3d6f8961226a0dbd99cf6d1b72df3795a8a5c1620a55754c161c2",
  "Modular Character Outfits - Fantasy[Standard]/Exports/glTF (Godot-Unreal)/Outfits/Male_Peasant.gltf": "8f5a6cf6c1cf96e0a0f8e1a861b4245d42b7f88d44e5076da3d48f3fc52f4e76",
  "Modular Character Outfits - Fantasy[Standard]/Exports/glTF (Godot-Unreal)/Outfits/Male_Peasant.bin": "e46bde795f7b69b351d322d7a5628e07131cfb48ed0afe366293f1b3f530433c",
  "Modular Character Outfits - Fantasy[Standard]/Exports/glTF (Godot-Unreal)/Outfits/T_Peasant_BaseColor.png": "a43635628d95571cd62fd4db0cc3a6ea32778b2bc02f242dda224c89d07f1ade",
  "Modular Character Outfits - Fantasy[Standard]/Exports/glTF (Godot-Unreal)/Outfits/Female_Peasant.gltf": "43ba98058b933cae9acbde0f3ef8c3dda4ba9f69f338606f9d87666d9efffb31",
  "Modular Character Outfits - Fantasy[Standard]/Exports/glTF (Godot-Unreal)/Outfits/Female_Peasant.bin": "40eccaa2bd545c893142ef7dca0866ea8f20df3a86c67c5615024bc4fe546e83",
};

const { values: args } = parseArgs({
  options: { src: { type: "string" }, out: { type: "string", default: "examples/outfits" } },
});
if (!args.src) throw new Error("usage: build.mjs --src <dir with the unzipped packs> [--out dir]");

const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");

/** Read every source, checking each file it touches against SOURCE_SHA256. */
async function loadAll(rels) {
  const docs = new Map();
  const mismatches = [];
  for (const rel of rels) {
    const { doc, files } = await readSource(path.join(args.src, rel));
    for (const f of files) {
      const key = path.relative(args.src, f);
      const got = sha256(await readFile(f));
      if (SOURCE_SHA256[key] !== got) mismatches.push(`  ${JSON.stringify(key)}: "${got}",`);
    }
    docs.set(rel, doc);
  }
  if (mismatches.length) throw new Error(`unpinned or changed sources:\n${mismatches.join("\n")}`);
  return docs;
}

// ---- quaternions ([x, y, z, w]) ----
const qmul = ([ax, ay, az, aw], [bx, by, bz, bw]) => [
  aw * bx + ax * bw + ay * bz - az * by,
  aw * by - ax * bz + ay * bw + az * bx,
  aw * bz + ax * by - ay * bx + az * bw,
  aw * bw - ax * bx - ay * by - az * bz,
];
const qinv = ([x, y, z, w]) => [-x, -y, -z, w];

/** Joint name → { node, parent name, rest-pose global rotation } for a skin. */
function skeleton(skin) {
  const joints = new Map();
  const parentOf = new Map();
  const all = skin.listJoints();
  for (const j of all) for (const c of j.listChildren()) parentOf.set(c, j);
  const globalRot = (node) => {
    let q = node.getRotation();
    for (let p = node.getParentNode(); p; p = p.getParentNode()) q = qmul(p.getRotation(), q);
    return q;
  };
  for (const j of all) {
    const s = j.getScale();
    if (s.some((v) => Math.abs(v - 1) > 1e-4)) throw new Error(`joint ${j.getName()} is scaled`);
    joints.set(j.getName(), { node: j, parent: parentOf.get(j)?.getName() ?? null, rest: globalRot(j) });
  }
  return joints;
}

/**
 * Retarget a clip onto `target` (both rigs share joint names): each joint keeps
 * the source's world-space rotation RELATIVE TO ITS BIND pose, so differing
 * rest orientations don't skew the pose. Translations become target rest +
 * source delta (only the pelvis moves); constant channels are dropped.
 */
function retarget(anim, source, target) {
  const channels = new Map();
  let times = null;
  for (const c of anim.listChannels()) {
    const input = c.getSampler().getInput().getArray();
    if (c.getSampler().getInterpolation() !== "LINEAR") throw new Error("expected LINEAR keys");
    if (times && (times.length !== input.length || times.some((t, i) => t !== input[i]))) {
      throw new Error(`${anim.getName()}: channels have different key times`);
    }
    times = input;
    channels.set(`${c.getTargetNode().getName()}/${c.getTargetPath()}`, c.getSampler().getOutput().getArray());
  }
  const order = [...target.keys()]; // parents precede children in skin order
  const rotations = new Map(order.map((n) => [n, new Float32Array(times.length * 4)]));
  const translations = new Map();
  for (let k = 0; k < times.length; k++) {
    const srcGlobal = new Map();
    const tgtGlobal = new Map();
    for (const name of order) {
      const s = source.get(name);
      const t = target.get(name);
      const r = channels.get(`${name}/rotation`);
      const local = r ? Array.from(r.slice(k * 4, k * 4 + 4)) : s.node.getRotation();
      const sParent = s.parent ? srcGlobal.get(s.parent) : qmul(s.rest, qinv(s.node.getRotation()));
      const sg = qmul(sParent, local);
      srcGlobal.set(name, sg);
      const tg = qmul(qmul(sg, qinv(s.rest)), t.rest);
      tgtGlobal.set(name, tg);
      const tParent = t.parent ? tgtGlobal.get(t.parent) : qmul(t.rest, qinv(t.node.getRotation()));
      let q = qmul(qinv(tParent), tg);
      const out = rotations.get(name);
      // Keep consecutive keys in one hemisphere so LINEAR keys don't spin.
      if (k > 0 && q[0] * out[k * 4 - 4] + q[1] * out[k * 4 - 3] + q[2] * out[k * 4 - 2] + q[3] * out[k * 4 - 1] < 0) {
        q = q.map((v) => -v);
      }
      const len = Math.hypot(...q);
      out.set(q.map((v) => v / len), k * 4);
    }
  }
  for (const name of order) {
    const tr = channels.get(`${name}/translation`);
    if (!tr) continue;
    const rest = source.get(name).node.getTranslation();
    const moves = tr.some((v, i) => Math.abs(v - rest[i % 3]) > 1e-5);
    if (!moves) continue;
    const base = target.get(name).node.getTranslation();
    translations.set(name, tr.map((v, i) => base[i % 3] + v - rest[i % 3]));
  }
  for (const [key, out] of channels) {
    if (key.endsWith("/scale") && out.some((v) => Math.abs(v - 1) > 1e-5)) throw new Error(`${key} scales`);
  }
  return { times, rotations, translations };
}

// ---- geometry ----
/** Accessor values as floats, undoing glTF normalized-integer encoding. */
function floats(accessor) {
  const a = accessor.getArray();
  if (!accessor.getNormalized()) return Float32Array.from(a);
  const max = { 1: 127, 2: 32767 }[a.BYTES_PER_ELEMENT] ?? 1;
  const unsigned = a instanceof Uint8Array || a instanceof Uint16Array;
  const div = unsigned ? 2 ** (8 * a.BYTES_PER_ELEMENT) - 1 : max;
  return Float32Array.from(a, (v) => Math.max(v / div, -1));
}

/**
 * Skinned primitives of `doc` as plain arrays in the canonical joint order,
 * keeping triangles that pass `keepTri(cornerHeadWeights)`.
 */
function extract(doc, canonical, keepTri = () => true) {
  const parts = [];
  for (const node of doc.getRoot().listNodes()) {
    const mesh = node.getMesh();
    if (!mesh) continue;
    const skin = node.getSkin();
    if (!skin) throw new Error(`${node.getName()} is not skinned`);
    const names = skin.listJoints().map((j) => j.getName());
    const remap = names.map((n) => {
      const i = canonical.indexOf(n);
      if (i < 0) throw new Error(`joint ${n} missing from the canonical skin`);
      return i;
    });
    for (const prim of mesh.listPrimitives()) {
      const pos = floats(prim.getAttribute("POSITION"));
      const nrm = floats(prim.getAttribute("NORMAL"));
      const uv = floats(prim.getAttribute("TEXCOORD_0"));
      const j0 = prim.getAttribute("JOINTS_0").getArray();
      const w0 = floats(prim.getAttribute("WEIGHTS_0"));
      const joints = Uint8Array.from(j0, (j) => remap[j]);
      const head = new Float32Array(pos.length / 3);
      for (let v = 0; v < head.length; v++) {
        for (let k = 0; k < 4; k++) if (HEAD_JOINTS.has(names[j0[v * 4 + k]])) head[v] += w0[v * 4 + k];
      }
      const src = prim.getIndices().getArray();
      const kept = [];
      for (let t = 0; t < src.length; t += 3) {
        if (keepTri([head[src[t]], head[src[t + 1]], head[src[t + 2]]])) kept.push(src[t], src[t + 1], src[t + 2]);
      }
      const material = prim.getMaterial();
      const image = material.getBaseColorTexture()?.getImage() ?? null;
      parts.push({
        material: material.getName(),
        image,
        factor: material.getBaseColorFactor(),
        doubleSided: material.getDoubleSided(),
        pos, nrm, uv, joints, weights: w0,
        indices: Uint32Array.from(kept),
      });
    }
  }
  return parts;
}

/** Concatenate parts that share a texture + factor: one draw per material. */
function groupByMaterial(parts) {
  const groups = new Map();
  for (const p of parts) {
    const key = `${p.image ? sha256(p.image) : "none"}|${p.factor.join(",")}|${p.doubleSided}`;
    if (!groups.has(key)) groups.set(key, { ...p, list: [] });
    groups.get(key).list.push(p);
  }
  return [...groups.values()].map((g) => {
    const cat = (field, n) => {
      const total = g.list.reduce((s, p) => s + p[field].length, 0);
      const out = new (g.list[0][field].constructor)(total);
      let o = 0;
      for (const p of g.list) out.set(p[field], o), (o += p[field].length);
      return out;
    };
    const indices = [];
    let base = 0;
    for (const p of g.list) {
      for (const i of p.indices) indices.push(i + base);
      base += p.pos.length / 3;
    }
    return compact({
      ...g, pos: cat("pos"), nrm: cat("nrm"), uv: cat("uv"),
      joints: cat("joints"), weights: cat("weights"), indices: Uint32Array.from(indices),
    });
  });
}

/** Drop vertices no triangle references (first-use order, so deterministic). */
function compact(g) {
  const map = new Int32Array(g.pos.length / 3).fill(-1);
  let n = 0;
  const indices = g.indices.map((i) => (map[i] < 0 ? (map[i] = n++) : map[i]));
  const pick = (arr, width) => {
    const out = new (arr.constructor)(n * width);
    map.forEach((m, i) => m >= 0 && out.set(arr.subarray(i * width, i * width + width), m * width));
    return out;
  };
  return {
    ...g, indices,
    pos: pick(g.pos, 3), nrm: pick(g.nrm, 3), uv: pick(g.uv, 2),
    joints: pick(g.joints, 4), weights: pick(g.weights, 4),
  };
}

function simplify(g, ratio) {
  if (ratio >= 1) return g;
  const target = Math.floor((g.indices.length / 3) * ratio) * 3;
  const [indices] = MeshoptSimplifier.simplify(g.indices, g.pos, 3, target, 1, ["Permissive"]);
  return compact({ ...g, indices });
}

// ---- textures ----
const textureCache = new Map();
/** Box-downscale a PNG to at most `edge` px; integer factors only. */
function downscale(bytes, edge) {
  const key = `${sha256(bytes)}@${edge}`;
  if (textureCache.has(key)) return textureCache.get(key);
  const { width, height, channels, pixels } = decodePng(Buffer.from(bytes));
  const f = Math.max(1, Math.ceil(Math.max(width, height) / edge));
  if (width % f || height % f) throw new Error(`${width}x${height} doesn't divide by ${f}`);
  const w = width / f, h = height / f;
  const out = Buffer.alloc(w * h * channels);
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      for (let c = 0; c < channels; c++) {
        let sum = 0;
        for (let dy = 0; dy < f; dy++) {
          for (let dx = 0; dx < f; dx++) sum += pixels[((y * f + dy) * width + x * f + dx) * channels + c];
        }
        out[(y * w + x) * channels + c] = Math.round(sum / (f * f));
      }
    }
  }
  const result = { png: encodePng(w, h, 8, channels === 4 ? 6 : 2, out), w, h };
  textureCache.set(key, result);
  return result;
}

// ---- output ----
function writePreset(skeletonDoc, groups, clips, lod) {
  const doc = new Document();
  const buffer = doc.createBuffer();
  const skinSrc = skeletonDoc.getRoot().listSkins()[0];
  const jointNames = skinSrc.listJoints().map((j) => j.getName());
  // Clone the skeleton: every skin joint plus the ancestors above it.
  const clones = new Map();
  const clone = (n) => {
    const c = doc.createNode(n.getName()).setTranslation(n.getTranslation()).setRotation(n.getRotation()).setScale(n.getScale());
    clones.set(n.getName(), c);
    for (const child of n.listChildren()) if (!child.getMesh()) c.addChild(clone(child));
    return c;
  };
  let top = skinSrc.listJoints()[0];
  while (top.getParentNode()) top = top.getParentNode();
  const scene = doc.createScene().addChild(clone(top));
  const skin = doc.createSkin("Armature")
    .setInverseBindMatrices(doc.createAccessor().setType("MAT4").setArray(Float32Array.from(skinSrc.getInverseBindMatrices().getArray())).setBuffer(buffer));
  for (const n of jointNames) skin.addJoint(clones.get(n));

  const accessor = (type, array, normalized = false) =>
    doc.createAccessor().setType(type).setArray(array).setNormalized(normalized).setBuffer(buffer);
  const textures = new Map();
  const stats = { tris: 0, verts: 0, draws: 0, textureBytes: 0 };
  for (const g of groups) {
    const material = doc.createMaterial(g.material)
      .setBaseColorFactor(g.factor).setDoubleSided(g.doubleSided).setMetallicFactor(0).setRoughnessFactor(1);
    if (g.image) {
      const key = sha256(g.image);
      if (!textures.has(key)) {
        const t = downscale(g.image, lod.texture);
        textures.set(key, doc.createTexture(g.material).setImage(t.png).setMimeType("image/png"));
      }
      material.setBaseColorTexture(textures.get(key));
      const t = downscale(g.image, lod.texture);
      // The loader uploads a texture per mesh: RGBA8 + mips ≈ 4/3 × w × h × 4.
      stats.textureBytes += Math.round((t.w * t.h * 4 * 4) / 3);
    }
    const n = g.pos.length / 3;
    const prim = doc.createPrimitive()
      .setAttribute("POSITION", accessor("VEC3", g.pos))
      .setAttribute("NORMAL", accessor("VEC3", g.nrm))
      .setAttribute("TEXCOORD_0", accessor("VEC2", g.uv))
      .setAttribute("JOINTS_0", accessor("VEC4", g.joints))
      .setAttribute("WEIGHTS_0", accessor("VEC4", g.weights))
      .setIndices(accessor("SCALAR", n < 65536 ? Uint16Array.from(g.indices) : g.indices))
      .setMaterial(material);
    const mesh = doc.createMesh(g.material).addPrimitive(prim);
    scene.addChild(doc.createNode(g.material).setMesh(mesh).setSkin(skin));
    stats.tris += g.indices.length / 3;
    stats.verts += n;
    stats.draws += 1;
  }
  for (const [name, clip] of Object.entries(clips)) {
    const anim = doc.createAnimation(name);
    const input = accessor("SCALAR", Float32Array.from(clip.times));
    const add = (node, pathName, type, values) => {
      const sampler = doc.createAnimationSampler().setInput(input).setOutput(accessor(type, values)).setInterpolation("LINEAR");
      anim.addSampler(sampler).addChannel(doc.createAnimationChannel().setTargetNode(clones.get(node)).setTargetPath(pathName).setSampler(sampler));
    };
    for (const [node, values] of clip.rotations) add(node, "rotation", "VEC4", values);
    for (const [node, values] of clip.translations) add(node, "translation", "VEC3", Float32Array.from(values));
  }
  return { doc, stats };
}

await MeshoptSimplifier.ready;
const sources = await loadAll([
  UAL1,
  ...Object.values(BODIES),
  ...Object.values(OUTFITS).flatMap((o) => (o ? Object.values(o) : [])),
]);
const ual = sources.get(UAL1);
const ualSkeleton = skeleton(ual.getRoot().listSkins()[0]);
await mkdir(args.out, { recursive: true });
const report = [];
for (const [bodyName, bodyFile] of Object.entries(BODIES)) {
  const body = sources.get(bodyFile);
  for (const [outfitName, outfitFiles] of Object.entries(OUTFITS)) {
    const outfit = outfitFiles ? sources.get(outfitFiles[bodyName]) : null;
    // The canonical skin is the outfit's: the head's joints (Head, neck,
    // spine, clavicles) bind identically on both rigs; the limbs don't.
    const rig = outfit ?? body;
    const rigSkin = rig.getRoot().listSkins()[0];
    if (rig.getRoot().listSkins().length !== 1) throw new Error("expected one skin per source");
    const canonical = rigSkin.listJoints().map((j) => j.getName());
    const parts = outfit
      ? [...extract(body, canonical, (w) => w.some((x) => x >= HEAD_WEIGHT)), ...extract(outfit, canonical)]
      : extract(body, canonical);
    const groups = groupByMaterial(parts);
    const target = skeleton(rigSkin);
    const clips = {};
    for (const [out, src] of Object.entries(CLIPS)) {
      const anim = ual.getRoot().listAnimations().find((a) => a.getName() === src);
      if (!anim) throw new Error(`clip ${src} not in UAL1`);
      clips[out] = retarget(anim, ualSkeleton, target);
    }
    for (const [i, lod] of LODS.entries()) {
      const { doc, stats } = writePreset(rig, groups.map((g) => simplify(g, lod.ratio)), clips, lod);
      const file = `${bodyName}_${outfitName}_lod${i}.glb`;
      const bytes = await io.writeBinary(doc);
      await writeFile(path.join(args.out, file), bytes);
      report.push({ file, ...stats, bytes: bytes.length, sha256: sha256(bytes) });
    }
  }
}
console.log("| File | Triangles | Vertices | Draws | Texture MB (GPU) | File MB | SHA-256 |");
console.log("| --- | ---: | ---: | ---: | ---: | ---: | --- |");
for (const r of report) {
  console.log(`| \`${r.file}\` | ${r.tris} | ${r.verts} | ${r.draws} | ${(r.textureBytes / 2 ** 20).toFixed(1)} | ${(r.bytes / 2 ** 20).toFixed(2)} | \`${r.sha256}\` |`);
}
