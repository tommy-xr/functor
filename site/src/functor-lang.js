// CodeMirror 6 language support for Functor Lang, plus the site's synthwave editor
// theme. The tokenizer is a small hand-rolled StreamLanguage (Functor Lang has no
// Lezer grammar); keep the keyword list in sync with the language
// (.claude/skills/functor-lang/SKILL.md is the source of truth).

import { HighlightStyle, StreamLanguage, syntaxHighlighting } from "@codemirror/language";
import { EditorView } from "@codemirror/view";
import { tags } from "@lezer/highlight";

const KEYWORDS = new Set(["let", "type", "match", "with", "mut", "in"]);
const ATOMS = new Set(["true", "false"]);

export const functorLangLanguage = StreamLanguage.define({
  name: "functor-lang",
  token(stream) {
    if (stream.eatSpace()) return null;
    if (stream.match("//")) {
      stream.skipToEnd();
      return "comment";
    }
    if (stream.match(/^"(?:[^"\\]|\\.)*"?/)) return "string";
    if (stream.match(/^\d+(\.\d+)?/)) return "number";
    // Uppercase head: constructors and prelude namespaces (Scene, Math, …).
    if (stream.match(/^[A-Z][A-Za-z0-9_]*/)) return "typeName";
    if (stream.match(/^[a-z_][A-Za-z0-9_]*/)) {
      const word = stream.current();
      if (KEYWORDS.has(word)) return "keyword";
      if (ATOMS.has(word)) return "atom";
      return "variableName";
    }
    if (stream.match("|>") || stream.match("=>") || stream.match(":=")) return "operator";
    if (stream.match(/^[+\-*/<>=|]/)) return "operator";
    stream.next();
    return null;
  },
  languageData: {
    commentTokens: { line: "//" },
  },
});

// Synthwave '84-adjacent palette: hot pink keywords, cyan names, glowing
// constructors — tuned against the site's #0d0221 background.
const highlight = HighlightStyle.define([
  { tag: tags.keyword, color: "#ff2fbf" },
  { tag: tags.atom, color: "#ff9f43" },
  { tag: tags.typeName, color: "#ffd76d" },
  { tag: tags.variableName, color: "#9ef7ff" },
  { tag: tags.operator, color: "#ff6ad5" },
  { tag: tags.number, color: "#c792ff" },
  { tag: tags.string, color: "#7bf59d" },
  { tag: tags.comment, color: "#6a5a96", fontStyle: "italic" },
]);

const chrome = EditorView.theme(
  {
    "&": {
      backgroundColor: "#0f0328",
      color: "#e8dcff",
      height: "100%",
      fontSize: "13.5px",
    },
    ".cm-content": {
      fontFamily: "'JetBrains Mono', 'Fira Code', ui-monospace, monospace",
      caretColor: "#27e8f7",
    },
    ".cm-cursor": { borderLeftColor: "#27e8f7" },
    "&.cm-focused .cm-selectionBackground, .cm-selectionBackground": {
      backgroundColor: "#3b1a6e88",
    },
    ".cm-activeLine": { backgroundColor: "#1c0a4233" },
    ".cm-activeLineGutter": { backgroundColor: "#1c0a42" },
    ".cm-gutters": {
      backgroundColor: "#0d0221",
      color: "#5d4a8a",
      border: "none",
      borderRight: "1px solid #2a1454",
    },
    "&.cm-focused": { outline: "none" },
  },
  { dark: true }
);

export const synthwaveEditorTheme = [chrome, syntaxHighlighting(highlight)];
