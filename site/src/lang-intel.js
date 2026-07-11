// Live language intelligence for the sandbox editor: loads the small
// functor-lang analysis wasm (built by `npm run build:lang-wasm`, copied to
// dist/pkg/ by build.mjs) and turns its type diagnostics into CodeMirror lint
// underlines. Optional by design — if the pkg is absent or fails to init, the
// setup resolves to [] and the editor works exactly as before (no analysis,
// no console spam beyond one info line).
//
// The parsed analyze result is memoized per doc string (analyzeCached) so the
// same pass backs both the linter and commit 8's inlays/lenses.

import { linter } from "@codemirror/lint";
import { EditorView } from "@codemirror/view";

// The runtime import specifier resolves to the glue esbuild leaves external
// (see build.mjs `external`), copied to /pkg/ at build time — NOT bundled.
const PKG_URL = "/pkg/functor_lang_wasm.js";

let analyzeFn = null; // (src) => JSON string, set once the wasm is ready
let lastDoc = null;
let lastResult = null;

// Run analyze at most once per distinct doc string. Returns the parsed
// `{ diagnostics, inlays, lenses }`, or null when the wasm isn't loaded.
export const analyzeCached = (docString) => {
  if (!analyzeFn) return null;
  if (docString === lastDoc) return lastResult;
  let result;
  try {
    result = JSON.parse(analyzeFn(docString));
  } catch {
    result = { diagnostics: [], inlays: [], lenses: [] };
  }
  lastDoc = docString;
  lastResult = result;
  return result;
};

// analyze offsets are whole-document UTF-16 — CodeMirror's native unit — so
// they map straight across; clamp defensively to the current doc length.
const toDiagnostics = (view) => {
  const doc = view.state.doc;
  const result = analyzeCached(doc.toString());
  if (!result || !Array.isArray(result.diagnostics)) return [];
  const len = doc.length;
  return result.diagnostics.map((d) => {
    const from = Math.max(0, Math.min(d.from | 0, len));
    let to = Math.max(from, Math.min(d.to | 0, len));
    return { from, to, severity: d.severity || "error", message: d.message || "" };
  });
};

// Recolor lint's wavy underline to the calm theme's red (--red: #f2637f)
// instead of its default bright #f11. Mirrors @codemirror/lint's own helper.
const wavyUnderline = (color) =>
  `url('data:image/svg+xml,<svg xmlns="http://www.w3.org/2000/svg" width="6" height="3">` +
  encodeURIComponent(
    `<path d="m0 2.5 l2 -1.5 l1 0 l2 1.5 l1 0" stroke="${color}" fill="none" stroke-width=".7"/>`
  ) +
  `</svg>')`;

const lintTheme = EditorView.theme({
  ".cm-lintRange-error": { backgroundImage: wavyUnderline("#f2637f") },
});

// Async setup: resolve to the lint extensions, or [] on any failure so the
// editor degrades to no-analysis silently.
export const setupLangIntel = async () => {
  try {
    const mod = await import(PKG_URL);
    await mod.default(); // init the wasm
    analyzeFn = mod.functor_lang_analyze;
  } catch {
    console.info(
      "[lang-intel] language analysis unavailable (pkg not built) — editor runs without diagnostics"
    );
    return [];
  }
  return [linter(toDiagnostics, { delay: 300 }), lintTheme];
};
