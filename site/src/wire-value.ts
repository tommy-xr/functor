// Decoding a packet's wire TEXT back into the value the game actually sent —
// what makes the wire log readable as `Steer {turn:1, thrust:true}` instead of
// as a byte count (design addendum 5, rule 3: "rows are decoded ADT values,
// never hex").
//
// WHAT IS ACTUALLY ON THE WIRE. `Effect.sendMsg(conn, msg)` frames its payload
// as a control-character prefix — U+0001 then `fun:`, never produced by
// ordinary game text — followed by the payload as JSON
// (functor_runtime_common::functor_lang_prelude, `TYPED_MSG_PREFIX` /
// `encode_typed_msg`). The JSON is serde's default representation of
// `EffectValue`, the plain-data subset of a Functor Lang value: externally
// tagged, so a variant is `{"Variant":["Steer",[…args…]]}`, a record is
// `{"Record":[["turn",{"Number":1}]]}`, and so on. Plain `Effect.send(conn,
// text)` carries the text unframed, and the runtime passes it through
// untouched — so a connection can carry both, and this module reports which.
//
// Pure and DOM-free on purpose: this is the part worth reasoning about, and it
// is the part a reader will not want to open a browser to check.

/** The serde shape of `functor_runtime_common::functor_lang_prelude::EffectValue`. */
export type WireValue =
  | { Number: number }
  | { Bool: boolean }
  | { Text: string }
  | { List: WireValue[] }
  | { Map: [WireKey, WireValue][] }
  | { Tuple: WireValue[] }
  | { Record: [string, WireValue][] }
  | { Variant: [string, WireValue[]] };

type WireKey = { Bool: boolean } | { Number: number } | { Text: string };

/** The framing `Effect.sendMsg` puts in front of a typed payload
 * (`TYPED_MSG_PREFIX` on the Rust side). */
const TYPED_MSG_PREFIX = "\u0001fun:";

/**
 * How deep a rendered value goes before it collapses to `{…}` / `[…]`.
 *
 * A wire log row is one line in a pinned panel, and the reader is scanning for
 * WHICH message this is, not auditing its contents — the constructor plus a
 * glance at the top fields answers that. The unelided rendering is always one
 * hover away (`fullWire`).
 *
 * The elision RULES are this module's own, matching the approved mockup's
 * `Move {dx:1}` rather than the Rust `Value::preview`'s spacier
 * `Move({ dx: 1 })` (functor-lang/src/value.rs): a wire row is a dense column
 * beside a game, not an editor inlay. The budgets are borrowed from it.
 */
const MAX_DEPTH = 2;

/** Items shown before a list/record elides the rest. */
const MAX_ITEMS = 4;

/** Characters of a string shown in a row before it elides (the Rust preview's
 * `MAX_PREVIEW_STRING`, functor-lang/src/value.rs — same job, same budget). */
const MAX_STRING = 40;

/**
 * Characters of the UNELIDED rendering kept for the row's tooltip.
 *
 * The payload is a running game's own data and can be any size; a title
 * attribute is not the place to put a megabyte of it.
 */
const MAX_FULL = 2000;

/** One decoded row, split into the two things a row shows. */
export interface DecodedWire {
  /** The constructor name — `Steer`, `Snapshot` — or "" for a payload that is
   * not a variant (plain text, a bare record). */
  head: string;
  /** The arguments, compact and depth-capped. */
  body: string;
  /** False for a plain `Effect.send` text payload, or a frame that would not
   * parse — the row then shows the raw text and says so. */
  typed: boolean;
}

/** A number as a game author wrote it: whole numbers without their `.0`, and
 * five significant digits otherwise — the precision the editor's inline values
 * and the Rust `Value::preview` already use, so one value reads the same
 * wherever the tooling shows it. */
const num = (n: number): string =>
  typeof n !== "number" || Number.isInteger(n) ? String(n) : String(Number(n.toPrecision(5)));

const str = (value: string, full: boolean): string =>
  JSON.stringify(full || value.length <= MAX_STRING ? value : `${value.slice(0, MAX_STRING)}…`);

const keyOf = (key: WireKey): string => {
  if (typeof key !== "object" || key === null) return String(key);
  if ("Text" in key) return key.Text;
  if ("Number" in key) return num(key.Number);
  return String(key.Bool);
};

/**
 * Render a value. `full` drops every limit — that is the tooltip's text.
 *
 * TOTAL by construction: the input is untrusted (a game can `Effect.send` any
 * text at all, including a hand-written `fun:` frame), so anything that
 * is not a value this understands renders as its own JSON rather than throwing.
 * A decoder that can throw would take the host's rAF loop down with it.
 */
const render = (value: WireValue, depth: number, full: boolean): string => {
  // `JSON.stringify` answers `undefined` for `undefined` — the one input that
  // would put the string "undefined" (or worse, nothing) in a row.
  const asJson = (other: unknown) => JSON.stringify(other) ?? String(other);
  if (typeof value !== "object" || value === null) return asJson(value);
  const deep = !full && depth >= MAX_DEPTH;
  const take = <T,>(items: T[]) => (full ? items : items.slice(0, MAX_ITEMS));
  const more = (items: unknown[], mark: string) =>
    !full && items.length > MAX_ITEMS ? [mark] : [];
  if ("Number" in value) return num(value.Number);
  if ("Bool" in value) return String(value.Bool);
  if ("Text" in value) return str(value.Text, full);
  if ("List" in value && Array.isArray(value.List)) {
    if (deep) return value.List.length ? `[…${value.List.length}]` : "[]";
    const shown = take(value.List).map((item) => render(item, depth + 1, full));
    return `[${[...shown, ...more(value.List, `…${value.List.length}`)].join(", ")}]`;
  }
  if ("Tuple" in value && Array.isArray(value.Tuple)) {
    return `(${value.Tuple.map((item) => render(item, depth + 1, full)).join(", ")})`;
  }
  if ("Record" in value && Array.isArray(value.Record)) {
    if (deep) return "{…}";
    const shown = take(value.Record).map(
      (entry) => `${entry?.[0]}:${render(entry?.[1], depth + 1, full)}`
    );
    return `{${[...shown, ...more(value.Record, "…")].join(", ")}}`;
  }
  if ("Map" in value && Array.isArray(value.Map)) {
    if (deep) return "{…}";
    const shown = take(value.Map).map(
      (entry) => `${keyOf(entry?.[0])}→${render(entry?.[1], depth + 1, full)}`
    );
    return `{${[...shown, ...more(value.Map, "…")].join(", ")}}`;
  }
  if ("Variant" in value && Array.isArray(value.Variant)) {
    const [ctor, args] = value.Variant;
    const label = typeof ctor === "string" ? ctor : asJson(ctor);
    if (!Array.isArray(args) || args.length === 0) return label;
    // A one-record argument is the common shape (`Steer(intent)`), and it reads
    // better braced than parenthesized — the mockup's `Move {dx:1}`.
    if (args.length === 1 && args[0] && "Record" in args[0]) {
      return `${label} ${render(args[0], depth, full)}`;
    }
    return `${label}(${args.map((arg) => render(arg, depth + 1, full)).join(", ")})`;
  }
  // Not a value this understands: show it as the JSON it is, rather than
  // pretending — and rather than throwing.
  return asJson(value);
};

/**
 * The typed payload inside a wire text, or null for plain `Effect.send` text
 * (and for a frame that does not parse — version skew, or a game that wrote the
 * prefix itself).
 */
const parseWire = (raw: string): WireValue | null => {
  if (!raw.startsWith(TYPED_MSG_PREFIX)) return null;
  try {
    const value = JSON.parse(raw.slice(TYPED_MSG_PREFIX.length)) as WireValue;
    return typeof value === "object" && value !== null ? value : null;
  } catch {
    return null;
  }
};

/**
 * Decode a packet's wire text into the row a wire log shows.
 *
 * Never throws, and that is load-bearing rather than defensive: this runs
 * inside the host's rAF loop, on text a game chose, so a single hostile or
 * skewed frame that threw would stop the whole chrono bar. Anything that is not
 * a typed payload is shown as the text it is, marked untyped.
 */
export function decodeWire(wire: string | undefined): DecodedWire {
  const raw = wire ?? "";
  const value = parseWire(raw);
  if (!value) return { head: "", body: raw, typed: false };
  const compact = render(value, 0, false);
  if ("Variant" in value && Array.isArray(value.Variant) && typeof value.Variant[0] === "string") {
    const ctor = value.Variant[0];
    // `render` already puts the constructor at the front; the row prints the
    // head itself, so the body is only what follows it.
    return { head: ctor, body: compact.slice(ctor.length).trim(), typed: true };
  }
  return { head: "", body: compact, typed: true };
}

/**
 * The same payload with every elision dropped — the row's tooltip.
 *
 * Separate from `decodeWire`, and deliberately: rendering a whole world
 * snapshot per row is far more expensive than the one-line form, and the panel
 * repaints its rows continuously while only ever showing one tooltip. Capped
 * all the same — a title attribute is not the place for a megabyte of payload.
 */
export function fullWire(wire: string | undefined): string {
  const raw = wire ?? "";
  const value = parseWire(raw);
  const rendered = value ? render(value, 0, true) : raw;
  return rendered.length <= MAX_FULL ? rendered : `${rendered.slice(0, MAX_FULL)}…`;
}
