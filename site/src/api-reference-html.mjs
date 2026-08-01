// Render the generated API reference to static HTML at build time.
//
// The reference used to be built in the browser from the bundled JSON, which
// meant a plain (no-JS) GET of /docs/ returned only the page shell — invisible
// to the LLM agents Functor is meant to be readable by. This module is the
// single renderer: `build.mjs` calls it to bake the real markup into
// `dist/docs/index.html`, and `src/api-docs.ts` only enhances that markup
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

// A category label repeats across groups (both halves have an "Input"), so
// everything the filter keys on is the group-qualified pair, not the label.
const categoryKey = (module) => `${module.group ?? "engine"}:${module.category ?? ""}`;

const module_ = (module) => {
  const id = slug(module.name);
  const entries = `${module.items.length} ${module.items.length === 1 ? "entry" : "entries"}`;
  return [
    `<section class="api-module" id="${escapeAttr(id)}" data-group="${escapeAttr(module.group ?? "engine")}" data-category="${escapeAttr(categoryKey(module))}" data-search="${escapeAttr(`${module.name} ${module.docs}`.toLowerCase())}">`,
    `<h2><span>${escapeText(module.name)}</span><span class="api-module-count">${entries}</span></h2>`,
    prose(module.docs),
    `<div class="api-items">`,
    module.items.map(item).join("\n"),
    `</div>`,
    `</section>`,
  ].join("\n");
};

const GROUP_LABELS = { engine: "Engine", stdlib: "Standard library" };

const navLink = (module) =>
  `<a href="#${escapeAttr(slug(module.name))}" data-module="${escapeAttr(module.name.toLowerCase())}">${escapeText(module.name)}</a>`;

// The nav is grouped so the two halves of the API — what the game runner
// provides, and what the language itself ships — are visibly distinct, and
// each group is subdivided by category so a 37-module list reads as a handful
// of topics. Both levels are single elements so the search filter can hide a
// whole empty category, and a whole empty group.
const navGroups = (modules) => {
  const groups = [];
  for (const module of modules) {
    const group = module.group ?? "engine";
    if (groups.at(-1)?.group !== group) groups.push({ group, categories: [] });
    const categories = groups.at(-1).categories;
    const key = categoryKey(module);
    if (categories.at(-1)?.key !== key)
      categories.push({ key, label: module.category ?? "", modules: [] });
    categories.at(-1).modules.push(module);
  }
  return groups
    .map((entry) =>
      [
        `<div class="api-nav-group" data-group="${escapeAttr(entry.group)}">`,
        `<div class="api-nav-group-label">${escapeText(GROUP_LABELS[entry.group] ?? entry.group)}</div>`,
        entry.categories
          .map((category) =>
            [
              `<div class="api-nav-category" data-group="${escapeAttr(entry.group)}" data-category="${escapeAttr(category.key)}">`,
              `<div class="api-nav-category-label">${escapeText(category.label)}</div>`,
              category.modules.map(navLink).join("\n"),
              `</div>`,
            ].join("\n"),
          )
          .join("\n"),
        `</div>`,
      ].join("\n"),
    )
    .join("\n");
};

const GROUP_BLURBS = {
  engine: "Provided by the game runner — available to a hosted <code>.fun</code> game.",
  stdlib:
    "Ships with the language — available in every Functor Lang program, runner or not.",
};

// A labelled divider announces each group in the scrolling reference. It is
// presentational (the headings that structure the page are the module ones),
// and api-docs.ts hides it when the filter empties its group.
const divider = (group) =>
  [
    `<div class="api-group-divider" data-group="${escapeAttr(group)}">`,
    `<h2>${escapeText(GROUP_LABELS[group] ?? group)}</h2>`,
    `<p>${GROUP_BLURBS[group] ?? ""}</p>`,
    `</div>`,
  ].join("\n");

// Inside a group, each category announces itself the same way — one level
// quieter, and hidden by the filter when its last module goes.
const categoryHeading = (module) =>
  [
    `<div class="api-category-divider" data-group="${escapeAttr(module.group ?? "engine")}" data-category="${escapeAttr(categoryKey(module))}">`,
    `<h2>${escapeText(module.category ?? "")}</h2>`,
    `</div>`,
  ].join("\n");

const sections = (modules) => {
  const out = [];
  let group;
  let category;
  for (const module of modules) {
    const moduleGroup = module.group ?? "engine";
    if (moduleGroup !== group) {
      group = moduleGroup;
      category = undefined;
      out.push(divider(group));
    }
    if (categoryKey(module) !== category) {
      category = categoryKey(module);
      out.push(categoryHeading(module));
    }
    out.push(module_(module));
  }
  return out.join("\n");
};

/** The static markup for the reference page: module nav, sections, and counts. */
export const renderApiReference = (reference) => ({
  nav: navGroups(reference.modules),
  sections: sections(reference.modules),
  moduleCount: reference.modules.length,
  itemCount: reference.modules.reduce((total, module) => total + module.items.length, 0),
});
