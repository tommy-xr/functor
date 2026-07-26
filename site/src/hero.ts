// The landing page's ONLY eager script: a few hundred bytes whose whole job is
// to start fetching the hero island. Everything heavy — the CodeMirror
// mini-editor, the player bridge, React — lives in ./hero-app.tsx and arrives
// through this dynamic import, so the landing page's first paint depends on the
// static shell alone. The boot loader (static markup in index.html) covers the
// gap and the island dismisses it once the player reports in.
//
// The import starts immediately rather than on idle: it is a fetch, not work on
// the main thread, so it does not delay the first paint, and the demo still
// comes up as early as the network allows. If it fails, the boot loader falls
// through to its 20s CSS safety valve — the same end state as an unreachable
// player.
void import("./hero-app.js").catch((err: unknown) => {
  console.error("hero: the interactive island failed to load", err);
});
