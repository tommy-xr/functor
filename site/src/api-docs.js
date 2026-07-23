import apiReference from "../generated/api-reference.json";

const moduleNav = document.querySelector("#api-module-nav");
const referenceRoot = document.querySelector("#api-reference");
const search = document.querySelector("#api-search");
const results = document.querySelector("#api-results");

const slug = (name) => `api-${name.toLowerCase().replace(/[^a-z0-9]+/g, "-")}`;

const appendInlineProse = (parent, prose) => {
  let cursor = 0;
  for (const match of prose.matchAll(/`([^`\n]+)`/g)) {
    parent.append(document.createTextNode(prose.slice(cursor, match.index)));
    const code = document.createElement("code");
    code.textContent = match[1];
    parent.append(code);
    cursor = match.index + match[0].length;
  }
  parent.append(document.createTextNode(prose.slice(cursor)));
};

const appendProse = (parent, prose) => {
  for (const paragraph of prose.split(/\n\s*\n/)) {
    const p = document.createElement("p");
    appendInlineProse(p, paragraph.replace(/\n/g, " "));
    parent.append(p);
  }
};

for (const module of apiReference.modules) {
  const moduleId = slug(module.name);
  const navLink = document.createElement("a");
  navLink.href = `#${moduleId}`;
  navLink.textContent = module.name;
  navLink.dataset.module = module.name.toLowerCase();
  moduleNav.append(navLink);

  const section = document.createElement("section");
  section.className = "api-module";
  section.id = moduleId;
  section.dataset.search = `${module.name} ${module.docs}`.toLowerCase();

  const heading = document.createElement("h2");
  const moduleName = document.createElement("span");
  moduleName.textContent = module.name;
  const count = document.createElement("span");
  count.className = "api-module-count";
  count.textContent = `${module.items.length} ${module.items.length === 1 ? "entry" : "entries"}`;
  heading.append(moduleName, count);
  section.append(heading);
  appendProse(section, module.docs);

  const items = document.createElement("div");
  items.className = "api-items";
  for (const item of module.items) {
    const article = document.createElement("article");
    article.className = "api-item";
    article.id = slug(item.qualified_name);
    article.dataset.search =
      `${item.qualified_name} ${item.name} ${item.kind} ${item.declaration} ${item.docs}`.toLowerCase();

    const itemHeading = document.createElement("h3");
    const anchor = document.createElement("a");
    anchor.href = `#${article.id}`;
    anchor.textContent = item.qualified_name;
    anchor.title = `Link to ${item.qualified_name}`;
    const kind = document.createElement("span");
    kind.className = `api-kind api-kind-${item.kind}`;
    kind.textContent = item.kind;
    itemHeading.append(anchor, kind);

    const declaration = document.createElement("pre");
    declaration.className = "api-declaration";
    const code = document.createElement("code");
    code.textContent = item.declaration;
    declaration.append(code);

    article.append(itemHeading, declaration);
    appendProse(article, item.docs);
    items.append(article);
  }
  section.append(items);
  referenceRoot.append(section);
}

document.querySelector("#api-module-count").textContent = apiReference.modules.length;
const totalItems = apiReference.modules.reduce((total, module) => total + module.items.length, 0);
document.querySelector("#api-item-count").textContent = totalItems;

const filterReference = () => {
  const query = search.value.trim().toLowerCase();
  let visibleItems = 0;
  let visibleModules = 0;

  for (const section of referenceRoot.querySelectorAll(".api-module")) {
    const moduleMatches = query && section.dataset.search.includes(query);
    let moduleItems = 0;
    for (const item of section.querySelectorAll(".api-item")) {
      const visible = !query || moduleMatches || item.dataset.search.includes(query);
      item.hidden = !visible;
      if (visible) moduleItems += 1;
    }
    section.hidden = moduleItems === 0;
    const navLink = moduleNav.querySelector(`[data-module="${section.id.slice(4)}"]`);
    if (navLink) navLink.hidden = moduleItems === 0;
    if (moduleItems > 0) {
      visibleModules += 1;
      visibleItems += moduleItems;
    }
  }

  results.textContent = query
    ? `${visibleItems} ${visibleItems === 1 ? "declaration" : "declarations"} in ${visibleModules} ${visibleModules === 1 ? "module" : "modules"}`
    : "";
};

search.addEventListener("input", filterReference);
document.addEventListener("keydown", (event) => {
  if (
    event.key === "/" &&
    document.activeElement !== search &&
    !["INPUT", "TEXTAREA"].includes(document.activeElement?.tagName)
  ) {
    event.preventDefault();
    search.focus();
  }
  if (event.key === "Escape" && document.activeElement === search) {
    search.value = "";
    filterReference();
    search.blur();
  }
});
