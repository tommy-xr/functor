// Docs page enhancement: syntax-highlight the <pre class="mle"> blocks and
// give the runnable ones (class "runnable") a "▶ try it" button that opens
// the program in the sandbox via the #src= fragment (see sandbox.js).
//
// The tokenizer mirrors src/mle.js's CodeMirror StreamLanguage; it's a few
// regexes, so a static-HTML variant beats dragging CodeMirror onto the docs
// page. Keep the two classifications in sync.

const KEYWORDS = new Set(["let", "type", "match", "with", "mut", "in"]);
const ATOMS = new Set(["true", "false"]);

const TOKEN =
  /\/\/[^\n]*|"(?:[^"\\]|\\.)*"?|\d+(?:\.\d+)?|[A-Za-z_][A-Za-z0-9_]*|\|>|=>|:=|[+\-*/<>=|]/g;

const escapeHtml = (s) =>
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

const highlight = (source) => {
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

const toBase64Url = (s) =>
  btoa(String.fromCharCode(...new TextEncoder().encode(s)))
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/, "");

for (const pre of document.querySelectorAll("pre.mle")) {
  const source = pre.textContent;
  pre.innerHTML = highlight(source);
  if (pre.classList.contains("runnable")) {
    const link = document.createElement("a");
    link.className = "try-button";
    link.textContent = "▶ try it";
    link.title = "Open this program live in the sandbox";
    link.href = `sandbox.html#src=${toBase64Url(source)}`;
    link.target = "_blank";
    pre.appendChild(link);
  }
}
