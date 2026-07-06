// Build the static site into site/dist: bundle the sandbox editor, copy the
// pages, the wasm runtime (built separately by wasm-pack — see the error
// below), and the example .mle sources. The repo examples are copied at
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

const PAGES = ["index.html", "sandbox.html", "player.html", "docs.html", "styles.css"];

const PKG = `${root}runtime/functor-runtime-web/pkg`;
const PKG_FILES = ["functor_runtime_web.js", "functor_runtime_web_bg.wasm"];

// dist/examples/<name>.mle — site-local plus the repo's MLE examples.
const EXAMPLES = {
  hero: `${site}examples/hero.mle`,
  orbit: `${root}examples/hello-cubes/game.mle`,
  physics: `${root}examples/physics/game.mle`,
  monitor: `${root}examples/monitor/game.mle`,
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
  await cp(path, `${dist}/examples/${name}.mle`);
}

await esbuild.build({
  entryPoints: [`${site}src/sandbox.js`, `${site}src/docs.js`],
  bundle: true,
  minify: true,
  format: "esm",
  outdir: `${dist}/assets`,
  logLevel: "info",
});

console.log(`site built at ${dist}`);
