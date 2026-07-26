// Render the generated API reference to static HTML at build time.
//
// The reference used to be built in the browser from the bundled JSON, which
// meant a plain (no-JS) GET of /docs/ returned only the page shell — invisible
// to the LLM agents Functor is meant to be readable by. This module is the
// single renderer: `build.mjs` calls it to bake the real markup into
// `dist/docs/index.html`, and `src/api-docs.js` only enhances that markup
// (search/filter). The emitted class names, ids and `data-search` attributes
// are what the client script and `styles.css` expect.

const escapeText = (value) =>
  String(value).replace(/[&<>]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" })[c]);

const escapeAttr = (value) => escapeText(value).replace(/"/g, "&quot;");

export const slug = (name) => `api-${name.toLowerCase().replace(/[^a-z0-9]+/g, "-")}`;

// `` `code` `` spans inside a paragraph, mirroring the client renderer.
const inlineProse = (prose) =>
  prose
    .split(/(`[^`\n]+`)/)
    .map((part) =>
      // Shape-tested, not just delimiter-tested: a bare ``` `` ``` in prose is
      // literal backticks, not an empty code span.
      /^`[^`\n]+`$/.test(part)
        ? `<code>${escapeText(part.slice(1, -1))}</code>`
        : escapeText(part),
    )
    .join("");

const prose = (text) =>
  text
    .split(/\n\s*\n/)
    .map((paragraph) => `<p>${inlineProse(paragraph.replace(/\n/g, " "))}</p>`)
    .join("\n");

const item = (entry) => {
  const id = slug(entry.qualified_name);
  const search =
    `${entry.qualified_name} ${entry.name} ${entry.kind} ${entry.declaration} ${entry.docs}`.toLowerCase();
  return [
    `<article class="api-item" id="${escapeAttr(id)}" data-search="${escapeAttr(search)}">`,
    `<h3><a href="#${escapeAttr(id)}" title="Link to ${escapeAttr(entry.qualified_name)}">${escapeText(entry.qualified_name)}</a>` +
      `<span class="api-kind api-kind-${escapeAttr(entry.kind)}">${escapeText(entry.kind)}</span></h3>`,
    `<pre class="api-declaration"><code>${escapeText(entry.declaration)}</code></pre>`,
    prose(entry.docs),
    `</article>`,
  ].join("\n");
};

const module_ = (module) => {
  const id = slug(module.name);
  const entries = `${module.items.length} ${module.items.length === 1 ? "entry" : "entries"}`;
  return [
    `<section class="api-module" id="${escapeAttr(id)}" data-search="${escapeAttr(`${module.name} ${module.docs}`.toLowerCase())}">`,
    `<h2><span>${escapeText(module.name)}</span><span class="api-module-count">${entries}</span></h2>`,
    prose(module.docs),
    `<div class="api-items">`,
    module.items.map(item).join("\n"),
    `</div>`,
    `</section>`,
  ].join("\n");
};

/** The static markup for the reference page: module nav, sections, and counts. */
export const renderApiReference = (reference) => ({
  nav: reference.modules
    .map(
      (module) =>
        `<a href="#${escapeAttr(slug(module.name))}" data-module="${escapeAttr(module.name.toLowerCase())}">${escapeText(module.name)}</a>`,
    )
    .join("\n"),
  sections: reference.modules.map(module_).join("\n"),
  moduleCount: reference.modules.length,
  itemCount: reference.modules.reduce((total, module) => total + module.items.length, 0),
});
