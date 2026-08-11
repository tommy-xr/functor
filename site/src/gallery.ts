// The games carousel's one job at runtime: play a card's box art. The cards
// themselves are prerendered by build.mjs, so if this never runs the shelf is
// still a scrollable, clickable, keyboard-navigable list of posters.
//
// A card's animation is fetched only when it is wanted — the card is well inside
// the viewport, or the pointer/keyboard is on it — so scrolling past the shelf
// costs one poster per card and nothing else. Under `prefers-reduced-motion` no
// animation is ever fetched or shown: the posters are the whole experience.
const rail = document.querySelector<HTMLElement>(".games-rail");

if (rail && !window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
  /**
   * Start the animation for a card, loading it on first use. The GIF sits in its
   * own image stacked over the poster and fades in once decoded, so a card never
   * flashes empty mid-swap and its box never resizes.
   */
  const play = (card: HTMLElement) => {
    const anim = card.querySelector<HTMLImageElement>(".game-anim");
    if (!anim) return;
    if (!anim.src && anim.dataset.anim) anim.src = anim.dataset.anim;
    card.classList.add("is-playing");
  };
  // Stopping only hides the animation; the loaded image is kept, so returning to
  // a card is instant and re-entry costs no fetch.
  const stop = (card: HTMLElement) => card.classList.remove("is-playing");

  const cards = [...rail.querySelectorAll<HTMLElement>(".game-card")];

  if ("IntersectionObserver" in window) {
    const io = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting) play(entry.target as HTMLElement);
          else stop(entry.target as HTMLElement);
        }
      },
      // Against the VIEWPORT, not the rail: intersection already accounts for
      // clipping by ancestors, so this covers both axes — a card is "in view"
      // only when the shelf has been scrolled to AND the card is in the rail's
      // visible run. (With the rail as root, every card in the horizontal run
      // counts as visible while the whole section is still below the fold, and
      // the page loads four animations nobody has seen.)
      { threshold: 0.75 }
    );
    for (const card of cards) io.observe(card);
  }

  // Pointer and keyboard both play immediately — a hovered or focused card should
  // not wait to satisfy the observer's threshold.
  for (const card of cards) {
    card.addEventListener("pointerenter", () => play(card));
    card.addEventListener("focusin", () => play(card));
  }
}
