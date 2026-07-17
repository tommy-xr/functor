// Derives all rasterized app icons from the single source-of-truth mark,
// docs/media/functor-icon.svg (a transparent lowercase-f).
//
// The mark ships in two theme colors, chosen per surface at generation time:
//   cyan #41d8e6  on dark grounds / dark editor themes (matches the site accent)
//   blue #2aa9e0  on light grounds / light editor themes (darker, holds contrast on white)
//
// Run: npm run generate:icons
//
// Outputs (gitignored — regenerate whenever the SVG changes):
//   site/favicon.svg                          adaptive (blue, cyan under prefers-color-scheme: dark)
//   site/favicon.ico                          16/32/48, blue (safe on any tab color)
//   site/favicon-16.png / -32.png             blue
//   site/apple-touch-icon.png                 180 on the brand tile, cyan
//   tools/functor-lang-vscode/images/functor-lang-file-dark.png    file icon, cyan (dark themes)
//   tools/functor-lang-vscode/images/functor-lang-file-light.png   file icon, blue (light themes)
//   tools/functor-lang-vscode/images/functor-lang-extension.png    Marketplace tile, cyan on tile
//
// Rasterization is @resvg/resvg-js (prebuilt, no system deps); .ico is png-to-ico.
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { Resvg } from "@resvg/resvg-js";
import pngToIco from "png-to-ico";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const SRC = "docs/media/functor-icon.svg";

const CYAN = "#41d8e6"; // dark grounds / dark themes
const BLUE = "#2aa9e0"; // light grounds / light themes (also the universal fallback)
const TILE = "#030817"; // brand background for surfaces that can't be transparent

// The source paints the mark in BLUE; swap that token to recolor per surface.
const src = readFileSync(resolve(root, SRC), "utf8");
const recolor = (fill) => src.replaceAll(BLUE, fill);

// An SVG favicon that follows the OS theme: blue by default, cyan in dark mode.
function adaptiveSvg() {
  const style =
    "<style>.mark{fill:" + BLUE + "}" +
    "@media(prefers-color-scheme:dark){.mark{fill:" + CYAN + "}}</style>";
  return src.replace(`<g fill="${BLUE}">`, `${style}\n  <g class="mark">`);
}

// Render the source (recolored to `fill`) to a `size`x`size` PNG buffer.
// `background` composites onto a solid tile; omit it for transparency.
function png(size, fill, background) {
  const r = new Resvg(recolor(fill), { fitTo: { mode: "width", value: size }, background });
  return r.render().asPng();
}

function write(relPath, buf) {
  const out = resolve(root, relPath);
  mkdirSync(dirname(out), { recursive: true });
  writeFileSync(out, buf);
  console.log(`  ${relPath.padEnd(58)} ${buf.length.toLocaleString()} bytes`);
}

console.log(`Deriving icons from ${SRC}\n`);

const VSCODE = "tools/functor-lang-vscode/images";

// Adaptive SVG favicon (modern browsers pick this first).
write("site/favicon.svg", Buffer.from(adaptiveSvg()));

// Blue raster favicons — the fallback for browsers that ignore the SVG; blue reads on any tab.
write("site/favicon-16.png", png(16, BLUE));
write("site/favicon-32.png", png(32, BLUE));
write("site/favicon.ico", await pngToIco([16, 32, 48].map((s) => png(s, BLUE))));

// VS Code file icon — one per theme (VS Code picks light/dark itself).
write(`${VSCODE}/functor-lang-file-dark.png`, png(128, CYAN));
write(`${VSCODE}/functor-lang-file-light.png`, png(128, BLUE));

// Opaque tiles (iOS home screen + Marketplace) sit on the dark brand tile, so cyan.
write("site/apple-touch-icon.png", png(180, CYAN, TILE));
write(`${VSCODE}/functor-lang-extension.png`, png(128, CYAN, TILE));

console.log("\nDone.");
