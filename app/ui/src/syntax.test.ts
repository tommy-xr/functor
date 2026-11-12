import { test } from "node:test";
import assert from "node:assert/strict";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { DiffLine } from "./generated/protocol.ts";
import { MAX_HIGHLIGHT_LINES, langFor, renderText, tokenize, tokenizeDiff, type Token } from "./syntax.ts";

const fun = langFor("game.fun")!;
/** Each token as `kind:text`, plain runs as `text`. */
const show = (tokens: Token[]) => tokens.map((t) => (t.kind ? `${t.kind}:${t.text}` : t.text));

test("languages by extension", () => {
  assert.ok(langFor("a/b/game.fun"));
  assert.ok(langFor("functor.json"));
  assert.equal(langFor("README.md"), null);
  assert.equal(langFor("fun"), null);
});

test("a line of Functor Lang", () => {
  const [line] = tokenize(fun, ['let speed = 90deg |> Math.max(1.5) // fast']);
  assert.deepEqual(show(line!), [
    "keyword:let",
    " ",
    "variableName:speed",
    " ",
    "operator:=",
    " ",
    "number:90deg",
    " ",
    "operator:|>",
    " ",
    "typeName:Math",
    ".",
    "variableName:max",
    "(",
    "number:1.5",
    ") ",
    "comment:// fast",
  ]);
});

test("a string open at a line's end carries to the next", () => {
  const lines = tokenize(fun, ['let s = "one', 'two" + 1', "if true then 2 else 3"]);
  assert.deepEqual(show(lines[0]!).slice(-1), ['string:"one']);
  assert.deepEqual(show(lines[1]!), ['string:two"', " ", "operator:+", " ", "number:1"]);
  assert.deepEqual(show(lines[2]!).filter((t) => t.includes(":")), [
    "keyword:if",
    "atom:true",
    "keyword:then",
    "number:2",
    "keyword:else",
    "number:3",
  ]);
});

test("an interpolated string's hole across lines", () => {
  const lines = tokenize(fun, ['$"a {x', '} b"', "y"]);
  assert.deepEqual(show(lines[0]!), ['string:$"a ', "bracket:{", "variableName:x"]);
  assert.deepEqual(show(lines[1]!), ["bracket:}", 'string: b"']);
  assert.deepEqual(show(lines[2]!), ["variableName:y"]);
});

test("a diff keeps one state per side and restarts at each hunk", () => {
  const line = (kind: DiffLine["kind"], text: string): DiffLine => ({ kind, text });
  const tokens = tokenizeDiff(fun, [
    line("hunk", "@@ -1,2 +1,2 @@"),
    line("removed", 'let a = "open'),
    line("added", "let a = 1"),
    line("context", 'end"'),
    line("hunk", "@@ -9 +9 @@"),
    line("context", 'end"'),
  ]);
  assert.equal(tokens[0], null);
  // The old side's string is still open on the context line, the new side's is not:
  // the line is shown as the new file has it.
  assert.deepEqual(show(tokens[3]!), ["variableName:end", 'string:"']);
  assert.deepEqual(show(tokens[5]!), ["variableName:end", 'string:"']);
});

test("rendered text keeps its exact characters", () => {
  const text = 'let x = 1\n\n// c\nlet s = "a\\"b"';
  const html = renderToStaticMarkup(createElement("pre", null, renderText("game.fun", text)));
  assert.match(html, /<span class="tok-keyword">let<\/span>/);
  assert.match(html, /<span class="tok-comment">\/\/ c<\/span>/);
  const plain = html.replace(/<[^>]+>/g, "").replace(/&quot;/g, '"');
  assert.equal(plain, text);
  // Unknown types and very long files stay plain.
  assert.equal(renderText("notes.txt", text), text);
  const long = "x\n".repeat(MAX_HIGHLIGHT_LINES);
  assert.equal(renderText("big.fun", long), long);
});
