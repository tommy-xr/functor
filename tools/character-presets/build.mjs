// Build the Quaternius character presets: one .glb per body × LOD holding
// every piece as a named part (body regions, eyes, brows, hairstyles, each
// outfit's slots) on ONE skin, base-colour textures downscaled and deduped,
// and the idle/walk/run clips retargeted in. `Scene.modelParts` picks the
// loadout; ASSETS.md lists the parts.
//
// Usage: npm run build:characters -- --src <dir> [--out examples/outfits]
//   <dir> holds the free packs unzipped side by side (examples/outfits/ASSETS.md
//   lists them); every source file is checked against SOURCE_SHA256. Writes the
//   .glbs plus `presets.lock.json`, the hashes `npm run fetch:assets` verifies.
import { Document } from "@gltf-transform/core";
import { MeshoptSimplifier } from "meshoptimizer";
import { createHash } from "node:crypto";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { parseArgs } from "node:util";
import { decodePng, encodePng } from "../../scripts/png.mjs";
import { io, readSource } from "./read.mjs";

const UBC = "Universal Base Characters[Standard]/Base Characters/Godot - UE";
const HAIR_DIR = "Universal Base Characters[Standard]/Hairstyles/Rigged to Head Bone/glTF (Godot -Unreal)";
const OUTFITS_DIR = "Modular Character Outfits - Fantasy[Standard]/Exports/glTF (Godot-Unreal)/Outfits";
const UAL1 = "Universal Animation Library[Standard]/Unreal-Godot/UAL1_Standard.glb";

// Per body: the base body, its hairstyles (part name → source file; each is
// fitted to one head), and its outfits. The FIRST outfit's skin is canonical.
const BODIES = {
  male: {
    body: `${UBC}/Superhero_Male_FullBody.gltf`,
    hair: { hair_buzzed: "Hair_Buzzed", hair_simple_parted: "Hair_SimpleParted", hair_beard: "Hair_Beard" },
    outfits: { ranger: `${OUTFITS_DIR}/Male_Ranger.gltf`, peasant: `${OUTFITS_DIR}/Male_Peasant.gltf` },
  },
  female: {
    body: `${UBC}/Superhero_Female_FullBody.gltf`,
    hair: { hair_buzzed: "Hair_BuzzedFemale", hair_buns: "Hair_Buns", hair_long: "Hair_Long" },
    outfits: { ranger: `${OUTFITS_DIR}/Female_Ranger.gltf`, peasant: `${OUTFITS_DIR}/Female_Peasant.gltf` },
  },
};
// Body file node → part (`regions` splits the body mesh, below).
const BODY_NODES = { SuperHero_Male: "regions", Superhero_Female: "regions", Eyes: "eyes", Eyebrows: "eyebrows" };
// Outfit node `<Sex>_<Outfit>_<piece>` → slot; the part is `<outfit>_<slot>`.
// Bracers ride with the arms and belts with the body, so a slot is one toggle.
const SLOTS = {
  Head_Hood: "hood", Acc_Pauldron: "pauldrons", Acc_Pauldrons: "pauldrons",
  Body: "body", Body_Belt_1: "body", Body_Belt_2: "body",
  Arms: "arms", Arms_Bracer: "arms", Legs: "legs", Feet: "feet", Feet_Boots: "feet",
};
const SLOT_ORDER = ["hood", "pauldrons", "body", "arms", "legs", "feet"];
// Output clip name → UAL1 clip (the no-root-motion file).
const CLIPS = { Idle: "Idle_Loop", Walk: "Walk_Loop", Run: "Jog_Fwd_Loop" };
// Triangle ratio and max texture edge per LOD.
const LODS = [
  { ratio: 1, texture: 1024 },
  { ratio: 0.12, texture: 512 },
  { ratio: 0.03, texture: 256 },
];
// Body regions by skin weight. A triangle is `head` if any corner has at least
// HEAD_WEIGHT on the head joints (the cut sits just under every outfit's
// collar); otherwise it takes the region with the most weight over its corners.
// The pelvis (underwear) goes with the legs: trousers cover it, short tops don't.
// Shins are their own region: trousers stop mid-calf and boots start below the
// knee, so only the pair covers them.
const HEAD_JOINTS = new Set(["Head", "neck_01"]);
const HEAD_WEIGHT = 0.5;
// Legs triangles with at least this much spine weight (summed over corners) also draw with the torso.
const WAIST_OVERLAP = 0.3;
// Rebinding moves the male body up to 36 mm; more means a mismatched source.
const MAX_REBIND = 0.05;
const REGIONS = ["head", "torso", "arms", "legs", "shins", "feet"];
const regionOf = (joint) =>
  HEAD_JOINTS.has(joint) ? "head"
  : /^(pelvis|thigh_)/.test(joint) ? "legs"
  : /^calf_/.test(joint) ? "shins"
  : /^(foot|ball)_/.test(joint) ? "feet"
  : /^(upperarm|lowerarm|hand|thumb|index|middle|ring|pinky)_/.test(joint) ? "arms"
  : "torso"; // spine_*, clavicle_*

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
  "Universal Base Characters[Standard]/Hairstyles/Rigged to Head Bone/glTF (Godot -Unreal)/Hair_Buzzed.gltf": "6e709c691d4feccda9788df917bf9604e751f02da21cee2bc81bcc3d26b3b580",
  "Universal Base Characters[Standard]/Hairstyles/Rigged to Head Bone/glTF (Godot -Unreal)/Hair_Buzzed.bin": "5d4b5452ff879ed1d516694be57d1106c070c7133e747a8c9432b01ebf20e908",
  "Universal Base Characters[Standard]/Hairstyles/Rigged to Head Bone/glTF (Godot -Unreal)/Hair_SimpleParted.gltf": "fe0f354d7f95d83d38e15c60794dcc063fa4d60550706f3d7ba51491cdc07c56",
  "Universal Base Characters[Standard]/Hairstyles/Rigged to Head Bone/glTF (Godot -Unreal)/Hair_SimpleParted.bin": "ecd7c81959f3985d6254458a440daaaa18a8bfbb1dbe111c68a04a72374d54db",
  "Universal Base Characters[Standard]/Hairstyles/Rigged to Head Bone/glTF (Godot -Unreal)/Hair_Beard.gltf": "80013b9791ce12c65fbe693cf669b02ef5c38091c653bf13b9415d2d8380e0ce",
  "Universal Base Characters[Standard]/Hairstyles/Rigged to Head Bone/glTF (Godot -Unreal)/Hair_Beard.bin": "bf2f46ef5808665ef31082f787bc68c51ff063e81b76ca1a51ac86a9ae723be7",
  "Universal Base Characters[Standard]/Hairstyles/Rigged to Head Bone/glTF (Godot -Unreal)/Hair_BuzzedFemale.gltf": "096a7a4ea5bcadc588f4c1e95f04b3461f8a20f3643a0308cfd23e81e180ab32",
  "Universal Base Characters[Standard]/Hairstyles/Rigged to Head Bone/glTF (Godot -Unreal)/Hair_BuzzedFemale.bin": "48fcdac8751e85c1d08ac9ff25b5c4d649e5b5d450d37ad8743b9f111a3a7e04",
  "Universal Base Characters[Standard]/Hairstyles/Rigged to Head Bone/glTF (Godot -Unreal)/Hair_Buns.gltf": "cbdd6f509ec8f1021361b53da6fa1a077b95762ebc8bc2d8acae59bb0cb263c1",
  "Universal Base Characters[Standard]/Hairstyles/Rigged to Head Bone/glTF (Godot -Unreal)/Hair_Buns.bin": "811606cc84f0bfdefa2e6f1396f3d94c35138308a96d78a4f447b2e7dfc6ed6c",
  "Universal Base Characters[Standard]/Hairstyles/Rigged to Head Bone/glTF (Godot -Unreal)/Hair_Long.gltf": "27e2c63e8f98c79fb600268ffcc92b36c621b8653b5a7dcbadb08328f3f887a9",
  "Universal Base Characters[Standard]/Hairstyles/Rigged to Head Bone/glTF (Godot -Unreal)/Hair_Long.bin": "268fd531d2c2ac321783d36fcf106d15c5c76e8923eec2c8704d5fe9de953589",
  "Universal Base Characters[Standard]/Hairstyles/Rigged to Head Bone/glTF (Godot -Unreal)/T_Hair_1_BaseColor.png": "bc7aa863bd22ab0a995cd838cceb4d3a5186ee54ee2fb0108fad85d20c057e6e",
  "Universal Base Characters[Standard]/Hairstyles/Rigged to Head Bone/glTF (Godot -Unreal)/T_Hair_2_BaseColor.png": "f9f4f2fb3eeeefb0b6f0fdef38b0770fd6607640c1c680b8139dd76697e9f9b3",
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
 * the source's world-space rotation RELATIVE TO ITS REST pose (= bind pose in
 * every source), so differing rest orientations don't skew the pose. Every
 * joint gets a rotation channel; translations become target rest + source
 * delta (only the pelvis moves) and constant translations are dropped.
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
  for (const name of order) {
    if (!source.has(name)) throw new Error(`joint ${name} missing from the clip's rig`);
    const p = target.get(name).parent;
    if (p && order.indexOf(p) > order.indexOf(name)) throw new Error(`${name} precedes its parent`);
  }
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
    // The delta is in the parent's frame: only valid if both parents rest alike.
    const sp = source.get(name).parent, tp = target.get(name).parent;
    if (sp && Math.abs(qmul(qinv(source.get(sp).rest), target.get(tp).rest)[3]) < 0.99999) {
      throw new Error(`${name}: parent rest rotations differ; the translation delta needs rotating`);
    }
    const base = target.get(name).node.getTranslation();
    translations.set(name, tr.map((v, i) => base[i % 3] + v - rest[i % 3]));
  }
  for (const [key, out] of channels) {
    if (key.endsWith("/scale") && out.some((v) => Math.abs(v - 1) > 1e-5)) throw new Error(`${key} scales`);
  }
  return { times, rotations, translations };
}

// ---- geometry ----
/** Accessor values as floats; the sources store every attribute as plain float. */
function floats(accessor) {
  if (accessor.getNormalized()) throw new Error("normalized attributes are not supported");
  return Float32Array.from(accessor.getArray());
}

/** Skinned primitives of `doc` as plain arrays in the canonical joint order. */
function extract(doc, canonical) {
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
      if (prim.getMode() !== 4 || prim.getAttribute("JOINTS_1")) throw new Error(`${node.getName()}: unsupported primitive`);
      if (prim.getMaterial().getBaseColorTextureInfo()?.getTexCoord()) throw new Error("base colour must use TEXCOORD_0");
      const pos = floats(prim.getAttribute("POSITION"));
      const nrm = floats(prim.getAttribute("NORMAL"));
      const uv = floats(prim.getAttribute("TEXCOORD_0"));
      const j0 = prim.getAttribute("JOINTS_0").getArray();
      const w0 = floats(prim.getAttribute("WEIGHTS_0"));
      const joints = Uint8Array.from(j0, (j) => remap[j]);
      const material = prim.getMaterial();
      const image = material.getBaseColorTexture()?.getImage() ?? null;
      parts.push({
        node: node.getName(),
        material: material.getName(),
        image,
        factor: material.getBaseColorFactor(),
        doubleSided: material.getDoubleSided(),
        pos, nrm, uv, joints, weights: w0,
        indices: Uint32Array.from(prim.getIndices().getArray()),
      });
    }
  }
  return parts;
}

// ---- rebinding ----
/** Column-major 4x4 product a × b. */
const mul = (a, b) => {
  const out = new Array(16);
  for (let c = 0; c < 4; c++) {
    for (let r = 0; r < 4; r++) {
      out[c * 4 + r] = a[r] * b[c * 4] + a[4 + r] * b[c * 4 + 1] + a[8 + r] * b[c * 4 + 2] + a[12 + r] * b[c * 4 + 3];
    }
  }
  return out;
};
const ibms = (skin) => {
  const a = skin.getInverseBindMatrices().getArray();
  return skin.listJoints().map((_, i) => Array.from(a.slice(i * 16, i * 16 + 16)));
};

/**
 * Re-skin primitives bound to `from` onto `to` (same joint order): each vertex
 * is posed by linear blend skinning into `to`'s bind pose, so `to`'s inverse
 * binds reproduce it at rest. Vertices whose joints bind identically on both
 * skins are left untouched (the head, hair, every female piece). Returns the
 * largest displacement in metres.
 */
function rebind(prims, from, to) {
  if (from.listJoints().some((j, i) => j.getName() !== to.listJoints()[i]?.getName())) {
    throw new Error("rebind needs the same joint order on both skins");
  }
  const src = ibms(from);
  const dst = ibms(to);
  // Bind world matrices of `to`: the joints' rest pose must equal the bind.
  const world = to.listJoints().map((j, i) => {
    const w = j.getWorldMatrix();
    if (mul(w, dst[i]).some((v, k) => Math.abs(v - (k % 5 === 0 ? 1 : 0)) > 1e-4)) {
      throw new Error(`${j.getName()}: rest pose differs from the bind pose`);
    }
    return w;
  });
  const same = src.map((m, i) => m.every((v, k) => v === dst[i][k]));
  const xform = src.map((m, i) => (same[i] ? null : mul(world[i], m)));
  let moved = 0;
  for (const p of prims) {
    for (let v = 0; v < p.pos.length / 3; v++) {
      const ws = [0, 1, 2, 3].filter((k) => p.weights[v * 4 + k] > 0);
      if (ws.every((k) => same[p.joints[v * 4 + k]])) continue;
      const P = [0, 0, 0], N = [0, 0, 0];
      const [x, y, z] = p.pos.subarray(v * 3, v * 3 + 3);
      const [nx, ny, nz] = p.nrm.subarray(v * 3, v * 3 + 3);
      const total = ws.reduce((s, k) => s + p.weights[v * 4 + k], 0);
      for (const k of ws) {
        const w = p.weights[v * 4 + k] / total;
        const m = xform[p.joints[v * 4 + k]] ?? [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];
        for (let r = 0; r < 3; r++) {
          P[r] += w * (m[r] * x + m[4 + r] * y + m[8 + r] * z + m[12 + r]);
          N[r] += w * (m[r] * nx + m[4 + r] * ny + m[8 + r] * nz);
        }
      }
      moved = Math.max(moved, Math.hypot(P[0] - x, P[1] - y, P[2] - z));
      const len = Math.hypot(...N) || 1;
      p.pos.set(P, v * 3);
      p.nrm.set(N.map((n) => n / len), v * 3);
    }
  }
  return moved;
}

/** Split the body by skin weight into one group per region (REGIONS order). */
function splitRegions(g, canonical) {
  const lists = new Map(REGIONS.map((r) => [r, []]));
  const corner = (v) => {
    const w = new Map(REGIONS.map((r) => [r, 0]));
    for (let k = 0; k < 4; k++) {
      const r = regionOf(canonical[g.joints[v * 4 + k]]);
      w.set(r, w.get(r) + g.weights[v * 4 + k]);
    }
    return w;
  };
  for (let t = 0; t < g.indices.length; t += 3) {
    const tri = [g.indices[t], g.indices[t + 1], g.indices[t + 2]];
    const ws = tri.map(corner);
    let region = "head";
    const sum = (r) => ws.reduce((s, w) => s + w.get(r), 0);
    if (!ws.some((w) => w.get("head") >= HEAD_WEIGHT)) {
      let best = -1;
      for (const r of REGIONS) if (sum(r) > best) (best = sum(r)), (region = r);
    }
    lists.get(region).push(...tri);
    // Trousers start below the pelvis cut, so the torso also draws the waist
    // band (legs triangles with some spine weight). Tops reach below the cut,
    // so the reverse would poke through them.
    if (region === "legs" && sum("torso") >= WAIST_OVERLAP) lists.get("torso").push(...tri);
  }
  return REGIONS.map((r) => {
    if (!lists.get(r).length) throw new Error(`body region ${r} is empty`);
    return { name: `body_${r}`, groups: [compact({ ...g, indices: Uint32Array.from(lists.get(r)) })] };
  });
}

/** Primitives with equal keys draw identically (texture, factor, sidedness). */
const materialKey = (p) => `${p.image ? sha256(p.image) : "none"}|${p.factor.join(",")}|${p.doubleSided}`;

/** Concatenate parts that share a texture + factor: one draw per material. */
function groupByMaterial(parts) {
  const groups = new Map();
  for (const p of parts) {
    const key = materialKey(p);
    if (!groups.has(key)) groups.set(key, { ...p, list: [] });
    groups.get(key).list.push(p);
  }
  return [...groups.values()].map((g) => {
    const cat = (field) => {
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

/**
 * Merge vertices split only by their normal (hard edges), averaging the
 * normals, so the simplifier treats just the UV seams as seams. LOD1/2 shade
 * those edges smooth.
 */
function weld(g) {
  const key = new Map();
  const nrm = Float32Array.from(g.nrm);
  const remap = new Uint32Array(g.pos.length / 3);
  for (let v = 0; v < remap.length; v++) {
    const k = [...g.pos.subarray(v * 3, v * 3 + 3), ...g.uv.subarray(v * 2, v * 2 + 2),
      ...g.joints.subarray(v * 4, v * 4 + 4), ...g.weights.subarray(v * 4, v * 4 + 4)].join(",");
    const first = key.get(k);
    if (first === undefined) key.set(k, (remap[v] = v));
    else {
      remap[v] = first;
      for (let c = 0; c < 3; c++) nrm[first * 3 + c] += g.nrm[v * 3 + c];
    }
  }
  for (let v = 0; v < remap.length; v++) {
    const len = Math.hypot(nrm[v * 3], nrm[v * 3 + 1], nrm[v * 3 + 2]) || 1;
    for (let c = 0; c < 3; c++) nrm[v * 3 + c] /= len;
  }
  return compact({ ...g, nrm, indices: g.indices.map((i) => remap[i]) });
}

function simplify(g, ratio) {
  if (ratio >= 1) return g;
  const w = weld(g);
  const target = Math.max(3, Math.floor((w.indices.length / 3) * ratio) * 3);
  const n = w.pos.length / 3;
  const attrs = new Float32Array(n * 5);
  for (let v = 0; v < n; v++) attrs.set([...w.uv.subarray(v * 2, v * 2 + 2), ...w.nrm.subarray(v * 3, v * 3 + 3)], v * 5);
  // Permissive: the outfits are mostly UV seams, which would otherwise pin the
  // count far above target; the UV weight keeps collapses from smearing texture.
  const weights = [4, 4, 0.5, 0.5, 0.5];
  const [indices] = MeshoptSimplifier.simplifyWithAttributes(
    w.indices, w.pos, 3, attrs, 5, weights, null, target, 1, ["Permissive"]);
  return compact({ ...w, indices });
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
/** `parts`: [{ name, groups }] → one node + mesh per part, one primitive per group. */
function writePreset(skeletonDoc, parts, clips, lod) {
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

  const accessor = (type, array) => doc.createAccessor().setType(type).setArray(array).setBuffer(buffer);
  const textures = new Map();
  const materials = new Map();
  const material = (g) => {
    const key = materialKey(g);
    if (materials.has(key)) return materials.get(key);
    const m = doc.createMaterial(g.material)
      .setBaseColorFactor(g.factor).setDoubleSided(g.doubleSided).setMetallicFactor(0).setRoughnessFactor(1);
    if (g.image) {
      const t = downscale(g.image, lod.texture);
      if (!textures.has(t.png)) {
        textures.set(t.png, doc.createTexture(g.material).setImage(t.png).setMimeType("image/png"));
      }
      m.setBaseColorTexture(textures.get(t.png));
    }
    materials.set(key, m);
    return m;
  };
  const stats = { tris: 0, verts: 0, draws: 0, parts: [] };
  for (const part of parts) {
    const mesh = doc.createMesh(part.name);
    const ps = { name: part.name, tris: 0, draws: 0, materials: [] };
    for (const g of part.groups) {
      const n = g.pos.length / 3;
      mesh.addPrimitive(doc.createPrimitive()
        .setAttribute("POSITION", accessor("VEC3", g.pos))
        .setAttribute("NORMAL", accessor("VEC3", g.nrm))
        .setAttribute("TEXCOORD_0", accessor("VEC2", g.uv))
        .setAttribute("JOINTS_0", accessor("VEC4", g.joints))
        .setAttribute("WEIGHTS_0", accessor("VEC4", g.weights))
        .setIndices(accessor("SCALAR", n < 65536 ? Uint16Array.from(g.indices) : g.indices))
        .setMaterial(material(g)));
      ps.tris += g.indices.length / 3;
      ps.draws += 1;
      ps.materials.push(g.material);
      stats.verts += n;
    }
    scene.addChild(doc.createNode(part.name).setMesh(mesh).setSkin(skin));
    stats.tris += ps.tris;
    stats.draws += ps.draws;
    stats.parts.push(ps);
  }
  for (const [name, clip] of Object.entries(clips)) {
    const anim = doc.createAnimation(name);
    const input = accessor("SCALAR", Float32Array.from(clip.times));
    const add = (node, pathName, type, values) => {
      const sampler = doc.createAnimationSampler().setInput(input).setOutput(accessor(type, values)).setInterpolation("LINEAR");
      anim.addSampler(sampler).addChannel(doc.createAnimationChannel().setTargetNode(clones.get(node)).setTargetPath(pathName).setSampler(sampler));
    };
    for (const [node, values] of clip.rotations) add(node, "rotation", "VEC4", values);
    for (const [node, values] of clip.translations) add(node, "translation", "VEC3", values);
  }
  return { doc, stats };
}

await MeshoptSimplifier.ready;
const hairFile = (name) => `${HAIR_DIR}/${name}.gltf`;
const sources = await loadAll([
  UAL1,
  ...Object.values(BODIES).flatMap((b) => [b.body, ...Object.values(b.hair).map(hairFile), ...Object.values(b.outfits)]),
]);
const skinOf = (doc) => {
  if (doc.getRoot().listSkins().length !== 1) throw new Error("expected one skin per source");
  return doc.getRoot().listSkins()[0];
};
const ual = sources.get(UAL1);
const ualSkeleton = skeleton(skinOf(ual));
await mkdir(args.out, { recursive: true });
const report = [];
const capital = (s) => s[0].toUpperCase() + s.slice(1);
for (const [bodyName, spec] of Object.entries(BODIES)) {
  // The canonical skin is the outfits' (male outfits and hair share one rig;
  // the male body binds its limbs 17–24 mm apart and is rebound onto it).
  const rig = sources.get(Object.values(spec.outfits)[0]);
  const rigSkin = skinOf(rig);
  const canonical = rigSkin.listJoints().map((j) => j.getName());
  const load = (file) => {
    const doc = sources.get(file);
    const prims = extract(doc, canonical);
    const moved = rebind(prims, skinOf(doc), rigSkin);
    if (moved > MAX_REBIND) throw new Error(`${file}: rebinding moves vertices ${moved} m`);
    if (moved) console.error(`${bodyName}: ${path.basename(file)} rebound, max ${(moved * 1000).toFixed(1)} mm`);
    return prims;
  };
  const named = new Map(); // part name → prims, in output order
  const add = (name, prim) => named.set(name, [...(named.get(name) ?? []), prim]);
  let bodyMesh = null;
  for (const p of load(spec.body)) {
    const part = BODY_NODES[p.node];
    if (!part) throw new Error(`unexpected body node ${p.node}`);
    if (part === "regions") {
      if (bodyMesh) throw new Error("expected one body primitive");
      bodyMesh = p;
    }
    else add(part, p);
  }
  for (const [part, file] of Object.entries(spec.hair)) for (const p of load(hairFile(file))) add(part, p);
  for (const [outfit, file] of Object.entries(spec.outfits)) {
    const prefix = `${capital(bodyName)}_${capital(outfit)}_`;
    const pieces = load(file).map((p) => {
      const slot = p.node.startsWith(prefix) && SLOTS[p.node.slice(prefix.length)];
      if (!slot) throw new Error(`unexpected outfit node ${p.node}`);
      return { slot, p };
    });
    for (const slot of SLOT_ORDER) for (const x of pieces) if (x.slot === slot) add(`${outfit}_${slot}`, x.p);
  }
  if (!bodyMesh) throw new Error("body mesh missing");
  const body = compact(bodyMesh);
  const others = [...named].map(([name, prims]) => ({ name, groups: groupByMaterial(prims) }));
  const target = skeleton(rigSkin);
  const clips = {};
  for (const [out, src] of Object.entries(CLIPS)) {
    const anim = ual.getRoot().listAnimations().find((a) => a.getName() === src);
    if (!anim) throw new Error(`clip ${src} not in UAL1`);
    clips[out] = retarget(anim, ualSkeleton, target);
  }
  for (const [i, lod] of LODS.entries()) {
    // The body simplifies whole, then splits, so its regions share every seam.
    const parts = [
      ...splitRegions(simplify(body, lod.ratio), canonical),
      ...others.map((p) => ({ name: p.name, groups: p.groups.map((g) => simplify(g, lod.ratio)) })),
    ];
    const { doc, stats } = writePreset(rig, parts, clips, lod);
    const file = `${bodyName}_lod${i}.glb`;
    const bytes = await io.writeBinary(doc);
    await writeFile(path.join(args.out, file), bytes);
    report.push({ file, body: bodyName, ...stats, bytes: bytes.length, sha256: sha256(bytes) });
  }
}
await writeFile(
  path.join(args.out, "presets.lock.json"),
  `${JSON.stringify(Object.fromEntries(report.map((r) => [r.file, r.sha256])), null, 2)}\n`,
);
console.log("| File | Parts | Triangles | Vertices | Primitives | File MB | SHA-256 |");
console.log("| --- | ---: | ---: | ---: | ---: | ---: | --- |");
for (const r of report) {
  console.log(`| \`${r.file}\` | ${r.parts.length} | ${r.tris} | ${r.verts} | ${r.draws} | ${(r.bytes / 2 ** 20).toFixed(2)} | \`${r.sha256}\` |`);
}
// Per-part triangles at LOD 0/1/2, one column per body.
const bodies = Object.keys(BODIES);
console.log(`\n| Part | Materials | ${bodies.map((b) => `${b} tris LOD0/1/2`).join(" | ")} |`);
console.log(`| --- | --- | ${bodies.map(() => "---:").join(" | ")} |`);
for (const name of new Set(report.flatMap((r) => r.parts.map((p) => p.name)))) {
  const of = (b) => report.filter((r) => r.body === b).map((r) => r.parts.find((p) => p.name === name));
  const cells = bodies.map((b) => (of(b)[0] ? of(b).map((p) => p.tris).join(" / ") : "–"));
  const mats = new Set(bodies.flatMap(of).filter(Boolean).map((p) => p.materials.join(" + ")));
  console.log(`| \`${name}\` | ${[...mats].join("; ")} | ${cells.join(" | ")} |`);
}
