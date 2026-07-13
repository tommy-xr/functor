// Build the static site into site/dist: bundle the sandbox editor, copy the
// pages, the wasm runtime (built separately by wasm-pack — see the error
// below), and the example .fun sources. The repo examples are copied at
// build time rather than duplicated here, so the sandbox always shows the
// same code that ships in examples/.
//
//   node site/build.mjs
//
// The output is fully static — deploy site/dist to any static host.

import { cp, mkdir, rm, access } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import esbuild from "esbuild";

const site = fileURLToPath(new URL(".", import.meta.url));
const root = fileURLToPath(new URL("..", import.meta.url));
const dist = `${site}dist`;

const PAGES = ["index.html", "sandbox.html", "ide.html", "player.html", "docs.html", "styles.css"];

const PKG = `${root}runtime/functor-runtime-web/pkg`;
const PKG_FILES = ["functor_runtime_web.js", "functor_runtime_web_bg.wasm"];

// The editor's in-browser language intelligence (diagnostics/hover), a separate
// small wasm bundle built by `npm run build:lang-wasm`. Optional: the site must
// still build without it (the editor just loses live analysis), so a missing
// pkg is a note, not an error.
const LANG_PKG = `${root}tools/functor-lang-wasm/pkg`;
const LANG_PKG_FILES = ["functor_lang_wasm.js", "functor_lang_wasm_bg.wasm"];

// dist/examples/<name>.fun — site-local plus the repo's Functor Lang examples.
const EXAMPLES = {
  hero: `${site}examples/hero.fun`,
  primitives: `${root}examples/primitives/game.fun`,
  // Named `bounce` (not `physics`): the flat copy makes `file = module`, and a
  // module literally named `Physics` collides with the builtin/prelude namespace.
  bounce: `${root}examples/physics/game.fun`,
  monitor: `${root}examples/monitor/game.fun`,
};

try {
  await access(`${PKG}/${PKG_FILES[0]}`);
} catch {
  console.error(
    `missing ${PKG_FILES[0]} — build the web runtime first:\n` +
      `  wasm-pack build runtime/functor-runtime-web --target=web`
  );
  process.exit(1);
}

await rm(dist, { recursive: true, force: true });
await mkdir(`${dist}/pkg`, { recursive: true });
await mkdir(`${dist}/examples`, { recursive: true });

for (const page of PAGES) {
  await cp(`${site}${page}`, `${dist}/${page}`);
}
for (const file of PKG_FILES) {
  await cp(`${PKG}/${file}`, `${dist}/pkg/${file}`);
}
for (const [name, path] of Object.entries(EXAMPLES)) {
  await cp(path, `${dist}/examples/${name}.fun`);
}

// The language-intelligence wasm, if it has been built. Absent → skip (build on).
let langPkgPresent = false;
try {
  await access(`${LANG_PKG}/${LANG_PKG_FILES[0]}`);
  langPkgPresent = true;
} catch {
  console.log(
    `note: ${LANG_PKG_FILES[0]} not found — skipping the editor language pkg ` +
      `(build it with \`npm run build:lang-wasm\`)`
  );
}
if (langPkgPresent) {
  for (const file of LANG_PKG_FILES) {
    await cp(`${LANG_PKG}/${file}`, `${dist}/pkg/${file}`);
  }
}

await esbuild.build({
  entryPoints: [`${site}src/sandbox.js`, `${site}src/ide.js`, `${site}src/docs.js`, `${site}src/hero.js`],
  bundle: true,
  minify: true,
  format: "esm",
  // The editor dynamic-imports the language wasm glue at runtime from /pkg/;
  // esbuild must not try to bundle that path (it's copied in above, or absent).
  external: ["/pkg/functor_lang_wasm.js"],
  outdir: `${dist}/assets`,
  logLevel: "info",
});

console.log(`site built at ${dist}`);
