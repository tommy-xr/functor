// Docs page enhancement: syntax-highlight the <pre class="functor-lang"> blocks and
// give the runnable ones (class "runnable") a "▶ try it" button that opens
// the program in the sandbox via the #src= fragment (see sandbox.js).
//
// The tokenizer mirrors src/functor-lang.js's CodeMirror StreamLanguage; it's a few
// regexes, so a static-HTML variant beats dragging CodeMirror onto the docs
// page. Keep the two classifications in sync.

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

const CODE_TOKEN =
  /^(?:\d+(?:\.\d+)?|[A-Za-z_][A-Za-z0-9_]*|\|>|=>|:=|&&|\|\||[+\-*/<>=|])/;

const escapeHtml = (s) =>
  s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

const classify = (token) => {
  if (token.startsWith("//")) return "tok-c";
  if (/^\d/.test(token)) return "tok-n";
  if (/^[A-Z]/.test(token)) return "tok-t";
  if (KEYWORDS.has(token)) return "tok-k";
  if (ATOMS.has(token)) return "tok-a";
  if (/^[a-z_]/.test(token)) return null;
  return "tok-o";
};

const highlight = (source) => {
  let html = "";
  const contexts = [];
  let i = 0;
  const emit = (text, cls = null) => {
    const escaped = escapeHtml(text);
    html += cls ? `<span class="${cls}">${escaped}</span>` : escaped;
  };
  const emitString = () => {
    const start = i++;
    while (i < source.length) {
      if (source[i] === "\\") i += Math.min(2, source.length - i);
      else if (source[i++] === '"') break;
    }
    emit(source.slice(start, i), "tok-s");
  };

  while (i < source.length) {
    const context = contexts.at(-1);
    if (context?.kind === "interpolated") {
      const start = i;
      if (source.startsWith("{{", i) || source.startsWith("}}", i)) i += 2;
      else if (source[i] === "\\") i += Math.min(2, source.length - i);
      else if (source[i] === '"') {
        i += 1;
        contexts.pop();
      } else if (source[i] === "{") {
        emit("{", "tok-o");
        i += 1;
        contexts.push({ kind: "hole", braces: 0 });
        continue;
      } else if (source[i] === "}") {
        // Invalid language input still needs a progress-safe preview; the
        // parser will report that a literal brace must be written as `}}`.
        i += 1;
      } else {
        while (i < source.length && !/["\\{}]/.test(source[i])) i += 1;
      }
      emit(source.slice(start, i), "tok-s");
      continue;
    }
    if (source.startsWith("//", i)) {
      const end = source.indexOf("\n", i);
      const next = end === -1 ? source.length : end;
      emit(source.slice(i, next), "tok-c");
      i = next;
      continue;
    }
    if (source.startsWith('$"', i)) {
      emit('$"', "tok-s");
      i += 2;
      contexts.push({ kind: "interpolated" });
      continue;
    }
    if (source[i] === '"') {
      emitString();
      continue;
    }
    if (context?.kind === "hole" && source[i] === "{") {
      context.braces += 1;
      emit(source[i++], "tok-o");
      continue;
    }
    if (context?.kind === "hole" && source[i] === "}") {
      if (context.braces === 0) contexts.pop();
      else context.braces -= 1;
      emit(source[i++], "tok-o");
      continue;
    }
    const match = source.slice(i).match(CODE_TOKEN);
    if (match) {
      const cls = classify(match[0]);
      const text = escapeHtml(match[0]);
      html += cls ? `<span class="${cls}">${text}</span>` : text;
      i += match[0].length;
      continue;
    }
    emit(source[i++]);
  }
  return html;
};

const toBase64Url = (s) =>
  btoa(String.fromCharCode(...new TextEncoder().encode(s)))
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/, "");

for (const pre of document.querySelectorAll("pre.functor-lang")) {
  const source = pre.textContent;
  pre.innerHTML = highlight(source);
  if (pre.classList.contains("runnable")) {
    const link = document.createElement("a");
    link.className = "try-button";
    link.textContent = "▶ try it";
    link.title = "Open this program live in the sandbox";
    const sandboxHref = document.body.dataset.sandboxHref || "sandbox.html";
    link.href = `${sandboxHref}#src=${toBase64Url(source)}`;
    link.target = "_blank";
    pre.appendChild(link);
  }
}
