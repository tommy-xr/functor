// Print a glTF's skinning contract: skins, joint palette, inverse binds,
// geometry, textures, and clips. With several files, also diff every skin's
// joint palette and inverse binds against the first file's first skin.
//
// Usage: node tools/character-presets/inspect.mjs <a.gltf|glb> [b ...]
import path from "node:path";
import { readSource } from "./read.mjs";

const files = process.argv.slice(2);

/** Width/height from a PNG header (every source and output image is PNG). */
const pngSize = (bytes) => {
  const v = new DataView(bytes.buffer, bytes.byteOffset);
  return [v.getUint32(16), v.getUint32(20)];
};

function describe(doc) {
  const root = doc.getRoot();
  const parentOf = new Map();
  for (const n of root.listNodes()) for (const c of n.listChildren()) parentOf.set(c, n);
  const skins = root.listSkins().map((skin) => {
    const joints = skin.listJoints();
    const ibm = skin.getInverseBindMatrices()?.getArray() ?? [];
    return {
      name: skin.getName(),
      joints: joints.map((j) => j.getName()),
      parents: joints.map((j) => parentOf.get(j)?.getName() ?? null),
      ibm: joints.map((_, i) => Array.from(ibm.slice(i * 16, i * 16 + 16))),
      skeleton: skin.getSkeleton()?.getName() ?? null,
    };
  });
  const meshes = root.listNodes().filter((n) => n.getMesh()).map((n) => {
    const prims = n.getMesh().listPrimitives();
    const tris = prims.reduce((s, p) => s + (p.getIndices()?.getCount() ?? 0) / 3, 0);
    const verts = prims.reduce((s, p) => s + p.getAttribute("POSITION").getCount(), 0);
    return {
      node: n.getName(),
      mesh: n.getMesh().getName(),
      skin: n.getSkin()?.getName() ?? null,
      prims: prims.length,
      tris,
      verts,
      materials: prims.map((p) => p.getMaterial()?.getName() ?? null),
    };
  });
  const textures = root.listTextures().filter((t) => t.getImage()?.byteLength).map((t) => ({
    name: t.getName() || t.getURI(),
    mime: t.getMimeType(),
    bytes: t.getImage()?.byteLength ?? 0,
    size: pngSize(t.getImage()),
  }));
  const animations = root.listAnimations().map((a) => {
    let duration = 0;
    for (const s of a.listSamplers()) {
      const input = s.getInput()?.getArray();
      if (input?.length) duration = Math.max(duration, input[input.length - 1]);
    }
    return { name: a.getName(), duration: +duration.toFixed(4), channels: a.listChannels().length };
  });
  return { skins, meshes, textures, animations, nodes: root.listNodes().length };
}

/** Max abs element difference between two 4x4 matrices. */
const maxDiff = (a, b) => Math.max(...a.map((v, i) => Math.abs(v - b[i])));

const results = [];
for (const f of files) results.push({ file: path.basename(f), ...describe((await readSource(f)).doc) });

{
  const ref = results.find((r) => r.skins.length)?.skins[0];
  for (const r of results) {
    console.log(`\n== ${r.file}: ${r.nodes} nodes, ${r.skins.length} skin(s), ${r.meshes.length} mesh node(s)`);
    for (const s of r.skins) {
      console.log(`  skin "${s.name}": ${s.joints.length} joints, skeleton root ${s.skeleton}`);
      if (ref && s !== ref) {
        const sameOrder = s.joints.length === ref.joints.length && s.joints.every((j, i) => j === ref.joints[i]);
        const missing = s.joints.filter((j) => !ref.joints.includes(j));
        const parentsMatch = s.joints.every((j, i) => {
          const k = ref.joints.indexOf(j);
          return k < 0 || ref.parents[k] === s.parents[i];
        });
        let worst = 0;
        s.joints.forEach((j, i) => {
          const k = ref.joints.indexOf(j);
          if (k >= 0) worst = Math.max(worst, maxDiff(s.ibm[i], ref.ibm[k]));
        });
        console.log(`    vs ref: same order=${sameOrder} not-in-ref=${missing.length ? missing.join(",") : "none"} parents match=${parentsMatch} max IBM diff=${worst.toExponential(2)}`);
      }
    }
    for (const m of r.meshes) console.log(`  mesh ${m.node} [${m.materials.join(",")}] skin=${m.skin} ${m.prims}p ${m.tris} tris ${m.verts} verts`);
    for (const t of r.textures) console.log(`  tex ${t.name} ${t.mime} ${t.size?.join("x")} ${(t.bytes / 1e6).toFixed(2)}MB`);
    if (r.animations.length) console.log(`  ${r.animations.length} clips: ${r.animations.map((a) => `${a.name}(${a.duration}s)`).join(" ")}`);
  }
  if (ref) console.log(`\nref joints (${ref.joints.length}): ${ref.joints.join(" ")}`);
}
