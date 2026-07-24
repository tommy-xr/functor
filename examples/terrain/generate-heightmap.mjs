// Deterministically generate the sample's 16-bit grayscale PNG using only
// Node built-ins. Re-run with:
//
//   node examples/terrain/generate-heightmap.mjs
//
// The checked-in 1024² map keeps the example download compact. For a shipping
// 4 km world, author a 4096² source (roughly one height sample per metre); the
// Terrain API and renderer do not change.
import { deflateSync } from "node:zlib";
import { writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const size = 1024;
const raw = Buffer.alloc((size * 2 + 1) * size);

const clamp = (value, lo, hi) => Math.max(lo, Math.min(hi, value));
const smoothstep = (edge0, edge1, value) => {
  const t = clamp((value - edge0) / (edge1 - edge0), 0, 1);
  return t * t * (3 - 2 * t);
};

for (let y = 0; y < size; y += 1) {
  const row = y * (size * 2 + 1);
  raw[row] = 0; // PNG "None" filter
  for (let x = 0; x < size; x += 1) {
    const nx = (x / (size - 1)) * 2 - 1;
    const nz = (y / (size - 1)) * 2 - 1;

    // Broad continental folds, a sharper mountain chain, and a river valley.
    // All frequencies are low enough to remain smooth at the compact sample
    // resolution while still showing the renderer's geometric LOD.
    const broad =
      Math.sin(nx * 5.2 + nz * 1.1) * 0.10 +
      Math.sin(nz * 7.0 - nx * 1.7) * 0.075 +
      Math.sin((nx + nz) * 13.0) * 0.035;
    const ridgeWave = Math.abs(Math.sin(nx * 8.0 - nz * 3.2));
    const ridgeMask = Math.exp(
      -Math.pow((nx + 0.18) * 1.3 - (nz - 0.08) * 0.48, 2) * 4.5,
    );
    const mountains = Math.pow(ridgeWave, 2.2) * ridgeMask * 0.43;
    const foothills =
      Math.abs(Math.sin(nx * 17.0 + nz * 11.0)) *
      Math.abs(Math.cos(nz * 9.0 - nx * 4.0)) *
      0.055;
    const riverCenter = -0.22 + Math.sin(nz * 4.0) * 0.12;
    const river = Math.exp(-Math.pow((nx - riverCenter) * 13.0, 2)) * 0.20;
    const edgeDistance = Math.max(Math.abs(nx), Math.abs(nz));
    const edgeShelf = smoothstep(0.72, 1.0, edgeDistance) * 0.12;

    const normalized = clamp(
      0.30 + broad + mountains + foothills - river - edgeShelf,
      0,
      1,
    );
    const sample = Math.round(normalized * 65535);
    raw.writeUInt16BE(sample, row + 1 + x * 2);
  }
}

const crcTable = new Uint32Array(256);
for (let n = 0; n < 256; n += 1) {
  let c = n;
  for (let k = 0; k < 8; k += 1) {
    c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  }
  crcTable[n] = c >>> 0;
}

const crc32 = (bytes) => {
  let crc = 0xffffffff;
  for (const byte of bytes) {
    crc = crcTable[(crc ^ byte) & 0xff] ^ (crc >>> 8);
  }
  return (crc ^ 0xffffffff) >>> 0;
};

const chunk = (name, data) => {
  const type = Buffer.from(name, "ascii");
  const body = Buffer.concat([type, data]);
  const result = Buffer.alloc(12 + data.length);
  result.writeUInt32BE(data.length, 0);
  body.copy(result, 4);
  result.writeUInt32BE(crc32(body), 8 + data.length);
  return result;
};

const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(size, 0);
ihdr.writeUInt32BE(size, 4);
ihdr[8] = 16; // bit depth
ihdr[9] = 0; // grayscale
ihdr[10] = 0; // compression
ihdr[11] = 0; // filter
ihdr[12] = 0; // no interlace

const png = Buffer.concat([
  Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
  chunk("IHDR", ihdr),
  chunk("IDAT", deflateSync(raw, { level: 9 })),
  chunk("IEND", Buffer.alloc(0)),
]);
writeFileSync(fileURLToPath(new URL("heightmap.png", import.meta.url)), png);
console.log(`wrote ${size}x${size} 16-bit heightmap (${png.length} bytes)`);
