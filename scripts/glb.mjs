// Minimal .glb (binary glTF 2.0) writing for generated assets: build the JSON
// document, append each binary payload as a buffer view, then assemble.

const pad4 = (b, fill = 0) => Buffer.concat([b, Buffer.alloc((4 - (b.length % 4)) % 4, fill)]);

/** Append `data` to `bin` as a new buffer view; returns `[bin, viewIndex]`. */
export function appendView(json, bin, data, target) {
  const base = pad4(bin);
  const view = { buffer: 0, byteOffset: base.length, byteLength: data.length };
  if (target) view.target = target;
  json.bufferViews = json.bufferViews ?? [];
  json.bufferViews.push(view);
  return [Buffer.concat([base, data]), json.bufferViews.length - 1];
}

/** Assemble a .glb from a JSON document and its BIN chunk. */
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
