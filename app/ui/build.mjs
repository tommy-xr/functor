// Builds the desktop app's UI into app/ui/dist: the page, one ES module
// bundle with its stylesheet, and the Functor icon. `npm run build:app-ui`.
// functor-app embeds dist when it exists at its build time, or serves it
// with `--ui-dir app/ui/dist`.

import { cp, rm } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import esbuild from "esbuild";

const ui = fileURLToPath(new URL(".", import.meta.url));
const root = fileURLToPath(new URL("../..", import.meta.url));
const dist = `${ui}dist`;

await rm(dist, { recursive: true, force: true });
await esbuild.build({
  entryPoints: [`${ui}src/main.tsx`],
  bundle: true,
  // Also selects React's production build.
  minify: true,
  format: "esm",
  outfile: `${dist}/app.js`,
  logLevel: "info",
});
await cp(`${ui}index.html`, `${dist}/index.html`);
await cp(`${root}docs/media/functor-icon.svg`, `${dist}/icon.svg`);
