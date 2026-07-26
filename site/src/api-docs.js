// Progressive enhancement for the prerendered API reference: the markup for
// every module and declaration is baked into docs/index.html at build time by
// site/build.mjs, so the page is complete without JavaScript (and readable by
// any plain HTTP fetch). This script only adds the live search/filter.

const moduleNav = document.querySelector("#api-module-nav");
const referenceRoot = document.querySelector("#api-reference");
const search = document.querySelector("#api-search");
const results = document.querySelector("#api-results");

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
