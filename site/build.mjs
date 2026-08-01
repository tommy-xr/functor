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
import { execSync, spawnSync } from "node:child_process";
import { dirname } from "node:path";
import esbuild from "esbuild";
import { EXAMPLES, exampleEntryPath } from "./src/examples.ts";
import { renderApiReference } from "./src/api-reference-html.mjs";
import { injectHeader } from "./src/header.ts";

const site = fileURLToPath(new URL(".", import.meta.url));
const root = fileURLToPath(new URL("..", import.meta.url));
const dist = `${site}dist`;

// The API JSON is derived from the embedded `.funi` prelude and intentionally
// gitignored. Generate the exact input this build consumes so a clean checkout
// never depends on a stale or manually prepared artifact.
const apiReference = `${site}generated/api-reference.json`;
const docs = spawnSync(
  "cargo",
  [
    "run",
    "-q",
    "-p",
    "functor-docgen",
    "--",
    "--deny-undocumented",
    "--format",
    "json",
    "--output",
    apiReference,
  ],
  { cwd: root, stdio: "inherit" },
);
if (docs.status !== 0) process.exit(docs.status ?? 1);

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

// The API reference is prerendered into its page (see src/api-reference-html.mjs):
// a no-JS fetch of /docs/ must return the real signatures and docs, not a shell.
const reference = JSON.parse(await readFile(apiReference, "utf8"));
const rendered = renderApiReference(reference);
// An empty reference would prerender an empty page that still "builds" — the
// exact silent failure this prerender exists to prevent. Refuse it.
if (rendered.moduleCount === 0 || rendered.itemCount === 0) {
  console.error(
    `the generated API reference is empty (${rendered.moduleCount} modules, ` +
      `${rendered.itemCount} declarations) — refusing to ship a blank reference`
  );
  process.exit(1);
}
const PRERENDER = {
  "api-module-nav": rendered.nav,
  "api-reference": rendered.sections,
  "api-module-count": String(rendered.moduleCount),
  "api-item-count": String(rendered.itemCount),
};

for (const page of PAGES) {
  const target = `${dist}/${page}`;
  await mkdir(dirname(target), { recursive: true });
  if (page.endsWith(".html")) {
    let html = await readFile(`${site}${page}`, "utf8");
    // The shared header first (it carries the badge span), then stamp the badge.
    html = injectHeader(html, page).replace(
      /(<span class="version-badge"[^>]*>)[^<]*(<\/span>)/,
      `$1${badge}$2`
    );
    if (page === "docs/index.html") {
      for (const [marker, markup] of Object.entries(PRERENDER)) {
        const token = `<!--${marker}-->`;
        // Exactly one slot each: a second occurrence (say, in a comment above
        // the real slot) would take the markup and leave the slot empty, since
        // a string `replace` only rewrites the first match.
        const occurrences = html.split(token).length - 1;
        if (occurrences !== 1) {
          console.error(
            `docs/index.html has ${occurrences} ${token} prerender markers — expected exactly 1`
          );
          process.exit(1);
        }
        html = html.replace(token, () => markup);
      }
      const leftover = html.match(/<!--api-[a-z-]+-->/);
      if (leftover) {
        console.error(`docs/index.html still has an unfilled prerender marker: ${leftover[0]}`);
        process.exit(1);
      }
    }
    await writeFile(target, html);
  } else {
    await cp(`${site}${page}`, target);
  }
}

// Machine-readable mirrors of the reference, at stable URLs. The JSON is the
// exact artifact the page was rendered from; the Markdown is written straight
// to disk by the generator (no piping, so no output-size ceiling).
await cp(apiReference, `${dist}/docs/api.json`);
const apiMarkdown = spawnSync(
  "cargo",
  [
    "run",
    "-q",
    "-p",
    "functor-docgen",
    "--",
    "--deny-undocumented",
    "--format",
    "markdown",
    "--output",
    `${dist}/docs/api.md`,
  ],
  { cwd: root, stdio: "inherit" },
);
if (apiMarkdown.status !== 0) process.exit(apiMarkdown.status ?? 1);
// llms.txt (llmstxt.org): the entry point an agent fetches to find the rest.
await writeFile(
  `${dist}/llms.txt`,
  `# Functor

> A functional toolkit for building 3D games in Functor Lang — Functor's own
> interpreted, F#-inspired game-logic language. A game is a \`.fun\` file the
> Rust runtime interprets directly (native OpenGL, WebGL2, and Quest/OpenXR),
> with hot-reload and no build step for game logic.

## Docs

- [API reference (Markdown)](https://functor.games/docs/api.md): every engine module, type, and function available to a hosted \`.fun\` game, generated from the \`.funi\` prelude embedded in the runtime.
- [API reference (JSON)](https://functor.games/docs/api.json): the same reference as structured data (modules → items with \`qualified_name\`, \`kind\`, \`declaration\`, \`docs\`).
- [API reference (HTML)](https://functor.games/docs/): the searchable rendering of the same data.
- [Manual](https://functor.games/manual/): getting started, the MVU game contract, language principles, topic guides.
- [Driving games with agents](https://functor.games/manual/#agents): register \`functor mcp\`, then prefer \`run_game_code_unsafe\` with one bare \`async (game) => { ... }\` JavaScript function for trusted multi-step automation. It requires Node.js 20+ (\`FUNCTOR_NODE\` overrides the executable) and is deliberately RCE-equivalent, not sandboxed. The injected SDK observes state/scene/trace/captures, controls time and release-safe input, waits deterministically, reloads/rewinds, and returns a concrete call trace plus final state; captures are MCP image blocks. Use the lower-level tools for one-offs and clients without Node.
- [Sandbox](https://functor.games/sandbox.html): run and edit the sample games in the browser.

## Source

- [GitHub repository](https://github.com/tommy-xr/functor): the runtimes, the language crate, and the \`examples/\` games.
`,
);
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
    { source: example.source, output: exampleEntryPath(example.id) },
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

const bundle = {
  bundle: true,
  minify: true,
  // `minify` is what selects React's PRODUCTION build: esbuild defines
  // process.env.NODE_ENV as "production" when minifying a browser bundle, so
  // the dev react-dom is dead-code-eliminated. Keep them together.
  format: "esm",
  // The editor dynamic-imports the language wasm glue at runtime from /pkg/;
  // esbuild must not try to bundle that path (it's copied in above, or absent).
  external: ["/pkg/functor_lang_wasm.js"],
  outdir: `${dist}/assets`,
  logLevel: "info",
};

await esbuild.build({
  ...bundle,
  // Entry points are PATHS, not import specifiers: esbuild only falls back to
  // a `.ts` sibling when the named `.js` is absent, so a stale leftover
  // `src/docs.js` would silently win. Name the file that actually exists. The
  // OUTPUT basenames are unchanged either way (esbuild always emits `.js`).
  entryPoints: [
    `${site}src/demo-editor.ts`,
    `${site}src/docs.ts`,
    `${site}src/api-docs.ts`,
    `${site}src/features.ts`,
  ],
});

// Vim keybindings are opt-in, and their adapter is large enough that the
// standard editor must not pay for it. Build each editor as its own tiny split
// graph: the page entry + its normal CodeMirror chunk load immediately, while
// @replit/codemirror-vim remains a dynamic chunk fetched only after opt-in.
// Separate builds keep sandbox/IDE from hoisting their shared CodeMirror
// code into cross-page chunks (each page still deploys as an independent unit),
// and the per-entry chunk prefix prevents output-name collisions.
const editorBuilds = [
  ["sandbox", `${site}src/sandbox.tsx`, "sandbox.html"],
  ["ide", `${site}src/ide.tsx`, "ide.html"],
];

const VIM_ADAPTER = "node_modules/@replit/codemirror-vim";

const assertLazyVim = (result, label) => {
  const outputs = Object.entries(result.metafile.outputs);
  const vimOutput = outputs.find(([, meta]) =>
    meta.entryPoint?.includes(`${VIM_ADAPTER}/dist/index.js`)
  );
  if (!vimOutput) {
    console.error(`editor build did not emit a lazy Vim chunk for ${label}`);
    process.exit(1);
  }
  const leak = outputs
    .filter(([output]) => output !== vimOutput[0])
    .find(([, meta]) =>
      Object.keys(meta.inputs).some((input) => input.includes(VIM_ADAPTER))
    );
  if (leak) {
    console.error(`Vim adapter leaked out of its lazy chunk into ${leak[0]}`);
    process.exit(1);
  }
};

const outputNamed = (result, name) =>
  Object.entries(result.metafile.outputs).find(
    ([output]) => output.split("/").pop() === name
  );

const assertPreloaded = async (result, htmlName, eagerOutputNames) => {
  const html = await readFile(`${dist}/${htmlName}`, "utf8");
  const preloaded = new Set(
    [...html.matchAll(/<link rel="modulepreload" href="assets\/([^"]+)"/g)].map(
      (match) => match[1]
    )
  );
  const required = new Set(eagerOutputNames);
  for (const outputName of eagerOutputNames) {
    const output = outputNamed(result, outputName);
    if (!output) {
      console.error(`${htmlName} expects missing build output ${outputName}`);
      process.exit(1);
    }
    for (const imported of output[1].imports) {
      if (imported.kind === "import-statement") {
        required.add(imported.path.split("/").pop());
      }
    }
  }
  const missing = [...required].filter((name) => !preloaded.has(name));
  if (missing.length) {
    console.error(
      `${htmlName} is missing modulepreload links for ${JSON.stringify(missing)}`
    );
    process.exit(1);
  }
};

for (const [name, entry, htmlName] of editorBuilds) {
  const result = await esbuild.build({
    ...bundle,
    entryPoints: [entry],
    splitting: true,
    chunkNames: `${name}-[name]`,
    metafile: true,
  });
  assertLazyVim(result, name);
  await assertPreloaded(result, htmlName, [`${name}-chunk.js`]);
}

// The hero is built SEPARATELY, with code splitting on, because it is the only
// entry whose weight is deferred: src/hero.ts is a ~170-byte eager loader that
// dynamic-imports src/hero-app.ts, and esbuild only emits that as its own
// output file when `splitting` is enabled (otherwise it inlines the imported
// module back into the entry and the landing page pays for it up front again).
//
// A separate call from the editor builds keeps the landing island independent;
// `chunkNames` is namespaced for the same reason — every split build shares an
// outdir, so their chunks must not be able to collide.
//
// The chunk name is deliberately UNHASHED, like every other output here. A
// hash would make the (unhashed, therefore cacheable-stale) hero.js point at a
// filename that no longer exists after a redeploy — a 404 instead of merely
// old-but-working code — and index.html could not name it in a static
// modulepreload. `...bundle` goes FIRST in both calls so a key added to the
// shared options can never silently override the hero's splitting settings.
const heroBuild = await esbuild.build({
  ...bundle,
  entryPoints: [`${site}src/hero.ts`],
  splitting: true,
  chunkNames: "hero-[name]",
  metafile: true,
});
assertLazyVim(heroBuild, "hero");

// The landing page preloads both the deferred island and its static shared
// chunk. Otherwise the tiny island shell would introduce a serial discovery
// hop before the CodeMirror payload despite the preload above it.
await assertPreloaded(heroBuild, "index.html", ["hero-hero-app.js"]);

console.log(`site built at ${dist}`);
