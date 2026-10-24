// Fetch a Poly Haven CC0 model's 1k glTF set and pack it into one self-contained
// .glb with gltfpack, optionally keeping a single scene root, dropping
// materials, and simplifying. Shared by `fetch-assets.mjs` (game LODs) and
// `examples/night-terrain/generate-impostors.mjs` (full-detail bakes).
//
// The runtime loads only .glb with embedded buffers and images, and Poly Haven
// ships multi-file .gltf — hence the repack. `-noq` keeps plain float vertex
// data: the loader does not read KHR_mesh_quantization.
import { mkdtemp, mkdir, writeFile, readFile, rename, rm } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import os from "node:os";
import path from "node:path";

const API = "https://api.polyhaven.com";
const GLTFPACK = "gltfpack@0.24.0";

async function download(url, dest) {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`HTTP ${response.status} from ${url}`);
  await writeFile(dest, Buffer.from(await response.arrayBuffer()));
}

/**
 * @param {object} spec
 * @param {string} spec.id        Poly Haven asset id, e.g. "rock_moss_set_01"
 * @param {string} spec.out       destination .glb path
 * @param {number} [spec.node]    keep only this node (a mesh-bearing scene root), moved to the origin
 * @param {string[]} [spec.dropMaterials]  primitives using these materials are removed
 * @param {number} [spec.simplify]  gltfpack -si ratio (omit to keep every triangle)
 * @returns {Promise<number>} the packed triangle count
 */
export async function fetchPolyHavenGlb({ id, out, node, dropMaterials = [], simplify }) {
  const response = await fetch(`${API}/files/${id}`);
  if (!response.ok) throw new Error(`HTTP ${response.status} listing ${id}`);
  const gltf = (await response.json()).gltf?.["1k"]?.gltf;
  if (!gltf) throw new Error(`${id}: no 1k glTF on Poly Haven`);
  const tmp = await mkdtemp(path.join(os.tmpdir(), `functor-${id}-`));
  try {
    const source = path.join(tmp, `${id}.gltf`);
    await download(gltf.url, source);
    for (const [rel, file] of Object.entries(gltf.include)) {
      await mkdir(path.dirname(path.join(tmp, rel)), { recursive: true });
      await download(file.url, path.join(tmp, rel));
    }
    const json = JSON.parse(await readFile(source, "utf8"));
    if (node !== undefined) {
      json.scenes[json.scene ?? 0].nodes = [node];
      json.nodes.forEach((n, i) => i === node || delete n.mesh);
      delete json.nodes[node].translation;
    }
    if (dropMaterials.length > 0) {
      const dropped = new Set(
        json.materials.flatMap((m, i) => (dropMaterials.includes(m.name) ? [i] : [])),
      );
      for (const mesh of json.meshes) {
        mesh.primitives = mesh.primitives.filter((p) => !dropped.has(p.material));
      }
    }
    // The runtime samples only the base colour; unreferenced normal/ORM maps
    // are dropped by gltfpack, cutting each file by most of its size.
    for (const material of json.materials ?? []) {
      delete material.normalTexture;
      delete material.occlusionTexture;
      delete material.emissiveTexture;
      if (material.pbrMetallicRoughness) delete material.pbrMetallicRoughness.metallicRoughnessTexture;
    }
    await writeFile(source, JSON.stringify(json));
    // Pack beside the source, then move into place: an interrupted run leaves
    // no partial `out` for the next fetch to mistake for a finished one.
    const packed = path.join(tmp, "packed.glb");
    const args = ["-y", GLTFPACK, "-i", source, "-o", packed, "-noq", "-v"];
    if (simplify !== undefined) args.push("-si", String(simplify));
    const report = execFileSync("npx", args, { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
    await rename(packed, out).catch(async () => {
      // Across filesystems (tmp → repo) rename fails; copy then swap in place.
      await writeFile(`${out}.partial`, await readFile(packed));
      await rename(`${out}.partial`, out);
    });
    const match = report.match(/^output: \d+ mesh primitives \((\d+) triangles/m);
    return match ? Number(match[1]) : NaN;
  } finally {
    await rm(tmp, { recursive: true, force: true });
  }
}
