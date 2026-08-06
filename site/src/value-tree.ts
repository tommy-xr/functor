// An expandable, indented view of one decoded value — the thing to reach for
// when a one-line rendering is no longer the question being asked.
//
// WHY IT EXISTS. Every value surface in the sandbox renders a value the way a
// ROW renders it: one line, depth-capped, elided (wire-value.ts's budgets, the
// editor's inline previews). That is right for scanning — "which message is
// this?" — and useless for the next question, "what is actually IN it?", which
// is exactly when the reader is looking at a world snapshot. This module
// answers the second question without changing how the first is answered: the
// row keeps its compact line, and the tree opens underneath it.
//
// WHAT IT CAN HONESTLY SHOW. Only what the page actually has. The wire log's
// payloads are decoded CLIENT-side from the full JSON frame (wire-value.ts), so
// nothing is lost before this module sees them and every field expands. The
// paused inspector's trace carries structure for each execution's RESULT and
// text for everything else, so the executions rows open here and the binding
// sites do not; see the note at the end of this file.
//
// DOM-imperative on purpose: its two hosts (the pinned wire log, and the wire
// tab that will consume it next) are imperative DOM inside a rAF-adjacent
// panel, and a node's open/closed state is local to that node — there is no
// shared state worth lifting anywhere.

import { keyLabel, summarize, type WireValue } from "./wire-value.js";

/**
 * Children rendered before a big collection stops and offers the rest.
 *
 * A list here is a running game's own data: a snapshot can carry thousands of
 * entities, and building thousands of rows the instant a triangle is clicked is
 * a frame the panel does not have. The rest is one click away, and the count is
 * on the button, so nothing is hidden — only deferred.
 */
const MAX_CHILDREN = 100;

/**
 * Characters of a string shown on its own row before it becomes expandable.
 *
 * Deliberately the same 40 as the row budget (wire-value's `MAX_STRING`), and
 * a leaf renders its string WHOLE rather than through `summarize`: the two
 * together are what guarantee the tree never elides something it offers no way
 * to open. A larger budget here would leave a band of strings (41–72 chars)
 * that the summary shortens and the tree does not expand — an ellipsis with
 * nothing behind it, which is the exact failure this component exists to end.
 */
const MAX_INLINE_STRING = 40;

/** One labelled child of a composite value. */
interface Child {
  label: string;
  value: WireValue;
}

/**
 * The children of a value, or null when it is a leaf.
 *
 * A single-argument variant (`Steer(intent)` — the common protocol shape)
 * delegates to its argument, so `Steer {turn: 1}` expands straight to the
 * record's fields instead of to a lone `0:` wrapper. Its summary already reads
 * that way (wire-value renders it braced), so the tree matches the line it
 * opens from. The delegation is deliberately wider than the summary's bracing
 * (which is Record-only): a `Steer([a, b])` still has nothing to say about its
 * lone argument, and a wrapper row that only ever holds one child is a level of
 * indentation charged for nothing.
 */
const childrenOf = (value: WireValue): Child[] | null => {
  if (typeof value !== "object" || value === null) return null;
  if ("Record" in value && Array.isArray(value.Record)) {
    return value.Record.map((entry) => ({ label: String(entry?.[0]), value: entry?.[1] }));
  }
  if ("Map" in value && Array.isArray(value.Map)) {
    return value.Map.map((entry) => ({ label: keyLabel(entry?.[0]), value: entry?.[1] }));
  }
  if ("List" in value && Array.isArray(value.List)) {
    return value.List.map((item, index) => ({ label: String(index), value: item }));
  }
  if ("Tuple" in value && Array.isArray(value.Tuple)) {
    return value.Tuple.map((item, index) => ({ label: String(index), value: item }));
  }
  if ("Variant" in value && Array.isArray(value.Variant)) {
    const args = value.Variant[1];
    if (!Array.isArray(args) || args.length === 0) return null;
    if (args.length === 1) return childrenOf(args[0]);
    return args.map((arg, index) => ({ label: String(index), value: arg }));
  }
  return null;
};

/** The ink a scalar carries: the same three-way reading the wire rows use
 * (data lit, text in the accent, everything structural dim). */
const scalarClass = (value: WireValue): string => {
  if (typeof value !== "object" || value === null) return "";
  if ("Number" in value) return " num";
  if ("Bool" in value) return " bool";
  if ("Text" in value) return " str";
  return "";
};

/** The whole string a `Text` node holds, or null for anything else. */
const textOf = (value: WireValue): string | null =>
  typeof value === "object" && value !== null && "Text" in value && typeof value.Text === "string"
    ? value.Text
    : null;

const span = (className: string, text: string): HTMLSpanElement => {
  const el = document.createElement("span");
  el.className = className;
  // textContent, never innerHTML: this is a running game's own data.
  el.textContent = text;
  return el;
};

/**
 * One node: its row, and (lazily) its children.
 *
 * Children are built on FIRST expand and then kept, so re-opening a node is
 * free and a closed node costs nothing — which is what makes a tree over a
 * world snapshot affordable at all. Closing hides rather than discards: the
 * reader who closes a node is usually about to open it again.
 */
const node = (
  label: string,
  value: WireValue,
  depth: number,
  onToggle: () => void
): HTMLElement => {
  const el = document.createElement("div");
  el.className = "mp-vt-node";
  const children = childrenOf(value);
  const text = textOf(value);
  const longText = text !== null && text.length > MAX_INLINE_STRING;
  // A leaf string is rendered whole (`summarize` would elide it at the same
  // budget that decides `longText`, and a leaf has no disclosure to offer).
  const summary = text !== null && !longText ? JSON.stringify(text) : summarize(value);

  if (!children && !longText) {
    const row = document.createElement("div");
    row.className = "mp-vt-row";
    row.append(span("mp-vt-tw", ""));
    if (label) row.append(span("mp-vt-k", label));
    row.append(span(`mp-vt-v${scalarClass(value)}`, summary));
    el.append(row);
    return el;
  }

  const row = document.createElement("button");
  row.type = "button";
  row.className = "mp-vt-row";
  row.setAttribute("aria-expanded", "false");
  const twisty = span("mp-vt-tw", "▸");
  row.append(twisty);
  if (label) row.append(span("mp-vt-k", label));
  row.append(span(`mp-vt-v${scalarClass(value)}`, summary));
  const kids = document.createElement("div");
  kids.className = "mp-vt-kids";
  kids.hidden = true;
  el.append(row, kids);

  let built = false;
  const build = () => {
    if (built) return;
    built = true;
    if (longText && text !== null) {
      kids.append(span("mp-vt-text", text));
      return;
    }
    const all = children ?? [];
    // A CHUNK at a time, always — including behind the "more" button. A list
    // here is game data, so "the rest" can be a hundred thousand entries: both
    // building that many nodes and spreading them as arguments are ways to take
    // the page down, and neither is something a reader asked for.
    const more = document.createElement("button");
    more.type = "button";
    more.className = "mp-vt-more";
    let shown = 0;
    const showChunk = () => {
      const batch = document.createDocumentFragment();
      for (const child of all.slice(shown, shown + MAX_CHILDREN)) {
        batch.append(node(child.label, child.value, depth + 1, onToggle));
      }
      shown = Math.min(shown + MAX_CHILDREN, all.length);
      kids.insertBefore(batch, more);
      if (shown >= all.length) more.remove();
      else more.textContent = `… ${all.length - shown} more`;
    };
    kids.append(more);
    showChunk();
    more.addEventListener("click", () => {
      showChunk();
      onToggle();
    });
  };

  row.addEventListener("click", () => {
    const open = row.getAttribute("aria-expanded") !== "true";
    if (open) build();
    row.setAttribute("aria-expanded", String(open));
    twisty.textContent = open ? "▾" : "▸";
    kids.hidden = !open;
    // The tree just changed height, and its host may be positioned against it.
    onToggle();
  });
  return el;
};

/** Is there anything to open? False for a scalar and for a nullary variant —
 * values whose whole content is already the one line a row shows, and which
 * should therefore not offer a disclosure that reveals nothing. */
export const hasTree = (value: WireValue): boolean =>
  childrenOf(value) !== null || (textOf(value)?.length ?? 0) > MAX_INLINE_STRING;

/**
 * Render `value` as an expandable tree inside `host`, replacing whatever was
 * there. The root opens expanded: the reader asked for this by opening it.
 *
 * `onToggle` fires after any change to the tree's SIZE (a node opening or
 * closing, a chunk of a long list arriving). A host that positions itself
 * against its own height — the wire log's panel does — re-places from it.
 */
export const mountValueTree = (
  host: HTMLElement,
  value: WireValue,
  onToggle: () => void = () => {}
): void => {
  const root = document.createElement("div");
  root.className = "mp-vt";
  const tree = node("", value, 0, onToggle);
  root.append(tree);
  host.replaceChildren(root);
  // Open the root by clicking its own row, so "expanded" has exactly one
  // implementation and the aria state can never disagree with the DOM.
  tree.querySelector<HTMLButtonElement>(":scope > button.mp-vt-row")?.click();
};

// --- What the trace can and cannot open, and why ------------------------------
//
// The paused inspector's trace used to carry its values as PRE-RENDERED TEXT
// only, which is why this module shipped without it. Each invocation now also
// carries its RESULT as structure (`result_json` — the runtime's
// `value_to_json`, converted by `fromValueJson`), built where the replayed
// value is already live, so the executions rows expand here like wire rows do.
//
// Individual BINDING sites still arrive as text. Their values exist only inside
// the interpreter's recorder (`RecordedBinding` in functor-lang/src/eval.rs
// "owns no `Value`s"), and every site that observes the model would carry its
// own copy of it — the doc would grow by the site count for a surface whose
// full `value` string is already untruncated (only `preview` elides, and the
// editor's hover shows the whole thing). Parsing that `Display` text into a
// tree here is still refused: it would be a re-parser for a rendering never
// specified as a format — wrong on strings containing `, ` and plausible when
// wrong.
