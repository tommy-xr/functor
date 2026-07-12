// Build-time syntax highlighter for Functor Lang code blocks. The tokenizer is a
// few regexes lifted verbatim from the old src/docs.js; it mirrors
// src/functor-lang.js's CodeMirror StreamLanguage (same token classes tok-*).
// Keep the two classifications in sync (functor-lang.js carries the note).

const KEYWORDS = new Set(["let", "type", "match", "with", "mut", "in"]);
const ATOMS = new Set(["true", "false"]);

const TOKEN =
  /\/\/[^\n]*|"(?:[^"\\]|\\.)*"?|\d+(?:\.\d+)?|[A-Za-z_][A-Za-z0-9_]*|\|>|=>|:=|[+\-*/<>=|]/g;

export const escapeHtml = (s) =>
  s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

const classify = (token) => {
  if (token.startsWith("//")) return "tok-c";
  if (token.startsWith('"')) return "tok-s";
  if (/^\d/.test(token)) return "tok-n";
  if (/^[A-Z]/.test(token)) return "tok-t";
  if (KEYWORDS.has(token)) return "tok-k";
  if (ATOMS.has(token)) return "tok-a";
  if (/^[a-z_]/.test(token)) return null;
  return "tok-o";
};

export const highlight = (source) => {
  let html = "";
  let last = 0;
  for (const match of source.matchAll(TOKEN)) {
    html += escapeHtml(source.slice(last, match.index));
    const cls = classify(match[0]);
    const text = escapeHtml(match[0]);
    html += cls ? `<span class="${cls}">${text}</span>` : text;
    last = match.index + match[0].length;
  }
  return html + escapeHtml(source.slice(last));
};

export const toBase64Url = (s) =>
  btoa(String.fromCharCode(...new TextEncoder().encode(s)))
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/, "");
