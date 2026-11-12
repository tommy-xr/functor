// CodeMirror 6 language support for Functor Lang, plus the site's synthwave editor
// theme. The tokenizer itself is shared with the desktop app (functor-lang-syntax.ts).

import { HighlightStyle, StreamLanguage, syntaxHighlighting } from "@codemirror/language";
import { EditorView } from "@codemirror/view";
import { tags } from "@lezer/highlight";
import { functorLangParser } from "./functor-lang-syntax.js";

export const functorLangLanguage = StreamLanguage.define(functorLangParser);

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
