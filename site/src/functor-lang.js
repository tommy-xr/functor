// CodeMirror 6 language support for Functor Lang, plus the site's synthwave editor
// theme. The tokenizer is a small hand-rolled StreamLanguage (Functor Lang has no
// Lezer grammar); keep the keyword list in sync with the language
// (.claude/skills/functor-lang/SKILL.md is the source of truth).

import { HighlightStyle, StreamLanguage, syntaxHighlighting } from "@codemirror/language";
import { EditorView } from "@codemirror/view";
import { tags } from "@lezer/highlight";

const KEYWORDS = new Set([
  "let",
  "type",
  "match",
  "with",
  "mut",
  "in",
  "if",
  "then",
  "else",
  "not",
]);
const ATOMS = new Set(["true", "false"]);

export const functorLangLanguage = StreamLanguage.define({
  name: "functor-lang",
  startState: () => ({ contexts: [] }),
  copyState: (state) => ({
    contexts: state.contexts.map((context) => ({ ...context })),
  }),
  token(stream, state) {
    const context = state.contexts.at(-1);
    if (context?.kind === "interpolated") {
      if (stream.match('\\"') || stream.match("\\\\") || stream.match("\\n") || stream.match("\\t")) {
        return "string";
      }
      if (stream.match("{{") || stream.match("}}")) return "string";
      if (stream.match('"')) {
        state.contexts.pop();
        return "string";
      }
      if (stream.match("{")) {
        state.contexts.push({ kind: "hole", braces: 0 });
        return "bracket";
      }
      while (!stream.eol() && !/["\\{}]/.test(stream.peek())) stream.next();
      if (stream.pos === stream.start) stream.next();
      return "string";
    }
    if (context?.kind === "string") {
      while (!stream.eol()) {
        if (stream.next() === "\\") stream.next();
        else if (stream.current().endsWith('"')) {
          state.contexts.pop();
          break;
        }
      }
      return "string";
    }
    if (stream.eatSpace()) return null;
    if (stream.match("//")) {
      stream.skipToEnd();
      return "comment";
    }
    if (stream.match('$"')) {
      state.contexts.push({ kind: "interpolated" });
      return "string";
    }
    if (stream.match('"')) {
      state.contexts.push({ kind: "string" });
      while (!stream.eol()) {
        if (stream.next() === "\\") stream.next();
        else if (stream.current().endsWith('"')) {
          state.contexts.pop();
          break;
        }
      }
      return "string";
    }
    if (context?.kind === "hole") {
      if (stream.match("{")) {
        context.braces += 1;
        return "bracket";
      }
      if (stream.match("}")) {
        if (context.braces === 0) state.contexts.pop();
        else context.braces -= 1;
        return "bracket";
      }
    }
    if (stream.match(/^\d+(\.\d+)?/)) return "number";
    // Uppercase head: constructors and prelude namespaces (Scene, Math, …).
    if (stream.match(/^[A-Z][A-Za-z0-9_]*/)) return "typeName";
    if (stream.match(/^[a-z_][A-Za-z0-9_]*/)) {
      const word = stream.current();
      if (KEYWORDS.has(word)) return "keyword";
      if (ATOMS.has(word)) return "atom";
      return "variableName";
    }
    if (
      stream.match("|>") ||
      stream.match("=>") ||
      stream.match(":=") ||
      stream.match("&&") ||
      stream.match("||")
    )
      return "operator";
    if (stream.match(/^[+\-*/<>=|]/)) return "operator";
    stream.next();
    return null;
  },
  languageData: {
    commentTokens: { line: "//" },
  },
});

// Calmer dark-violet palette matching the site theme: demoted pink keywords,
// cyan the primary accent — tuned against the site's #0f0c1d background.
const highlight = HighlightStyle.define([
  { tag: tags.keyword, color: "#e858b8" },
  { tag: tags.atom, color: "#eec877" },
  { tag: tags.typeName, color: "#eec877" },
  { tag: tags.variableName, color: "#c7f2f7" },
  { tag: tags.operator, color: "#9b94b3" },
  { tag: tags.number, color: "#b7a9e0" },
  { tag: tags.string, color: "#6fdc92" },
  { tag: tags.comment, color: "#6c6685", fontStyle: "italic" },
]);

const chrome = EditorView.theme(
  {
    "&": {
      backgroundColor: "#161226",
      color: "#e9e6f2",
      height: "100%",
      fontSize: "13.5px",
    },
    ".cm-content": {
      fontFamily: "'JetBrains Mono', 'Fira Code', ui-monospace, monospace",
      caretColor: "#41d8e6",
    },
    ".cm-cursor": { borderLeftColor: "#41d8e6" },
    "&.cm-focused .cm-selectionBackground, .cm-selectionBackground": {
      backgroundColor: "#2b254288",
    },
    ".cm-activeLine": { backgroundColor: "#1e183333" },
    ".cm-activeLineGutter": { backgroundColor: "#1e1833" },
    ".cm-gutters": {
      backgroundColor: "#0f0c1d",
      color: "#6c6685",
      border: "none",
      borderRight: "1px solid #2b2542",
    },
    "&.cm-focused": { outline: "none" },
  },
  { dark: true }
);

export const synthwaveEditorTheme = [chrome, syntaxHighlighting(highlight)];
