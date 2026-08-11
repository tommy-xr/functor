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
  // A card can be wanted for more than one reason at once (in view AND hovered),
  // so each is tracked by name: it plays while any reason holds and stops only
  // when the last one is released. Without that, moving the pointer off a card
  // that is still on screen would stop it — or, worse, a card hovered as it
  // scrolls away would never receive another observer callback and would animate
  // off screen forever.
  const reasons = new WeakMap<HTMLElement, Set<string>>();
  const reasonsFor = (card: HTMLElement) => {
    const existing = reasons.get(card);
    if (existing) return existing;
    const created = new Set<string>();
    reasons.set(card, created);
    return created;
  };

  const want = (card: HTMLElement, reason: string) => {
    reasonsFor(card).add(reason);
    const anim = card.querySelector<HTMLImageElement>(".game-anim");
    if (!anim) return;
    if (!anim.getAttribute("src") && anim.dataset.anim) {
      // A card whose art fails to load stays a poster rather than fading a
      // broken image over one, and never retries.
      anim.addEventListener("error", () => card.classList.remove("is-playing"), { once: true });
      anim.src = anim.dataset.anim;
    }
    card.classList.add("is-playing");
  };

  const release = (card: HTMLElement, reason: string) => {
    const held = reasonsFor(card);
    held.delete(reason);
    if (held.size === 0) card.classList.remove("is-playing");
  };

  const cards = [...rail.querySelectorAll<HTMLElement>(".game-card")];

  if ("IntersectionObserver" in window) {
    const io = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          const card = entry.target as HTMLElement;
          if (entry.isIntersecting) want(card, "view");
          else release(card, "view");
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

  // Pointer and keyboard both play immediately — a hovered or focused card
  // should not wait to satisfy the observer's threshold, and on a touch device
  // (no hover at all) the observer is the only thing that ever plays one.
  for (const card of cards) {
    card.addEventListener("pointerenter", () => want(card, "pointer"));
    card.addEventListener("pointerleave", () => release(card, "pointer"));
    card.addEventListener("focusin", () => want(card, "focus"));
    card.addEventListener("focusout", () => release(card, "focus"));
  }
}
