// The agent's Markdown as React elements. `marked` only tokenizes; every
// token becomes an element or escaped text here, so no HTML from the model
// is ever inserted: raw HTML shows as its source, images as their alt text,
// and only absolute http(s)/mailto links stay links (opened outside the app).
// No JSX, so `node --test` can import it.

import { Lexer, type Token, type Tokens } from "marked";
import { createElement as h, type MouseEvent, type ReactNode } from "react";

const SCHEMES = new Set(["http:", "https:", "mailto:"]);

/** `href` normalised when it is an absolute http(s) or mailto URL, else `null`. */
export function safeHref(href: string): string | null {
  try {
    const url = new URL(href);
    return SCHEMES.has(url.protocol) ? url.href : null;
  } catch {
    return null;
  }
}

/** Opens a link in a new browsing context; the app page itself never navigates. */
function openOutside(e: MouseEvent<HTMLAnchorElement>) {
  e.preventDefault();
  window.open(e.currentTarget.href, "_blank", "noopener,noreferrer");
}

export function renderMarkdown(text: string): ReactNode {
  return blocks(new Lexer({ gfm: true }).lex(text));
}

function blocks(tokens: Token[]): ReactNode[] {
  return tokens.map((t, key) => block(t, key));
}

function block(t: Token, key: number): ReactNode {
  switch (t.type) {
    case "space":
    case "def":
      return null;
    case "paragraph":
      return h("p", { key }, inline(t.tokens ?? []));
    case "heading":
      return h("p", { key, className: "md-heading" }, inline(t.tokens ?? []));
    case "code":
      return h("pre", { key }, h("code", null, t.text));
    case "blockquote":
      return h("blockquote", { key }, blocks(t.tokens ?? []));
    case "hr":
      return h("hr", { key });
    case "list": {
      const list = t as Tokens.List;
      const start = list.ordered && typeof list.start === "number" && list.start !== 1 ? list.start : undefined;
      return h(
        list.ordered ? "ol" : "ul",
        { key, start },
        list.items.map((item, i) => h("li", { key: i }, blocks(item.tokens))),
      );
    }
    case "table": {
      const table = t as Tokens.Table;
      const cells = (row: Tokens.TableCell[], tag: "th" | "td") =>
        row.map((c, i) => h(tag, { key: i }, inline(c.tokens)));
      return h(
        "table",
        { key },
        h("thead", null, h("tr", null, cells(table.header, "th"))),
        h(
          "tbody",
          null,
          table.rows.map((row, i) => h("tr", { key: i }, cells(row, "td"))),
        ),
      );
    }
    // A tight list item's text, or a block of raw HTML: shown, never parsed.
    case "text":
      return t.tokens ? h("span", { key }, inline(t.tokens)) : decode(t.text);
    case "html":
      return h("p", { key }, t.text);
    default:
      return h("p", { key }, t.raw);
  }
}

function inline(tokens: Token[]): ReactNode[] {
  return tokens.map((t, key) => {
    switch (t.type) {
      case "text":
        return t.tokens ? h("span", { key }, inline(t.tokens)) : decode(t.text);
      case "escape":
        return t.text;
      case "strong":
        return h("strong", { key }, inline(t.tokens ?? []));
      case "em":
        return h("em", { key }, inline(t.tokens ?? []));
      case "del":
        return h("del", { key }, inline(t.tokens ?? []));
      case "codespan":
        return h("code", { key }, t.text);
      case "br":
        return h("br", { key });
      case "link": {
        const href = safeHref((t as Tokens.Link).href);
        const children = inline(t.tokens ?? []);
        if (!href) return h("span", { key }, children);
        return h("a", { key, href, target: "_blank", rel: "noopener noreferrer", onClick: openOutside }, children);
      }
      // Never fetched: an image would be a request the page must not make.
      case "image":
        return (t as Tokens.Image).text;
      case "checkbox":
        return (t as Tokens.Checkbox).checked ? "☑ " : "☐ ";
      // Raw HTML (`<img onerror=…>`) and anything unknown: its source, as text.
      default:
        return t.raw;
    }
  });
}

const ENTITIES: Record<string, string> = { amp: "&", lt: "<", gt: ">", quot: '"', apos: "'", nbsp: " " };

/** Markdown text keeps entities as written: `&amp;` → `&`, `&#60;` → `<`. The result is text, never markup. */
function decode(text: string): string {
  return text.replace(/&(#x[0-9a-f]+|#\d+|[a-z]+);/gi, (whole, ref: string) => {
    if (ref[0] !== "#") return ENTITIES[ref.toLowerCase()] ?? whole;
    const code = ref[1] === "x" || ref[1] === "X" ? parseInt(ref.slice(2), 16) : parseInt(ref.slice(1), 10);
    return code > 0 && code <= 0x10ffff ? String.fromCodePoint(code) : whole;
  });
}
