// The landing page's ONLY eager script: a ~200-byte loader whose whole job is
// to pull in the hero island. Everything heavy — the CodeMirror mini-editor and
// the player bridge — lives in ./hero-app.ts and arrives through this dynamic
// import, so the landing page's first paint depends on the static shell alone.
// The boot loader (static markup in index.html) covers the gap and the island
// dismisses it once the card is live.
//
// index.html carries a <link rel="modulepreload"> for the island, so the
// browser starts fetching it from the HTML parse rather than waiting for this
// module to execute — the split costs no round-trip. (That is also why the
// island's chunk name is stable and unhashed; site/build.mjs asserts the two
// agree.)
void import("./hero-app.js").catch((err: unknown) => {
  console.error("hero: the interactive island failed to load", err);
  // The scene itself is an iframe in the static markup and is very likely
  // running underneath. Retire the loader rather than let it claim "loading"
  // over a live card until the 20s CSS safety valve fires — the island is what
  // would otherwise dismiss it.
  document.querySelector("[data-fn-boot]")?.classList.add("is-done");
});
