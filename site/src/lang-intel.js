// Live language intelligence for the sandbox and IDE editors: loads the small
// functor-lang analysis wasm (built by `npm run build:lang-wasm`, copied to
// dist/pkg/ by build.mjs) and turns its type diagnostics into CodeMirror lint
// underlines, plus hover types, inlay hints, and signature codelenses. The
// sandbox analyzes its single buffer; the IDE registers a context provider
// (setLangContext) so every pass runs over the whole file set with sibling
// modules resolved.
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

import { forceLinting, linter } from "@codemirror/lint";
import { Decoration, EditorView, ViewPlugin, WidgetType, hoverTooltip } from "@codemirror/view";
import { StateEffect, StateField } from "@codemirror/state";
import { functorLangLanguage } from "./functor-lang.js";

// The runtime import specifier resolves to the glue esbuild leaves external
// (see build.mjs `external`), copied to /pkg/ at build time — NOT bundled.
const PKG_URL = "/pkg/functor_lang_wasm.js";

let analyzeFn = null; // (src) => JSON string, set once the wasm is ready
let hoverFn = null; // (src, offset) => JSON string ("" when nothing to show)
let completeFn = null; // (src, offset) => JSON string, set once the wasm is ready
let analyzeProjectFn = null; // (filesJson, active) => JSON string
let hoverProjectFn = null; // (filesJson, active, offset) => JSON string
let completeProjectFn = null; // (filesJson, active, offset) => JSON string
let resetFn = null; // () => void, clears the wasm completion cache
let lastKey = null;
let lastResult = null;

// The multi-file seam (the IDE): the host registers a provider returning the
// whole file set (`[{ path, source }]`, entry first) plus the active path;
// every analysis then runs project-wide — sibling modules resolve — with the
// active file's source swapped for the live editor doc. Null (the sandbox
// default) keeps the single-file calls.
let contextFn = null;

export const setLangContext = (fn) => {
  contextFn = fn;
};

// The `*_project` call args for the live `docString`, or null in single-file
// mode (no provider, or a provider mid-teardown returning junk).
const projectArgs = (docString) => {
  if (!contextFn) return null;
  const { active, files } = contextFn() ?? {};
  if (!active || !Array.isArray(files)) return null;
  const withLive = files.map((f) => ({
    path: f.path,
    source: f.path === active ? docString : f.source,
  }));
  return { filesJson: JSON.stringify(withLive), active };
};

// The memo key covers everything an analysis depends on: in project mode the
// serialized file set + active path (so a file switch or sibling edit is a
// fresh analysis), in single-file mode just the doc.
const cacheKey = (docString, args) => (args ? `${args.active}\x00${args.filesJson}` : docString);

// Clear the wasm completion last-good cache — called by the sandbox when the
// editor document is wholly replaced (example switch, inline load, reset) so
// stale candidates from the previous program can't leak into the new one. A
// no-op when analysis is degraded/unavailable.
export const resetIntel = () => {
  if (resetFn) resetFn();
};

// Run analyze at most once per distinct (doc, context) key. Returns the parsed
// `{ diagnostics, inlays, lenses }`, or null when the wasm isn't loaded.
export const analyzeCached = (docString) => {
  if (!analyzeFn) return null;
  const args = projectArgs(docString);
  const key = cacheKey(docString, args);
  if (key === lastKey) return lastResult;
  let result;
  try {
    result = JSON.parse(
      args ? analyzeProjectFn(args.filesJson, args.active) : analyzeFn(docString)
    );
  } catch {
    result = { diagnostics: [], inlays: [], lenses: [] };
  }
  lastKey = key;
  lastResult = result;
  return result;
};

// Raw completion at a UTF-16 offset — the test/introspection seam
// (window.__lang.complete). Returns the parsed `{ items }`, or null when the
// wasm isn't loaded.
export const completeAt = (src, offset) => {
  if (!completeFn) return null;
  try {
    return JSON.parse(completeFn(src, offset));
  } catch {
    return null;
  }
};

// Read the memoized result WITHOUT running analyze: returns it only when it is
// already current for `docString`, else null. The decoration field uses this so
// it never triggers an analyze of its own (the lint source is the sole caller
// that fills the cache).
const peekCached = (docString) =>
  analyzeFn && cacheKey(docString, projectArgs(docString)) === lastKey ? lastResult : null;

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
    const doc = view.state.doc.toString();
    const args = projectArgs(doc);
    info = JSON.parse(
      (args ? hoverProjectFn(args.filesJson, args.active, pos) : hoverFn(doc, pos)) || ""
    );
  } catch {
    return null;
  }
  if (!info || !info.text) return null;
  // Clamp to the current doc, exactly like toDiagnostics/buildDecorations: the
  // wasm offsets can lag the live buffer by a keystroke.
  const len = view.state.doc.length;
  const from = Math.max(0, Math.min(info.from | 0, len));
  const to = Math.max(from, Math.min(info.to | 0, len));
  return {
    pos: from,
    end: to,
    create: () => {
      const dom = document.createElement("div");
      dom.className = "cm-hover-type";
      dom.textContent = info.text;
      return { dom };
    },
  };
});

// --- Autocomplete -------------------------------------------------------------
// A CodeMirror completion source backed by the wasm's scope-aware `complete`.
// Registered via the language's `data` facet (below), so basicSetup's
// `autocompletion()` picks it up through `languageDataAt("autocomplete")` — no
// second popup, and in degraded mode (no wasm) it is simply never registered.

// Map the wasm's completion kind to a CodeMirror `type`, so the built-in icons
// render (function/variable/namespace/keyword/enum/property all have glyphs).
const KIND_TO_TYPE = {
  function: "function",
  value: "variable",
  module: "namespace",
  keyword: "keyword",
  constructor: "enum",
  field: "property",
};

// Fires on explicit trigger (Ctrl-Space), after a `.`, or while typing a word.
// Returns `validFor` so CodeMirror filters the list client-side as more word
// chars are typed — one wasm call per token, not per keystroke.
const functorCompletions = (context) => {
  if (!completeFn) return null;
  const word = context.matchBefore(/[A-Za-z_]\w*/);
  const afterDot = context.pos > 0 && context.state.sliceDoc(context.pos - 1, context.pos) === ".";
  if (!context.explicit && !afterDot && !word) return null;
  let items;
  try {
    const doc = context.state.doc.toString();
    const args = projectArgs(doc);
    items = JSON.parse(
      args
        ? completeProjectFn(args.filesJson, args.active, context.pos)
        : completeFn(doc, context.pos)
    ).items;
  } catch {
    return null;
  }
  if (!Array.isArray(items) || items.length === 0) return null;
  return {
    from: word ? word.from : context.pos,
    options: items.map((it) => ({
      label: it.label,
      type: KIND_TO_TYPE[it.kind],
      detail: it.detail || undefined,
    })),
    validFor: /^\w*$/,
  };
};

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

// A signal to drop all decorations NOW (file switch / topology change): the
// current set belongs to the outgoing context, and the keep-on-stale rule
// below would otherwise show it against the incoming doc until the debounced
// lint pass lands.
const clearDecorations = StateEffect.define();

// The editor now shows a different document or file-set shape (IDE file
// switch, file create/delete): drop the outgoing decorations immediately and
// force a fresh lint pass — a topology change alone doesn't change the doc,
// so the linter would otherwise never rerun. The effect trips the linter's
// `needsRefresh` (forceLinting alone only flushes an already-pending query);
// safe before setup resolves (the effect is simply unhandled without it).
export const refreshIntel = (view) => {
  view.dispatch({ effects: clearDecorations.of(null) });
  forceLinting(view);
};

// True when a transaction carries the clear signal — the linter must re-query
// even though the doc is unchanged (the analysis context changed under it).
const contextChanged = (update) =>
  update.transactions.some((tr) => tr.effects.some((e) => e.is(clearDecorations)));

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
      if (effect.is(clearDecorations)) decos = Decoration.none;
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
  // Autocomplete popup: the same calm dark panel as the hover tooltip.
  ".cm-tooltip.cm-tooltip-autocomplete": {
    backgroundColor: "#1e1833",
    border: "1px solid #2b2542",
    borderRadius: "5px",
  },
  ".cm-tooltip-autocomplete > ul": {
    fontFamily: mono,
    fontSize: "12.5px",
    maxHeight: "16em",
  },
  ".cm-tooltip-autocomplete > ul > li": {
    color: "#e9e6f2",
    padding: "2px 6px",
  },
  ".cm-tooltip-autocomplete ul li[aria-selected]": {
    backgroundColor: "#0e3b46",
    color: "#c7f2f7",
  },
  // The detail (type signature) trailing each option, dimmed.
  ".cm-completionDetail": {
    color: "#6c6685",
    fontStyle: "italic",
  },
  // The kind icon, tinted to the cyan accent so glyphs read on the dark panel.
  ".cm-completionIcon": {
    color: "#9b94b3",
    opacity: "0.9",
    marginRight: "0.4em",
  },
});

// Async setup: resolve to the full intel extension set, or [] on any failure so
// the editor degrades to no-analysis silently.
export const setupLangIntel = async () => {
  try {
    const mod = await import(PKG_URL);
    await mod.default(); // init the wasm
    // A partial/mismatched bundle (missing an expected export) degrades fully
    // rather than installing half the intel — the catch below is the one seam.
    if (
      typeof mod.functor_lang_analyze !== "function" ||
      typeof mod.functor_lang_hover !== "function" ||
      typeof mod.functor_lang_complete !== "function" ||
      typeof mod.functor_lang_analyze_project !== "function" ||
      typeof mod.functor_lang_hover_project !== "function" ||
      typeof mod.functor_lang_complete_project !== "function"
    ) {
      throw new Error("functor-lang-wasm is missing an expected export");
    }
    analyzeFn = mod.functor_lang_analyze;
    hoverFn = mod.functor_lang_hover;
    completeFn = mod.functor_lang_complete;
    analyzeProjectFn = mod.functor_lang_analyze_project;
    hoverProjectFn = mod.functor_lang_hover_project;
    completeProjectFn = mod.functor_lang_complete_project;
    // reset is optional — an older bundle without it just skips cache clearing.
    resetFn = typeof mod.functor_lang_reset === "function" ? mod.functor_lang_reset : null;
  } catch {
    console.info(
      "[lang-intel] language analysis unavailable (pkg not built) — editor runs without diagnostics"
    );
    return [];
  }
  return [
    linter(toDiagnostics, { delay: 300, needsRefresh: contextChanged }),
    hoverTypes,
    decorationField,
    initialRefresh,
    // Register the completion source on the language's data facet — basicSetup's
    // autocompletion() picks it up via languageDataAt (no second popup).
    functorLangLanguage.data.of({ autocomplete: functorCompletions }),
    intelTheme,
  ];
};
