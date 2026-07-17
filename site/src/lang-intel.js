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

// The host page can observe each lint pass's diagnostics (the status bar's
// Problems panel) — called with the same clamped list the linter gets.
let diagnosticsListener = null;
export const onDiagnostics = (fn) => {
  diagnosticsListener = fn;
};

// analyze offsets are whole-document UTF-16 — CodeMirror's native unit — so
// they map straight across; clamp defensively to the current doc length.
const toDiagnostics = (view) => {
  const doc = view.state.doc;
  const result = analyzeCached(doc.toString());
  // The lint pass just refreshed the cache for this doc; nudge the decoration
  // field to re-read it (the initial doc lands here too, so inlays/lenses show
  // on load — not only after the first edit).
  scheduleRefresh(view);
  const len = doc.length;
  const diagnostics =
    !result || !Array.isArray(result.diagnostics)
      ? []
      : result.diagnostics.map((d) => {
          const from = Math.max(0, Math.min(d.from | 0, len));
          const to = Math.max(from, Math.min(d.to | 0, len));
          return { from, to, severity: d.severity || "error", message: d.message || "" };
        });
  if (diagnosticsListener) diagnosticsListener(diagnostics);
  return diagnostics;
};

// --- Hover types --------------------------------------------------------------
// Ask the wasm for the type under the cursor (UTF-16 offset == CodeMirror pos)
// and render it monospace in a small calm-theme tooltip.
const hoverTypes = hoverTooltip((view, pos) => {
  if (!hoverFn) return null;
  // The live value first (the paused inspector's inline-vs-hover policy:
  // previews render inline, the FULL value lives here). ALL recorded sites
  // answer — including reads the inline dedup suppressed. Half-open bounds —
  // the character AT nameEnd is the operator/space after the name.
  const hit = liveSites.find((s) => pos >= s.nameStart && pos < s.nameEnd);
  const live = hit ? { name: hit.b.name, value: hit.b.value, count: hit.b.count || 1, nameStart: hit.nameStart, nameEnd: hit.nameEnd } : null;
  let info;
  try {
    const doc = view.state.doc.toString();
    const args = projectArgs(doc);
    info = JSON.parse(
      (args ? hoverProjectFn(args.filesJson, args.active, pos) : hoverFn(doc, pos)) || ""
    );
  } catch {
    info = null;
  }
  if (!live && (!info || !info.text)) return null;
  // Clamp to the current doc, exactly like toDiagnostics/buildDecorations: the
  // wasm offsets can lag the live buffer by a keystroke.
  const len = view.state.doc.length;
  const from = live
    ? live.nameStart
    : Math.max(0, Math.min(info.from | 0, len));
  const to = live ? live.nameEnd : Math.max(from, Math.min(info.to | 0, len));
  return {
    pos: from,
    end: to,
    create: () => {
      const dom = document.createElement("div");
      dom.className = "cm-hover-type";
      if (live) {
        const line = document.createElement("div");
        line.className = "cm-hover-live";
        line.textContent =
          live.count > 1
            ? `${live.name} = ${live.value} (×${live.count})`
            : `${live.name} = ${live.value}`;
        dom.appendChild(line);
      }
      if (info && info.text) {
        const line = document.createElement("div");
        line.textContent = info.text;
        dom.appendChild(line);
      }
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

// --- Live values (paused-inspector overlay) ------------------------------------
// The player relays the paused trace (`functor-inspector-trace` postMessage,
// see site/player.html); the host page hands it to `setLiveTrace`. While the
// game is paused and the buffer matches the trace's source hash, every
// recorded site renders an inline `= preview` next to its name — binders and
// variable reads — and hovering a name shows the FULL value. Any edit clears
// the overlay instantly (the hash no longer matches; the IDE's push loop
// delivers a fresh trace after the hot reload).

class LiveValueWidget extends WidgetType {
  constructor(label) {
    super();
    this.label = label;
  }
  eq(other) {
    return other.label === this.label;
  }
  toDOM() {
    const span = document.createElement("span");
    span.className = "cm-live-value";
    span.textContent = this.label;
    return span;
  }
  ignoreEvent() {
    return true;
  }
}

const setLiveOverlay = StateEffect.define();

// The current overlay's hint data, kept in lockstep with the decoration
// field below — plus ALL recorded sites for the buffer (hover searches every
// site, not just the per-line dedup winners: the second `x` in `x + x` still
// hovers to its value).
let liveHints = [];
let liveSites = [];

const liveField = StateField.define({
  create: () => Decoration.none,
  update(value, tr) {
    // Any edit invalidates the overlay wholesale — the trace's source hash no
    // longer matches, and honest absence beats drifted values. A fresh trace
    // re-populates after the reload round-trips.
    if (tr.docChanged) {
      liveHints = [];
      liveSites = [];
      return Decoration.none;
    }
    for (const effect of tr.effects) {
      if (effect.is(setLiveOverlay)) {
        liveHints = effect.value;
        return Decoration.set(
          effect.value.map((h) =>
            Decoration.widget({ widget: new LiveValueWidget(h.label), side: 1 }).range(h.offset)
          ),
          true
        );
      }
      if (effect.is(clearDecorations)) {
        liveHints = [];
        liveSites = [];
        return Decoration.none;
      }
    }
    return value;
  },
  provide: (f) => EditorView.decorations.from(f),
});

// The trace + selection state, module-level like the analysis caches.
let liveTrace = null; // the last paused trace doc (null while playing)
let selectedExec = new Map(); // entry name → selected execution index
let liveRefreshToken = 0; // supersedes in-flight async hash checks

// The 0-based line of `offset` in `source`.
const lineOf = (source, offset) => {
  let line = 0;
  for (let i = 0; i < offset && i < source.length; i++) {
    if (source.charCodeAt(i) === 10) line += 1;
  }
  return line;
};

// Trace spans are UTF-8 BYTE offsets into the file text; JS strings and
// CodeMirror positions count UTF-16 code units. Returns a byte→UTF-16 lookup
// array, or null for pure-ASCII sources (identity — the common case).
const byteToUtf16 = (source) => {
  let ascii = true;
  for (let i = 0; i < source.length; i++) {
    if (source.charCodeAt(i) > 127) {
      ascii = false;
      break;
    }
  }
  if (ascii) return null;
  const map = [];
  let bytes = 0;
  let units = 0;
  for (const ch of source) {
    const cp = ch.codePointAt(0);
    const width = cp <= 0x7f ? 1 : cp <= 0x7ff ? 2 : cp <= 0xffff ? 3 : 4;
    for (let k = 0; k < width; k++) map[bytes + k] = units;
    bytes += width;
    units += ch.length;
  }
  map[bytes] = units;
  return map;
};

// Locate the binder name inside its recorded span (the LSP's rule): spans are
// name-precise except `let [mut] name =` regions, where we scan FORWARD past
// the keywords (a type annotation or comment inside the region can contain
// the name too). Returns the offset just after the name, or span end.
const hintOffset = (source, b) => {
  const region = source.slice(b.start, b.end);
  const atName = (i) => {
    if (!region.startsWith(b.name, i)) return false;
    const next = region[i + b.name.length];
    return !(next && /[A-Za-z0-9_]/.test(next));
  };
  if (atName(0)) return b.start + b.name.length;
  let i = 0;
  for (const keyword of ["let", "mut"]) {
    if (region.startsWith(keyword, i)) {
      i += keyword.length;
      while (i < region.length && /\s/.test(region[i])) i += 1;
      if (atName(i)) return b.start + i + b.name.length;
    }
  }
  const found = region.lastIndexOf(b.name);
  return found >= 0 ? b.start + found + b.name.length : b.end;
};

// The wire's file name for the buffer being edited: the active path in
// project mode (the IDE), else the trace's single user file (the sandbox
// serves examples under their own names — `hero.fun`, not `game.fun`).
// SINGLE-FILE ASSUMPTION: a sandbox program that ever loads sibling modules
// would yield multiple sources here and the overlay stays hidden — the
// sandbox is a one-buffer editor by design, so there is nothing to overlay
// the siblings on anyway.
const liveFileName = (docString) => {
  const args = projectArgs(docString);
  if (args) return args.active;
  const sources = liveTrace?.sources || [];
  return sources.length === 1 ? sources[0].file : null;
};

// The selected executions' recorded sites for `fileName`, with spans
// CONVERTED to UTF-16 (`byteToUtf16`) and name-located (`hintOffset`) —
// `[{ b, offset, nameStart, nameEnd }]` for every site (hover searches all
// of them; the inline dedup happens in liveHintsFor).
const liveSitesFor = (fileName, source) => {
  const conv = byteToUtf16(source);
  const invs = (liveTrace.invocations || []).filter((i) => !i.ghost);
  const sites = [];
  for (const entry of new Set(invs.map((i) => i.entry))) {
    const group = invs.filter((i) => i.entry === entry);
    const count = Math.max(1, ...group.map((i) => i.count || 1));
    const sel = (selectedExec.get(entry) || 0) % count;
    const inv = group.find((i) => i.index === sel) || group[0];
    for (const b of inv.bindings || []) {
      if (b.file !== fileName) continue;
      const start = conv ? conv[b.start] ?? source.length : b.start;
      const end = conv ? conv[b.end] ?? source.length : b.end;
      const offset = hintOffset(source, { ...b, start, end });
      sites.push({ b, offset, nameStart: offset - b.name.length, nameEnd: offset });
    }
  }
  return sites;
};

// Compute the overlay hints for the CURRENT buffer from the selected
// execution of each entry — the LSP's policy, ported: previews inline, one
// hint per (line, name), a binder site beats a reference, earliest read wins.
const liveHintsFor = (sites, source) => {
  const chosen = new Map(); // `${line}\x00${name}` → { b, offset, … }
  for (const site of sites) {
    const key = `${lineOf(source, site.offset)}\x00${site.b.name}`;
    const cur = chosen.get(key);
    const wins =
      !cur ||
      (site.b.site === "binder" && cur.b.site === "ref") ||
      (!(site.b.site === "ref" && cur.b.site === "binder") && site.offset < cur.offset);
    if (wins) chosen.set(key, site);
  }
  return [...chosen.values()]
    .map(({ b, offset, nameStart, nameEnd }) => {
      const preview = shortNumbers(b.preview ?? b.value);
      return {
        offset,
        label: b.count > 1 ? `= ${preview} (×${b.count})` : `= ${preview}`,
        name: b.name,
        value: b.value,
        count: b.count || 1,
        nameStart,
        nameEnd,
      };
    })
    .sort((a, z) => a.offset - z.offset);
};

// Inline labels shorten long floats (0.15000000782310963 → 0.15) — full-
// precision noise makes dense lines unreadable; hover keeps the exact value.
// Quoted segments pass through untouched: a STRING value containing
// number-like text ("lat: 37.7749295") must never be rewritten.
const shortNumbers = (text) =>
  String(text ?? "")
    .split(/("(?:[^"\\]|\\.)*")/)
    .map((part, i) =>
      i % 2
        ? part
        : part.replace(/-?\d+\.\d{6,}(?:e-?\d+)?/g, (m) => String(Number(Number(m).toPrecision(5))))
    )
    .join("");

const sha256Hex = async (text) => {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text));
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
};

// Rebuild the overlay for the current buffer: async (the hash check), token-
// guarded so a stale completion can't clobber a newer state.
const refreshLive = async (view) => {
  const token = ++liveRefreshToken;
  const clear = () => view.dispatch({ effects: setLiveOverlay.of([]) });
  if (!liveTrace || !liveTrace.paused) {
    clear();
    return;
  }
  const source = view.state.doc.toString();
  const fileName = liveFileName(source);
  const traceHash = (liveTrace.sources || []).find((s) => s.file === fileName)?.hash;
  if (!fileName || !traceHash) {
    clear();
    return;
  }
  const hash = await sha256Hex(source);
  if (token !== liveRefreshToken) return; // superseded
  if (hash !== traceHash || source !== view.state.doc.toString()) {
    clear();
    return;
  }
  const sites = liveSitesFor(fileName, source);
  liveSites = sites;
  view.dispatch({ effects: setLiveOverlay.of(liveHintsFor(sites, source)) });
};

// Host seams -------------------------------------------------------------------

// A new trace arrived (or the game resumed: pass an unpaused doc/null).
// Resets the execution selection — the frame changed under it.
export const setLiveTrace = (view, trace) => {
  liveTrace = trace && trace.paused ? trace : null;
  selectedExec = new Map();
  refreshLive(view);
};

// The current overlay's hints — the e2e position-invariant seam (every
// hint's [nameStart, nameEnd) must slice to exactly its name in the doc,
// which fails loudly if byte offsets ever leak through unconverted).
export const currentLiveHints = () => liveHints;

// The frame's executions in order, for the host's picker UI:
// `[{ entry, index, count, provenance, selected }]`. Empty while playing.
export const liveExecutions = () => {
  if (!liveTrace) return [];
  return (liveTrace.invocations || [])
    .filter((i) => !i.ghost)
    .map((i) => ({
      entry: i.entry,
      index: i.index || 0,
      count: i.count || 1,
      provenance: i.provenance || "",
      selected: ((selectedExec.get(i.entry) || 0) % (i.count || 1)) === (i.index || 0),
    }));
};

// Select which execution of `entry` overlays (the picker's click).
export const selectExecution = (view, entry, index) => {
  selectedExec.set(entry, index);
  refreshLive(view);
};

// Re-verify the overlay after a context change the doc didn't see (the IDE's
// file switch re-uses setDoc, whose doc change already clears; this is for
// hosts that need an explicit nudge, e.g. after a reload result).
export const refreshLiveValues = (view) => refreshLive(view);

// The whole host-side wiring, shared by the sandbox and the IDE: listen for
// the player's `functor-inspector-trace` relay (guarded to OUR iframe),
// hand traces to the overlay, and keep the status bar's executions picker in
// sync (rows select which execution's values render).
export const wireLiveTrace = (view, statusBar, playerIframe, ready) => {
  const renderExecutions = () => {
    statusBar.setExecutions(
      liveExecutions().map((e) => ({
        label: `${e.entry} ${e.index + 1}/${e.count} — ${e.provenance}`,
        selected: e.selected,
        onPick: () => {
          selectExecution(view, e.entry, e.index);
          renderExecutions();
        },
      }))
    );
  };
  window.addEventListener("message", (event) => {
    const data = event.data;
    if (!data || data.type !== "functor-inspector-trace") return;
    if (event.source !== playerIframe.contentWindow) return;
    setLiveTrace(view, data.trace);
    renderExecutions();
  });
  // A trace can beat the async extension install (the runtime pauses before
  // setupLangIntel resolves): the overlay effect would land on an
  // unconfigured field and the generation never re-fires for an unchanged
  // doc. Replay once the extensions are in.
  if (ready) ready.then(() => refreshLive(view));
};

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
  // Live values (the paused inspector): cyan-tinted so runtime data reads
  // apart from the static type inlays.
  ".cm-live-value": {
    color: "#41d8e6",
    fontSize: "0.85em",
    padding: "0 2px",
    opacity: "0.85",
  },
  ".cm-hover-live": {
    color: "#41d8e6",
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
    liveField,
    initialRefresh,
    // Register the completion source on the language's data facet — basicSetup's
    // autocompletion() picks it up via languageDataAt (no second popup).
    functorLangLanguage.data.of({ autocomplete: functorCompletions }),
    intelTheme,
  ];
};
