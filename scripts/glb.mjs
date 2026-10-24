// Minimal .glb (binary glTF 2.0) read/write, plus the one-joint skin that gets a
// static model LIT. Shared by `polyhaven-gltf.mjs` and the night-terrain
// impostor baker.

const pad4 = (b, fill = 0) => Buffer.concat([b, Buffer.alloc((4 - (b.length % 4)) % 4, fill)]);

/** Split a .glb into its JSON document and BIN chunk. */
export function readGlb(glb) {
  const jsonLength = glb.readUInt32LE(12);
  const json = JSON.parse(glb.toString("utf8", 20, 20 + jsonLength));
  const binLength = glb.readUInt32LE(20 + jsonLength);
  const bin = glb.subarray(28 + jsonLength, 28 + jsonLength + binLength);
  return { json, bin };
}

/** Assemble a .glb from a JSON document and BIN chunk. */
export function writeGlb(json, bin) {
  const body = pad4(bin);
  json.buffers = [{ byteLength: body.length }];
  const jsonChunk = pad4(Buffer.from(JSON.stringify(json)), 0x20);
  const chunk = (type, data) => {
    const head = Buffer.alloc(8);
    head.writeUInt32LE(data.length, 0);
    head.write(type, 4, "ascii");
    return Buffer.concat([head, data]);
  };
  const header = Buffer.alloc(12);
  header.write("glTF", 0, "ascii");
  header.writeUInt32LE(2, 4);
  header.writeUInt32LE(12 + 8 + jsonChunk.length + 8 + body.length, 8);
  return Buffer.concat([header, chunk("JSON", jsonChunk), chunk("BIN\0", body)]);
}

/** Append `data` to `bin` as a new buffer view; returns `[bin, viewIndex]`. */
export function appendView(json, bin, data, target) {
  const base = pad4(bin);
  const view = { buffer: 0, byteOffset: base.length, byteLength: data.length };
  if (target) view.target = target;
  json.bufferViews = json.bufferViews ?? [];
  json.bufferViews.push(view);
  return [Buffer.concat([base, data]), json.bufferViews.length - 1];
}

/**
 * Bind every mesh to a single identity joint.
 *
 * The runtime draws static glTF models UNLIT and only skinned ones through
 * the lit shading path, so a night scene's flashlight and moon would pass
 * straight over a static rock. An identity skin changes nothing about the
 * shape — every vertex follows one joint that never moves — but routes the
 * model through lit shading, instanced or not. Remove once static models are
 * lit by the runtime.
 */
export function skinForLighting(glb) {
  let { json, bin } = readGlb(glb);
  let view;
  const identity = new Float32Array([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]);
  [bin, view] = appendView(json, bin, Buffer.from(identity.buffer));
  json.accessors.push({ bufferView: view, componentType: 5126, count: 1, type: "MAT4" });
  const inverseBindMatrices = json.accessors.length - 1;
  json.nodes.push({ name: "lighting-joint" });
  const joint = json.nodes.length - 1;
  json.skins = [{ joints: [joint], inverseBindMatrices, skeleton: joint }];
  json.scenes[json.scene ?? 0].nodes.push(joint);
  for (const node of json.nodes) if (node.mesh !== undefined) node.skin = 0;
  for (const mesh of json.meshes) {
    for (const primitive of mesh.primitives) {
      const count = json.accessors[primitive.attributes.POSITION].count;
      const weights = new Float32Array(count * 4);
      for (let i = 0; i < count; i += 1) weights[i * 4] = 1;
      [bin, view] = appendView(json, bin, Buffer.alloc(count * 4), 34962); // all joint 0
      json.accessors.push({ bufferView: view, componentType: 5121, count, type: "VEC4" });
      primitive.attributes.JOINTS_0 = json.accessors.length - 1;
      [bin, view] = appendView(json, bin, Buffer.from(weights.buffer), 34962);
      json.accessors.push({ bufferView: view, componentType: 5126, count, type: "VEC4" });
      primitive.attributes.WEIGHTS_0 = json.accessors.length - 1;
    }
  }
  return writeGlb(json, bin);
}
