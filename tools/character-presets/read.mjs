// Load a source .gltf/.glb for the preset pipeline. The runtime samples only
// base colour, so other texture slots are dropped BEFORE reading — the UBC
// .gltf files reference normal maps that aren't in the archive
// (`T_Eye_Normal_png.png`), which would otherwise fail the read.
import { NodeIO } from "@gltf-transform/core";
import { readFile } from "node:fs/promises";
import path from "node:path";

export const io = new NodeIO();

/** Returns the document and every file it was read from (for the input lock). */
export async function readSource(file) {
  if (file.endsWith(".glb")) return { doc: await io.read(file), files: [file] };
  const json = JSON.parse(await readFile(file, "utf8"));
  const files = [file];
  const used = new Set();
  for (const m of json.materials ?? []) {
    delete m.normalTexture;
    delete m.occlusionTexture;
    delete m.emissiveTexture;
    const pbr = m.pbrMetallicRoughness;
    if (pbr) delete pbr.metallicRoughnessTexture;
    if (pbr?.baseColorTexture) used.add(json.textures[pbr.baseColorTexture.index].source);
  }
  // Unused images become empty placeholders, never read from disk.
  const resources = {};
  const dir = path.dirname(file);
  const load = async (uri) => {
    files.push(path.join(dir, uri));
    resources[uri] = await readFile(path.join(dir, uri));
  };
  for (const b of json.buffers ?? []) await load(b.uri);
  for (const [i, img] of (json.images ?? []).entries()) {
    if (used.has(i)) await load(img.uri);
    else resources[(img.uri = `unused-${i}.png`)] = new Uint8Array(0);
  }
  return { doc: await io.readJSON({ json, resources }), files };
}
