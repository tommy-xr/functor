// Fetch a Poly Haven CC0 model's 1k glTF set and pack it into one self-contained
// .glb with gltfpack, optionally keeping a single scene root, dropping
// materials, and simplifying. Shared by `fetch-assets.mjs` (game LODs) and
// `examples/night-terrain/generate-impostors.mjs` (full-detail bakes).
//
// The runtime loads only .glb with embedded buffers and images, and Poly Haven
// ships multi-file .gltf — hence the repack. `-noq` keeps plain float vertex
// data: the loader does not read KHR_mesh_quantization.
import { mkdtemp, mkdir, writeFile, readFile, rm } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import os from "node:os";
import path from "node:path";
import { skinForLighting } from "./glb.mjs";

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
 * @param {number} [spec.node]    keep only this scene root, moved to the origin
 * @param {string[]} [spec.dropMaterials]  primitives using these materials are removed
 * @param {number} [spec.simplify]  gltfpack -si ratio (omit to keep every triangle)
 * @param {boolean} [spec.aggressive]  gltfpack -sa: reach the ratio regardless of error
 * @param {boolean} [spec.lit]  add the identity skin that gets it lit (see `skinForLighting`)
 * @returns {Promise<number>} the packed triangle count
 */
export async function fetchPolyHavenGlb({ id, out, node, dropMaterials = [], simplify, aggressive, lit }) {
  const files = await (await fetch(`${API}/files/${id}`)).json();
  const gltf = files.gltf?.["1k"]?.gltf;
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
    await writeFile(source, JSON.stringify(json));
    const args = ["-y", GLTFPACK, "-i", source, "-o", out, "-noq", "-v"];
    if (simplify !== undefined) args.push("-si", String(simplify));
    if (aggressive) args.push("-sa");
    const report = execFileSync("npx", args, { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
    if (lit) await writeFile(out, skinForLighting(await readFile(out)));
    const match = report.match(/^output: \d+ mesh primitives \((\d+) triangles/m);
    return match ? Number(match[1]) : NaN;
  } finally {
    await rm(tmp, { recursive: true, force: true });
  }
}
