// Live language intelligence for the sandbox editor: loads the small
// functor-lang analysis wasm (built by `npm run build:lang-wasm`, copied to
// dist/pkg/ by build.mjs) and turns its type diagnostics into CodeMirror lint
// underlines, plus hover types, inlay hints, and signature codelenses.
// Optional by design — if the pkg is absent or fails to init, the setup
// resolves to [] and the editor works exactly as before (no analysis, no
// console spam beyond one info line).
//
// The parsed analyze result is memoized per doc string (analyzeCached) so the
// SAME pass backs the linter, the inlays, and the lenses — one analyze per doc
// version, never a second. The lint source runs the analyze (debounced); the
// decoration StateField only PEEKS the cache and refreshes when the lint pass
// has filled it, so inlays/lenses lag the doc by the lint debounce (acceptable
// and cheaper than a second scheduler) instead of re-analyzing on every key.

import { linter } from "@codemirror/lint";
import { Decoration, EditorView, ViewPlugin, WidgetType, hoverTooltip } from "@codemirror/view";
import { StateEffect, StateField } from "@codemirror/state";

// The runtime import specifier resolves to the glue esbuild leaves external
// (see build.mjs `external`), copied to /pkg/ at build time — NOT bundled.
const PKG_URL = "/pkg/functor_lang_wasm.js";

let analyzeFn = null; // (src) => JSON string, set once the wasm is ready
let hoverFn = null; // (src, offset) => JSON string ("" when nothing to show)
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

// Read the memoized result WITHOUT running analyze: returns it only when it is
// already current for `docString`, else null. The decoration field uses this so
// it never triggers an analyze of its own (the lint source is the sole caller
// that fills the cache).
const peekCached = (docString) =>
  analyzeFn && docString === lastDoc ? lastResult : null;

// analyze offsets are whole-document UTF-16 — CodeMirror's native unit — so
// they map straight across; clamp defensively to the current doc length.
const toDiagnostics = (view) => {
  const doc = view.state.doc;
  const result = analyzeCached(doc.toString());
  // The lint pass just refreshed the cache for this doc; nudge the decoration
  // field to re-read it (the initial doc lands here too, so inlays/lenses show
  // on load — not only after the first edit).
  scheduleRefresh(view);
  if (!result || !Array.isArray(result.diagnostics)) return [];
  const len = doc.length;
  return result.diagnostics.map((d) => {
    const from = Math.max(0, Math.min(d.from | 0, len));
    let to = Math.max(from, Math.min(d.to | 0, len));
    return { from, to, severity: d.severity || "error", message: d.message || "" };
  });
};

// --- Hover types --------------------------------------------------------------
// Ask the wasm for the type under the cursor (UTF-16 offset == CodeMirror pos)
// and render it monospace in a small calm-theme tooltip.
const hoverTypes = hoverTooltip((view, pos) => {
  if (!hoverFn) return null;
  let info;
  try {
    info = JSON.parse(hoverFn(view.state.doc.toString(), pos) || "");
  } catch {
    return null;
  }
  if (!info || !info.text) return null;
  return {
    pos: info.from | 0,
    end: info.to | 0,
    create: () => {
      const dom = document.createElement("div");
      dom.className = "cm-hover-type";
      dom.textContent = info.text;
      return { dom };
    },
  };
});

// --- Inlay hints + codelenses (one StateField) --------------------------------
// Block decorations (the codelenses) MUST come from a StateField, not a
// ViewPlugin — CodeMirror rejects block decorations from plugins. Inlays ride
// along in the same field so both share the one cache-driven refresh.

class InlayWidget extends WidgetType {
  constructor(label) {
    super();
    this.label = label;
  }
  eq(other) {
    return other.label === this.label;
  }
  toDOM() {
    const span = document.createElement("span");
    span.className = "cm-inlay";
    span.textContent = this.label;
    return span;
  }
  ignoreEvent() {
    return true;
  }
}

class LensWidget extends WidgetType {
  constructor(text, indent) {
    super();
    this.text = text;
    this.indent = indent;
  }
  eq(other) {
    return other.text === this.text && other.indent === this.indent;
  }
  toDOM() {
    const div = document.createElement("div");
    div.className = "cm-lens";
    div.textContent = this.text;
    if (this.indent) div.style.paddingLeft = `${this.indent}ch`;
    return div;
  }
  ignoreEvent() {
    return true;
  }
}

// Build the combined decoration set from the CACHED analysis for this doc, or
// null when the cache is stale/empty (the field then keeps what it has).
const buildDecorations = (state) => {
  const result = peekCached(state.doc.toString());
  if (!result) return null;
  const doc = state.doc;
  const len = doc.length;
  const ranges = [];
  for (const it of result.inlays || []) {
    const pos = Math.max(0, Math.min(it.pos | 0, len));
    ranges.push(
      Decoration.widget({ widget: new InlayWidget(String(it.label ?? "")), side: 1 }).range(pos)
    );
  }
  for (const it of result.lenses || []) {
    const from = Math.max(0, Math.min(it.from | 0, len));
    const line = doc.lineAt(from);
    ranges.push(
      Decoration.widget({
        widget: new LensWidget(String(it.text ?? ""), from - line.from),
        block: true,
        side: -1,
      }).range(line.from)
    );
  }
  return Decoration.set(ranges, true);
};

// A signal to re-derive decorations from the (freshly filled) cache.
const refreshDecorations = StateEffect.define();

// Fired from the lint source after it fills the cache; a rAF avoids dispatching
// mid-update. Only refreshes when the cache is current, so a keystroke landing
// in the same frame doesn't clear the decorations.
const scheduleRefresh = (view) =>
  requestAnimationFrame(() => view.dispatch({ effects: refreshDecorations.of(null) }));

// Decorate the INITIAL doc once, without waiting for an edit: the linter's
// first pass isn't guaranteed to fire on load, so this one-shot analyzes the
// starting buffer and refreshes. Ongoing edits are covered by the lint pass
// (scheduleRefresh), so this fires exactly once.
const initialRefresh = ViewPlugin.fromClass(
  class {
    constructor(view) {
      requestAnimationFrame(() => {
        analyzeCached(view.state.doc.toString());
        view.dispatch({ effects: refreshDecorations.of(null) });
      });
    }
  }
);

const decorationField = StateField.define({
  create: (state) => buildDecorations(state) || Decoration.none,
  update(value, tr) {
    let decos = value;
    // Keep decorations glued to their text between refreshes (no jitter under a
    // keystroke storm) — they only re-derive when the lint pass refreshes.
    if (tr.docChanged) decos = decos.map(tr.changes);
    for (const effect of tr.effects) {
      if (effect.is(refreshDecorations)) {
        const built = buildDecorations(tr.state);
        if (built) decos = built; // null == stale cache: keep the mapped set
      }
    }
    return decos;
  },
  provide: (f) => EditorView.decorations.from(f),
});

// --- Theme (editor chrome) ----------------------------------------------------
// Recolor lint's wavy underline to the calm theme's red (--red: #f2637f)
// instead of its default bright #f11. Mirrors @codemirror/lint's own helper.
const wavyUnderline = (color) =>
  `url('data:image/svg+xml,<svg xmlns="http://www.w3.org/2000/svg" width="6" height="3">` +
  encodeURIComponent(
    `<path d="m0 2.5 l2 -1.5 l1 0 l2 1.5 l1 0" stroke="${color}" fill="none" stroke-width=".7"/>`
  ) +
  `</svg>')`;

const mono = "'JetBrains Mono', 'Fira Code', ui-monospace, monospace";

const intelTheme = EditorView.theme({
  ".cm-lintRange-error": { backgroundImage: wavyUnderline("#f2637f") },
  // Hover: a small dark panel consistent with the editor chrome.
  ".cm-tooltip.cm-tooltip-hover": {
    backgroundColor: "#1e1833",
    border: "1px solid #2b2542",
    borderRadius: "5px",
  },
  ".cm-hover-type": {
    fontFamily: mono,
    fontSize: "12.5px",
    color: "#e9e6f2",
    padding: "3px 8px",
    whiteSpace: "pre",
  },
  // Inlay hints: dimmed, slightly smaller, non-intrusive.
  ".cm-inlay": {
    color: "#6c6685",
    fontSize: "0.85em",
    padding: "0 1px",
  },
  // Codelenses: a dimmed signature line above the def.
  ".cm-lens": {
    fontFamily: mono,
    fontSize: "0.82em",
    color: "#6c6685",
    fontStyle: "italic",
    lineHeight: "1.5",
  },
});

// Async setup: resolve to the full intel extension set, or [] on any failure so
// the editor degrades to no-analysis silently.
export const setupLangIntel = async () => {
  try {
    const mod = await import(PKG_URL);
    await mod.default(); // init the wasm
    analyzeFn = mod.functor_lang_analyze;
    hoverFn = mod.functor_lang_hover;
  } catch {
    console.info(
      "[lang-intel] language analysis unavailable (pkg not built) — editor runs without diagnostics"
    );
    return [];
  }
  return [
    linter(toDiagnostics, { delay: 300 }),
    hoverTypes,
    decorationField,
    initialRefresh,
    intelTheme,
  ];
};
