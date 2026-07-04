// A dependency-free static server for site/dist (dev + e2e). The one thing a
// generic `python3 -m http.server` gets wrong is the wasm MIME type, which
// WebAssembly.instantiateStreaming requires.
//
//   node site/serve.mjs [--port 8123]

import { createReadStream } from "node:fs";
import { stat } from "node:fs/promises";
import { createServer } from "node:http";
import { extname, join, normalize, sep } from "node:path";
import { fileURLToPath } from "node:url";

const dist = fileURLToPath(new URL("dist", import.meta.url));
const portFlag = process.argv.indexOf("--port");
const port = portFlag > 0 ? Number(process.argv[portFlag + 1]) : 8123;

const MIME = {
  ".html": "text/html; charset=utf-8",
  ".js": "application/javascript",
  ".css": "text/css",
  ".wasm": "application/wasm",
  ".mle": "text/plain; charset=utf-8",
  ".png": "image/png",
  ".svg": "image/svg+xml",
};

createServer(async (req, res) => {
  const path = decodeURIComponent(new URL(req.url, "http://x").pathname);
  // normalize() collapses any ../ so the resolved path stays inside dist
  // (dist itself or below — a bare prefix check would admit dist-siblings).
  let file = normalize(join(dist, path));
  if (file !== dist && !file.startsWith(dist + sep)) {
    res.writeHead(403).end();
    return;
  }
  try {
    let info = await stat(file);
    if (info.isDirectory()) {
      file = join(file, "index.html");
      await stat(file);
    }
  } catch {
    res.writeHead(404).end("not found");
    return;
  }
  res.writeHead(200, { "Content-Type": MIME[extname(file)] ?? "application/octet-stream" });
  createReadStream(file).pipe(res);
}).listen(port, "127.0.0.1", () => {
  console.log(`serving site/dist at http://127.0.0.1:${port}`);
});
