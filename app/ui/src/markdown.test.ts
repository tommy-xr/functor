import { test } from "node:test";
import assert from "node:assert/strict";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { renderMarkdown, safeHref } from "./markdown.ts";

const html = (md: string) => renderToStaticMarkup(createElement("div", null, renderMarkdown(md)));

test("paragraphs, lists, inline code and fenced code", () => {
  assert.equal(
    html("The count is `3`.\n\n- one\n- **two**\n\n1. first\n\n```fun\nlet a = 1 < 2\n```"),
    "<div><p>The count is <code>3</code>.</p>" +
      "<ul><li><span>one</span></li><li><span><strong>two</strong></span></li></ul>" +
      "<ol><li><span>first</span></li></ol>" +
      "<pre><code>let a = 1 &lt; 2</code></pre></div>",
  );
});

test("raw HTML from the model is shown as text, never as markup", () => {
  const out = html('<script>alert(1)</script>\n\nhi <img src=x onerror="alert(1)"> and <b>bold</b>');
  assert.ok(!/<script|<img|<b>/i.test(out), out);
  assert.ok(out.includes("&lt;script&gt;alert(1)&lt;/script&gt;"), out);
  assert.ok(out.includes("&lt;img src=x onerror=&quot;alert(1)&quot;&gt;"), out);
});

test("images are never loaded: only their alt text shows", () => {
  const out = html("![a cat](https://example.com/cat.png)");
  assert.equal(out, "<div><p>a cat</p></div>");
});

test("javascript: and other unsafe links are neutralised to their text", () => {
  for (const md of [
    "[click](javascript:alert(1))",
    "[click](JaVaScRiPt:alert(1))",
    "[click](java\tscript:alert(1))",
    "[click](data:text/html,<script>alert(1)</script>)",
    "[click](vbscript:msgbox(1))",
    "[click](/relative/path)",
    "<javascript:alert(1)>",
  ]) {
    const out = html(md);
    // At most the link's text remains, inert.
    assert.ok(!out.includes("<a") && !out.includes("href"), `${md} → ${out}`);
  }
});

test("http(s) and mailto links open outside the app", () => {
  const out = html("See the [guide](https://functor.games/manual/) or <https://example.com>.");
  assert.equal(
    out,
    '<div><p>See the <a href="https://functor.games/manual/" target="_blank" rel="noopener noreferrer">guide</a>' +
      ' or <a href="https://example.com/" target="_blank" rel="noopener noreferrer">https://example.com</a>.</p></div>',
  );
  assert.equal(safeHref("mailto:a@b.c"), "mailto:a@b.c");
  assert.equal(safeHref("  https://x.dev/a b"), "https://x.dev/a%20b");
  assert.equal(safeHref("javascript:alert(1)"), null);
  assert.equal(safeHref("file:///etc/passwd"), null);
  assert.equal(safeHref("game.fun"), null);
});

test("entities written in the text read as their characters, as plain text", () => {
  assert.equal(html("a &amp; b &lt;tag&gt; &#65;&#x42; &bogus;"), "<div><p>a &amp; b &lt;tag&gt; AB &amp;bogus;</p></div>");
});

test("a partial reply renders what has arrived", () => {
  assert.equal(html("The count is `3"), "<div><p>The count is `3</p></div>");
  assert.equal(html("```fun\nlet a"), "<div><pre><code>let a</code></pre></div>");
});
