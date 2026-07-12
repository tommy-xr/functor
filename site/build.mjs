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
import esbuild from "esbuild";
import { marked, Renderer } from "marked";
import * as pagefind from "pagefind";
import { highlight, toBase64Url, escapeHtml } from "./src/highlight.mjs";
import { renderDocsPage } from "./src/docs-page.mjs";

const site = fileURLToPath(new URL(".", import.meta.url));
const root = fileURLToPath(new URL("..", import.meta.url));
const dist = `${site}dist`;

const PAGES = ["index.html", "sandbox.html", "player.html", "styles.css"];

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
  entryPoints: [`${site}src/sandbox.js`, `${site}src/hero.js`],
  bundle: true,
  minify: true,
  format: "esm",
  // The editor dynamic-imports the language wasm glue at runtime from /pkg/;
  // esbuild must not try to bundle that path (it's copied in above, or absent).
  external: ["/pkg/functor_lang_wasm.js"],
  outdir: `${dist}/assets`,
  logLevel: "info",
});

await buildDocs();
await buildSearchIndex();

console.log(`site built at ${dist}`);

// ---------------------------------------------------------------- search
// Pagefind's programmatic Node API indexes the rendered site into dist/pagefind/
// (its stock UI fetches this at runtime). We addDirectory over all of dist so
// result URLs are root-relative (/docs/<slug>/), but ONLY the docs pages carry
// data-pagefind-body — and once any page has it, Pagefind excludes every page
// without it, so the index is exactly the docs. The index is the sole reason
// docs pages load any JS; a build without it leaves the guarded UI mount a no-op.
async function buildSearchIndex() {
  const t0 = performance.now();
  const { index } = await pagefind.createIndex();
  await index.addDirectory({ path: dist });
  await index.writeFiles({ outputPath: `${dist}/pagefind` });
  await pagefind.close();
  console.log(`docs search index built in ${Math.round(performance.now() - t0)}ms`);
}

// ------------------------------------------------------------------- docs
// The docs are a build-time markdown pipeline. site/docs/manifest.json is the
// single nav source of truth (ordered groups → { slug, title } entries); each
// slug is a sibling <slug>.md rendered through marked into a page. The `index`
// slug is the docs root (dist/docs/index.html → served at /docs/); every other
// slug nests (dist/docs/<slug>/index.html → /docs/<slug>/) for clean URLs.
async function buildDocs() {
  const manifest = JSON.parse(await readFile(`${site}docs/manifest.json`, "utf8"));

  // Relative path from a page's URL back to the site root. `index` lives at
  // /docs/ ("../"); a flat slug at /docs/<slug>/ ("../../"); a nested slug like
  // `compare/elm` at /docs/compare/elm/ ("../../../") — one "../" per URL segment.
  const rootPrefixFor = (slug) =>
    slug === "index" ? "../" : "../".repeat(slug.split("/").length + 1);
  const docHref = (slug, rootPrefix) =>
    slug === "index" ? `${rootPrefix}docs/` : `${rootPrefix}docs/${slug}/`;

  const renderSidebar = (currentSlug, rootPrefix) =>
    manifest.groups
      .flatMap((group) => [
        `        <div class="docs-nav-group">${escapeHtml(group.title)}</div>`,
        ...group.entries.map((entry) => {
          const current = entry.slug === currentSlug ? ` aria-current="page"` : "";
          return `        <a href="${docHref(entry.slug, rootPrefix)}"${current}>${escapeHtml(entry.title)}</a>`;
        }),
      ])
      .join("\n");

  // A code fence's info string drives the render: `functor` highlights (and
  // `functor run` adds the sandbox try-button), `sh`/`shell` keep the shell
  // style, anything else is a plain escaped block.
  const renderCode = (text, lang, rootPrefix) => {
    const words = (lang || "").trim().split(/\s+/);
    if (words[0] === "functor") {
      const button = words.includes("run")
        ? `<a class="try-button" href="${rootPrefix}sandbox.html#src=${toBase64Url(text)}" title="Open this program live in the sandbox" target="_blank">▶ try it</a>`
        : "";
      return `<pre class="functor-lang">${highlight(text)}${button}</pre>`;
    }
    if (words[0] === "sh" || words[0] === "shell") {
      return `<pre class="shell">${escapeHtml(text)}</pre>`;
    }
    return `<pre>${escapeHtml(text)}</pre>`;
  };

  for (const entry of manifest.groups.flatMap((g) => g.entries)) {
    const rootPrefix = rootPrefixFor(entry.slug);
    const md = await readFile(`${site}docs/${entry.slug}.md`, "utf8");
    const renderer = new Renderer();
    renderer.code = ({ text, lang }) => renderCode(text, lang, rootPrefix);
    const content = marked.parse(md, { renderer });
    const html = renderDocsPage({
      title: entry.title,
      sidebar: renderSidebar(entry.slug, rootPrefix),
      content,
      rootPrefix,
    });
    const outDir = entry.slug === "index" ? `${dist}/docs` : `${dist}/docs/${entry.slug}`;
    await mkdir(outDir, { recursive: true });
    await writeFile(`${outDir}/index.html`, html);
  }

  // Meta-refresh stub so the old /docs.html link still lands on the docs index.
  await writeFile(
    `${dist}/docs.html`,
    `<!doctype html>
<meta charset="utf-8" />
<meta http-equiv="refresh" content="0; url=docs/" />
<link rel="canonical" href="docs/" />
<title>functor docs</title>
<p>Redirecting to <a href="docs/">the docs</a>…</p>
`
  );
}
