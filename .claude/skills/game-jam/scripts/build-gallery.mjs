#!/usr/bin/env node
// Build a playable web gallery from a set of Functor projects.
//
//   node build-gallery.mjs platformer=path/to/examples/platformer racer=... --out /tmp/gallery --serve 8321
//   node build-gallery.mjs --manifest entries.json --out /tmp/gallery
//
// Manifest: [{ "name": "racer", "dir": "…/examples/racer", "title"?, "description"?,
//              "controls"?, "scores"?, "thumb"? }]
// Anything not in the manifest is read from a `// gallery:` header comment in the
// project's game.fun — `// gallery: <one-line description>` and `// gallery-controls: …`.
// Thumbnail = newest PNG in the project's `.captures/`, omitted if there is none.

import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

function die(msg) {
  console.error(`error: ${msg}`);
  process.exit(1);
}

// One left-to-right pass: flags consume their value, everything else is an entry.
// (A two-pass scan would read flag VALUES containing "=" as bogus entries.)
const FLAGS = ["--out", "--functor", "--serve", "--manifest"];
const flags = {};
const positional = [];
for (let i = 0; i < process.argv.length - 2; i++) {
  const a = process.argv[i + 2];
  const eq = a.startsWith("--") ? a.indexOf("=") : -1;
  const [name, inline] = eq === -1 ? [a, null] : [a.slice(0, eq), a.slice(eq + 1)];
  if (FLAGS.includes(name)) {
    const v = inline ?? process.argv[i + 3];
    if (v === undefined) die(`${name} needs a value`);
    flags[name] = v;
    if (inline === null) i++;
  } else if (a.startsWith("--")) die(`unknown option ${name}`);
  else positional.push(a);
}

const repoRoot = path.resolve(import.meta.dirname, "../../../..");
const outDir = path.resolve(flags["--out"] ?? die("--out <dir> is required"));
const functor = path.resolve(flags["--functor"] ?? path.join(repoRoot, "target/release/functor"));
const servePort = flags["--serve"];
const MARKER = ".functor-gallery";

let entries;
if (flags["--manifest"]) {
  entries = JSON.parse(fs.readFileSync(flags["--manifest"], "utf8"));
  if (!Array.isArray(entries)) die("manifest must be a JSON array of entry objects");
} else {
  entries = positional.map((a) => {
    const i = a.indexOf("=");
    if (i === -1) die(`expected <name>=<projectDir>, got "${a}"`);
    return { name: a.slice(0, i), dir: a.slice(i + 1) };
  });
}
if (!entries.length) die("no entries: pass <name>=<projectDir> args or --manifest <file>");

// Names become directory names under outDir and hrefs in the index — keep them
// to a single safe path segment, and unique.
const seen = new Set();
for (const e of entries) {
  if (!e || typeof e.name !== "string" || !/^[A-Za-z0-9][A-Za-z0-9._-]*$/.test(e.name))
    die(`invalid entry name ${JSON.stringify(e?.name)} (use letters, digits, . _ -)`);
  if (!e.dir) die(`entry "${e.name}" has no dir`);
  if (seen.has(e.name)) die(`duplicate entry name "${e.name}"`);
  seen.add(e.name);
}
if (!fs.existsSync(functor))
  die(`functor binary not found at ${functor}\n  build it with \`npm run build:cli\`, or pass --functor <path>`);

// `// gallery:` / `// gallery-controls:` header comments in game.fun.
function headerMeta(dir) {
  const f = path.join(dir, "game.fun");
  if (!fs.existsSync(f)) return {};
  const meta = {};
  for (const line of fs.readFileSync(f, "utf8").split("\n").slice(0, 40)) {
    const m = /^\s*\/\/\s*gallery(-controls)?:\s*(.+?)\s*$/.exec(line);
    if (m) meta[m[1] ? "controls" : "description"] = m[2];
  }
  return meta;
}

function newestCapture(dir) {
  const capDir = path.join(dir, ".captures");
  if (!fs.existsSync(capDir)) return null;
  const pngs = fs
    .readdirSync(capDir)
    .filter((f) => f.endsWith(".png"))
    .map((f) => ({ f, mtime: fs.statSync(path.join(capDir, f)).mtimeMs }))
    .sort((a, b) => b.mtime - a.mtime);
  return pngs.length ? path.join(capDir, pngs[0].f) : null;
}

const esc = (s) =>
  String(s ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

// --out is wiped on every run, so only ever wipe a dir this script made.
if (fs.existsSync(outDir) && fs.readdirSync(outDir).length && !fs.existsSync(path.join(outDir, MARKER)))
  die(`${outDir} is not empty and was not created by this script — refusing to delete it`);
fs.rmSync(outDir, { recursive: true, force: true });
fs.mkdirSync(path.join(outDir, "thumbs"), { recursive: true });
fs.writeFileSync(path.join(outDir, MARKER), "");

const built = [];
for (const e of entries) {
  const dir = path.resolve(e.dir);
  process.stdout.write(`[gallery] building ${e.name} … `);
  try {
    execFileSync(functor, ["-d", dir, "build", "wasm"], { stdio: "pipe", maxBuffer: 32 << 20 });
  } catch (err) {
    // An empty Buffer is truthy — check length, or a stdout-only failure prints nothing.
    const out = err.stderr?.length ? err.stderr : err.stdout?.length ? err.stdout : String(err);
    console.log(`FAILED\n${out.toString().trim().split("\n").slice(-6).join("\n")}`);
    continue;
  }
  const bundle = path.join(dir, "dist/web");
  if (!fs.existsSync(bundle)) {
    console.log(`FAILED (no dist/web produced)`);
    continue;
  }
  fs.cpSync(bundle, path.join(outDir, e.name), { recursive: true });

  const thumbSrc = e.thumb ? path.resolve(e.thumb) : newestCapture(dir);
  let thumb = null;
  if (thumbSrc && fs.existsSync(thumbSrc)) {
    thumb = `thumbs/${e.name}.png`;
    fs.copyFileSync(thumbSrc, path.join(outDir, thumb));
  }
  built.push({ ...headerMeta(dir), ...e, thumb });
  console.log("ok");
}
if (!built.length) die("every entry failed to build");

const card = (e) => `
<article class="card">
  ${e.thumb ? `<img class="shot" src="${esc(e.thumb)}" alt="${esc(e.name)}">` : `<div class="shot noshot">no capture</div>`}
  <div class="body">
    <div class="title"><h2>${esc(e.title ?? e.name)}</h2>${e.scores ? `<span class="scores">${esc(e.scores)}</span>` : ""}</div>
    ${e.description ? `<p class="desc">${esc(e.description)}</p>` : ""}
    ${e.controls ? `<p class="ctrls">${esc(e.controls)}</p>` : ""}
    <a class="play" href="${encodeURIComponent(e.name)}/">Play</a>
  </div>
</article>`;

const html = `<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Functor Game Jam — Play the Entries</title>
<style>
:root{--card:#1e1233;--line:#3a2258;--cyan:#4ff0e0;--pink:#ff5fa8;--violet:#a97bff;--fg:#e6dcff;--dim:#9d8cc0;
  --mono:"JetBrains Mono",ui-monospace,SFMono-Regular,Menlo,Consolas,monospace}
*{box-sizing:border-box}
body{margin:0;color:var(--fg);font-family:var(--mono);font-size:15px;line-height:1.55;min-height:100vh;
  background:radial-gradient(1100px 520px at 50% -180px,#2c1350 0%,transparent 70%),linear-gradient(180deg,#120a1f,#0b0616)}
header{max-width:1180px;margin:0 auto;padding:64px 24px 28px;text-align:center}
h1{margin:0 0 10px;font-size:clamp(28px,5vw,46px);letter-spacing:.14em;text-transform:uppercase;
  background:linear-gradient(92deg,var(--cyan),var(--pink));-webkit-background-clip:text;background-clip:text;color:transparent}
.sub{color:var(--dim);margin:0 auto;max-width:660px;font-size:14px}
.rule{height:1px;max-width:1180px;margin:28px auto 0;background:linear-gradient(90deg,transparent,var(--pink),var(--cyan),transparent)}
main{max-width:1180px;margin:0 auto;padding:34px 24px 80px;display:grid;gap:26px;
  grid-template-columns:repeat(auto-fill,minmax(330px,1fr))}
.card{background:linear-gradient(180deg,var(--card),#160d28);border:1px solid var(--line);border-radius:12px;
  overflow:hidden;display:flex;flex-direction:column;transition:border-color .18s,transform .18s,box-shadow .18s}
.card:hover{border-color:var(--cyan);transform:translateY(-3px);box-shadow:0 12px 34px rgba(79,240,224,.14)}
.shot{display:block;width:100%;aspect-ratio:16/10;object-fit:cover;background:#0b0616;border-bottom:1px solid var(--line)}
.noshot{display:flex;align-items:center;justify-content:center;color:var(--dim);font-size:12px;letter-spacing:.12em}
.body{padding:16px 18px 18px;display:flex;flex-direction:column;gap:10px;flex:1}
.title{display:flex;align-items:baseline;justify-content:space-between;gap:10px}
h2{margin:0;font-size:17px;letter-spacing:.09em;text-transform:uppercase;color:var(--cyan)}
.scores{font-size:11px;color:var(--pink);white-space:nowrap}
.desc{margin:0;font-size:13px;opacity:.9}
.ctrls{margin:0;font-size:12px;color:var(--dim);border-left:2px solid var(--violet);padding-left:10px}
.play{margin-top:auto;display:block;text-align:center;text-decoration:none;padding:9px;border:1px solid var(--pink);
  border-radius:7px;color:var(--pink);letter-spacing:.16em;font-size:12px;text-transform:uppercase;transition:background .18s,color .18s}
.play:hover{background:var(--pink);color:#160d28}
</style></head>
<body>
<header><h1>Functor Game Jam</h1>
<p class="sub">Each entry is a self-contained wasm bundle interpreting its <code>.fun</code> source in the browser. Click a card to play.</p></header>
<div class="rule"></div>
<main>${built.map(card).join("\n")}</main>
</body></html>
`;
fs.writeFileSync(path.join(outDir, "index.html"), html);
console.log(`[gallery] ${built.length}/${entries.length} entries → ${outDir}/index.html`);

if (servePort) {
  const { createServer } = await import("node:http");
  const types = { ".html": "text/html", ".js": "text/javascript", ".wasm": "application/wasm",
    ".json": "application/json", ".png": "image/png", ".fun": "text/plain",
    ".ogg": "audio/ogg", ".wav": "audio/wav", ".glb": "model/gltf-binary" };
  createServer((req, res) => {
    let rel;
    try {
      rel = decodeURIComponent(req.url.split("?")[0]);
    } catch {
      return res.writeHead(400).end("bad request"); // a stray "%" must not kill the server
    }
    let p = path.join(outDir, rel);
    // Prefix alone would also match a SIBLING dir (/out-secrets vs /out).
    if (p !== outDir && !p.startsWith(outDir + path.sep)) return res.writeHead(403).end();
    if (fs.existsSync(p) && fs.statSync(p).isDirectory()) p = path.join(p, "index.html");
    if (!fs.existsSync(p)) return res.writeHead(404).end("not found");
    res.writeHead(200, { "content-type": types[path.extname(p)] ?? "application/octet-stream" });
    fs.createReadStream(p).on("error", () => res.destroy()).pipe(res);
  }).listen(Number(servePort), "127.0.0.1", () =>
    console.log(`[gallery] serving http://127.0.0.1:${servePort}/`));
}
