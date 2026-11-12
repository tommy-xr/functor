// Syntax highlighting for read-only text: the site's Functor Lang stream
// parser, run line by line over the whole text (no editor), its tokens
// rendered as spans of text nodes. No JSX, so `node --test` runs it.

import type { StreamParser, StringStream } from "@codemirror/language";
import { createElement as h, type ReactNode } from "react";
import { functorLangParser } from "../../../site/src/functor-lang-syntax.ts";
import type { DiffLine } from "./generated/protocol";

/** A run of text and its token kind (`keyword`, `string`, …), or null when plain. */
export type Token = { text: string; kind: string | null };

/** A stream parser with its state type erased: only `tokenize*` reads the state. */
type Lang = StreamParser<unknown>;

/** Beyond this, a file shows as plain text: tokens are cheap, a span per token is not. */
export const MAX_HIGHLIGHT_LINES = 5000;

/**
 * The language for a path: Functor Lang (`.fun`, `.funi`), and `.json` through
 * the same parser: its strings, plain numbers and booleans lex alike (`null`
 * stays plain; `-1` and `1e-5` split at the sign).
 */
export function langFor(path: string): Lang | null {
  return /\.(funi?|json)$/i.test(path) ? (functorLangParser as Lang) : null;
}

function start(lang: Lang): unknown {
  return lang.startState ? lang.startState(4) : null;
}

/**
 * The part of CodeMirror's `StringStream` the Functor Lang parser calls; a
 * parser calling more throws, and the text shows plain (`renderText`,
 * `renderLines`). Importing the real one brings most of
 * @codemirror/language (~230 KB minified) into the bundle.
 */
class LineStream {
  pos = 0;
  start = 0;
  string: string;
  constructor(line: string) {
    this.string = line;
  }
  eol() {
    return this.pos >= this.string.length;
  }
  peek() {
    return this.string.charAt(this.pos) || undefined;
  }
  next() {
    return this.pos < this.string.length ? this.string.charAt(this.pos++) : undefined;
  }
  eatSpace() {
    const from = this.pos;
    while (/\s/.test(this.string.charAt(this.pos))) this.pos++;
    return this.pos > from;
  }
  skipToEnd() {
    this.pos = this.string.length;
  }
  current() {
    return this.string.slice(this.start, this.pos);
  }
  match(pattern: string | RegExp, consume = true): boolean | RegExpMatchArray | null {
    if (typeof pattern === "string") {
      if (!this.string.startsWith(pattern, this.pos)) return null;
      if (consume) this.pos += pattern.length;
      return true;
    }
    const m = this.string.slice(this.pos).match(pattern);
    if (!m || m.index! > 0) return null;
    if (consume) this.pos += m[0].length;
    return m;
  }
}

/** One line's tokens, advancing `state` past it. */
export function tokenizeLine(lang: Lang, line: string, state: unknown): Token[] {
  const out: Token[] = [];
  const stream = new LineStream(line);
  while (!stream.eol()) {
    const kind = lang.token(stream as unknown as StringStream, state) ?? null;
    // A token that consumed nothing would loop forever: take one character.
    if (stream.pos === stream.start) stream.next();
    const text = line.slice(stream.start, stream.pos);
    const last = out.at(-1);
    if (last && last.kind === kind) last.text += text;
    else out.push({ text, kind });
    stream.start = stream.pos;
  }
  return out;
}

/** Every line's tokens, one state carried through, so a string or comment spans lines. */
export function tokenize(lang: Lang, lines: string[]): Token[][] {
  const state = start(lang);
  return lines.map((line) => tokenizeLine(lang, line, state));
}

/**
 * A diff's lines' tokens (null for hunk headers and notes). The old side
 * (context + removed) and new side (context + added) each carry their own
 * state, restarted at each hunk: a hunk does not say what came before it.
 */
export function tokenizeDiff(lang: Lang, lines: DiffLine[]): (Token[] | null)[] {
  let old = start(lang);
  let neu = start(lang);
  return lines.map((line) => {
    switch (line.kind) {
      case "hunk":
        old = start(lang);
        neu = start(lang);
        return null;
      case "note":
        return null;
      case "added":
        return tokenizeLine(lang, line.text, neu);
      case "removed":
        return tokenizeLine(lang, line.text, old);
      case "context":
        tokenizeLine(lang, line.text, old);
        return tokenizeLine(lang, line.text, neu);
    }
  });
}

/** Tokens as text nodes, the coloured ones in `<span class="tok-…">`. */
export function renderTokens(tokens: Token[], key = ""): ReactNode[] {
  return tokens.map((t, i) => (t.kind && COLOURED.has(t.kind) ? h("span", { key: `${key}${i}`, className: `tok-${t.kind}` }, t.text) : t.text));
}

/** A whole text's nodes, its lines joined by newlines; plain text past `MAX_HIGHLIGHT_LINES`. */
export function renderText(path: string, text: string): ReactNode {
  const lang = langFor(path);
  if (!lang) return text;
  const lines = text.split("\n");
  if (lines.length > MAX_HIGHLIGHT_LINES) return text;
  let tokens: Token[][];
  try {
    tokens = tokenize(lang, lines);
  } catch {
    return text;
  }
  return tokens.flatMap((line, i) => {
    const nodes = renderTokens(line, `${i}:`);
    return i < lines.length - 1 ? [...nodes, "\n"] : nodes;
  });
}

/** The kinds with a colour (styles.css `.tok-*`); identifiers and brackets stay plain. */
const COLOURED = new Set(["keyword", "atom", "typeName", "number", "string", "comment", "operator"]);
