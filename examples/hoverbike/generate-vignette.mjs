// Generate `vignette.png`, the comfort vignette's feathered annulus: black,
// clear inside `INNER` of the half-width, opaque from `OUTER` out, with a
// smoothstep between. `bars.fun` scales it so the clear radius is the
// aperture. Node built-ins only; re-running reproduces it byte for byte:
//
//   node examples/hoverbike/generate-vignette.mjs
import { writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { encodePng } from "../../scripts/png.mjs";

const SIZE = 256;
// Mirrors bars.fun's `vignetteInner` / `vignetteOuter`.
const INNER = 0.5;
const OUTER = 0.86;

const smoothstep = (a, b, v) => {
  const t = Math.max(0, Math.min(1, (v - a) / (b - a)));
  return t * t * (3 - 2 * t);
};

const pixels = Buffer.alloc(SIZE * SIZE * 4);
for (let y = 0; y < SIZE; y += 1) {
  for (let x = 0; x < SIZE; x += 1) {
    const r = Math.hypot((x + 0.5) / SIZE - 0.5, (y + 0.5) / SIZE - 0.5) * 2;
    pixels[(y * SIZE + x) * 4 + 3] = Math.round(255 * smoothstep(INNER, OUTER, r));
  }
}
writeFileSync(fileURLToPath(new URL("vignette.png", import.meta.url)), encodePng(SIZE, SIZE, 8, 6, pixels));
