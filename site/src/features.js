// Landing-page feature rows: reveal each row as it scrolls into focus. The CSS
// only hides rows once `.reveal` is present, so if this script never runs the
// rows stay fully visible (progressive enhancement).
const rows = document.querySelector(".feature-rows");

if (rows && "IntersectionObserver" in window) {
  rows.classList.add("reveal");
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
  for (const row of rows.querySelectorAll(".feature-row")) io.observe(row);
}
