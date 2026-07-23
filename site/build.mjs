// Build the static site into site/dist: bundle the sandbox editor, copy the
// pages, the wasm runtime (built separately by wasm-pack — see the error
// below), and the example .fun sources. The repo examples are copied at
// build time rather than duplicated here, so the sandbox always shows the
// same code that ships in examples/.
//
//   node site/build.mjs
//
// The output is fully static — deploy site/dist to any static host.

import { cp, mkdir, rm, access, readFile, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { execSync } from "node:child_process";
import { dirname } from "node:path";
import esbuild from "esbuild";
import { EXAMPLES } from "./src/examples.js";

const site = fileURLToPath(new URL(".", import.meta.url));
const root = fileURLToPath(new URL("..", import.meta.url));
const dist = `${site}dist`;

const PAGES = [
  "index.html",
  "sandbox.html",
  "ide.html",
  "player.html",
  "docs.html",
  "docs/index.html",
  "manual/index.html",
  "demo-editor.html",
  "styles.css",
];

// Favicons, generated from docs/media/functor-icon.svg by `npm run generate:icons`
// (gitignored — site:build regenerates them first). Copied defensively so a stale
// checkout without them still builds.
const ICONS = ["favicon.svg", "favicon.ico", "favicon-16.png", "favicon-32.png", "apple-touch-icon.png"];

const PKG = `${root}runtime/functor-runtime-web/pkg`;
const PKG_FILES = ["functor_runtime_web.js", "functor_runtime_web_bg.wasm"];
// The shared time-travel scrubber, imported by player.html (served next to it,
// like pkg/). Single source in the runtime crate — copied, never duplicated.
const SCRUBBER = `${root}runtime/functor-runtime-web/scrubber.js`;
const TIMELINE_MODEL = `${root}runtime/functor-runtime-web/timeline-model.js`;

// The editor's in-browser language intelligence (diagnostics/hover), a separate
// small wasm bundle built by `npm run build:lang-wasm`. Optional: the site must
// still build without it (the editor just loses live analysis), so a missing
// pkg is a note, not an error.
const LANG_PKG = `${root}tools/functor-lang-wasm/pkg`;
const LANG_PKG_FILES = ["functor_lang_wasm.js", "functor_lang_wasm_bg.wasm"];

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

// The header version-badge names the build. A deploy (CI — the functor.games
// build) names the release it ships from: the nearest reachable vX.Y.Z, walked
// back from HEAD (hence the deploy's fetch-depth: 0). A local build is stricter
// — only an exact, clean release tag counts; dev work, a commit ahead of a tag,
// or a dirty tag checkout all read "v0.0.0 · dev", so a working copy never
// mislabels itself as a release it merely descends from.
let badge = "v0.0.0 · dev";
try {
  const describe = process.env.CI
    ? "git describe --tags --abbrev=0 --match 'v[0-9]*'"
    : "git describe --tags --exact-match --dirty --match 'v[0-9]*'";
  const tag = execSync(describe, {
    cwd: root,
    stdio: ["ignore", "pipe", "ignore"],
  })
    .toString()
    .trim();
  if (/^v\d+\.\d+\.\d+$/.test(tag)) badge = `${tag} · alpha`;
} catch {}

for (const page of PAGES) {
  const target = `${dist}/${page}`;
  await mkdir(dirname(target), { recursive: true });
  if (page.endsWith(".html")) {
    const html = await readFile(`${site}${page}`, "utf8");
    await writeFile(
      target,
      html.replace(
        /(<span class="version-badge"[^>]*>)[^<]*(<\/span>)/,
        `$1${badge}$2`
      )
    );
  } else {
    await cp(`${site}${page}`, target);
  }
}
for (const icon of ICONS) {
  try {
    await cp(`${site}${icon}`, `${dist}/${icon}`);
  } catch {
    console.warn(`note: ${icon} missing — run \`npm run generate:icons\``);
  }
}
for (const file of PKG_FILES) {
  await cp(`${PKG}/${file}`, `${dist}/pkg/${file}`);
}
await cp(SCRUBBER, `${dist}/scrubber.js`);
await cp(TIMELINE_MODEL, `${dist}/timeline-model.js`);
// Feature-showcase GIFs (committed + optimised in site/media) → dist/media. They
// are regenerated from the site/demos scripts; the committed copies are what the
// landing page embeds (building them in CI would need the runtime + Playwright).
await cp(`${site}media`, `${dist}/media`, { recursive: true }).catch(() => {});
for (const example of EXAMPLES) {
  const files = [
    { source: example.source, output: `examples/${example.id}.fun` },
    ...(example.siblings ?? []),
  ];
  for (const { source, output } of [...files, ...(example.assets ?? [])]) {
    const slash = output.lastIndexOf("/");
    if (slash >= 0) {
      await mkdir(`${dist}/${output.slice(0, slash)}`, { recursive: true });
    }
    await cp(`${root}${source}`, `${dist}/${output}`);
  }
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
  entryPoints: [
    `${site}src/sandbox.js`,
    `${site}src/ide.js`,
    `${site}src/docs.js`,
    `${site}src/api-docs.js`,
    `${site}src/hero.js`,
    `${site}src/demo-editor.js`,
    `${site}src/features.js`,
  ],
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
