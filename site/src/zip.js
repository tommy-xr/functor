// Minimal STORE-only (uncompressed) ZIP writer — enough to download a Functor
// project (a handful of small .fun text files) as a .zip, with no dependency.
// Deflate isn't worth the code for a few KB of source; a store-only archive
// unzips everywhere and drops straight into `functor -d <dir> build wasm`.

const CRC_TABLE = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();

const crc32 = (bytes) => {
  let c = 0xffffffff;
  for (let i = 0; i < bytes.length; i++) c = CRC_TABLE[(c ^ bytes[i]) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
};

const u16 = (v) => new Uint8Array([v & 0xff, (v >>> 8) & 0xff]);
const u32 = (v) =>
  new Uint8Array([v & 0xff, (v >>> 8) & 0xff, (v >>> 16) & 0xff, (v >>> 24) & 0xff]);

const concat = (arrs) => {
  let len = 0;
  for (const a of arrs) len += a.length;
  const out = new Uint8Array(len);
  let o = 0;
  for (const a of arrs) {
    out.set(a, o);
    o += a.length;
  }
  return out;
};

// files: [{ path, source }] (source a string). Returns a Blob (application/zip).
// Timestamps are pinned (1980-01-01) so identical inputs yield identical bytes.
export function zipFiles(files) {
  const enc = new TextEncoder();
  const parts = [];
  const central = [];
  let offset = 0;

  for (const f of files) {
    const name = enc.encode(f.path);
    const data = enc.encode(f.source);
    const crc = crc32(data);
    const local = concat([
      u32(0x04034b50), u16(20), u16(0), u16(0), // sig, version, flags, method=store
      u16(0), u16(0x21), // mod time 0, mod date 1980-01-01
      u32(crc), u32(data.length), u32(data.length),
      u16(name.length), u16(0),
      name,
    ]);
    const localOffset = offset;
    parts.push(local, data);
    offset += local.length + data.length;
    central.push(
      concat([
        u32(0x02014b50), u16(20), u16(20), u16(0), u16(0), // sig, made-by, need, flags, method
        u16(0), u16(0x21),
        u32(crc), u32(data.length), u32(data.length),
        u16(name.length), u16(0), u16(0), u16(0), u16(0),
        u32(0), u32(localOffset),
        name,
      ])
    );
  }

  const centralStart = offset;
  let centralSize = 0;
  for (const c of central) centralSize += c.length;
  const end = concat([
    u32(0x06054b50), u16(0), u16(0),
    u16(files.length), u16(files.length),
    u32(centralSize), u32(centralStart), u16(0),
  ]);
  return new Blob([...parts, ...central, end], { type: "application/zip" });
}
