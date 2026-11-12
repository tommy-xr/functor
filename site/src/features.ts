// Landing-page feature rows: reveal each row as it scrolls into focus. The CSS
// only hides rows once `.reveal` is present, so if this script never runs the
// rows stay fully visible (progressive enhancement).
const groups = document.querySelectorAll(".feature-rows");

if (groups.length && "IntersectionObserver" in window) {
  const io = new IntersectionObserver(
    (entries) => {
      for (const entry of entries) {
        if (!entry.isIntersecting) continue;
        entry.target.classList.add("in-view");
        io.unobserve(entry.target); // reveal once, then stop watching
      }
    },
    { threshold: 0.2, rootMargin: "0px 0px -12% 0px" }
  );
  for (const rows of groups) {
    rows.classList.add("reveal");
    for (const row of rows.querySelectorAll(".feature-row")) io.observe(row);
  }
}

// A nav link to a section of this page leaves the phone menu open over it: close it.
const toggle = document.querySelector<HTMLInputElement>("#nav-toggle");
for (const link of document.querySelectorAll('.site-nav a[href*="#"]')) {
  link.addEventListener("click", () => {
    if (toggle) toggle.checked = false;
  });
}
