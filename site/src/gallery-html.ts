// The landing page's games carousel, rendered at BUILD time from the same
// `GALLERY` list that site/demos/box-art.mjs captures the media from (see
// src/examples.ts). It is prerendered rather than fetched so the cards are in the
// HTML a no-JS visitor gets — the only thing the runtime island adds is swapping
// each poster for its animation (src/gallery.ts).
//
// Every card is ONE link to the sandbox, so a card is one tab stop and the whole
// shelf is keyboard-reachable without any script.
import type { Example, ExampleGallery } from "./examples.js";

/**
 * One card, with its URLs already resolved. build.mjs derives this ONCE from
 * `GALLERY` and uses it for all three consumers — this markup, the emitted
 * `examples/gallery.json`, and the build's own check that the box art exists —
 * so the guard can never pass on paths the cards don't actually use.
 */
export interface GalleryCard {
  id: string;
  title: string;
  blurb: string;
  controls?: string;
  /** Where the card links, relative to the site root. */
  play: string;
  /** The still shown until the animation is wanted. */
  poster: string;
  /** The looping box art, swapped in by src/gallery.ts. */
  animation: string;
}

export const galleryCard = ({ id, gallery }: Example & { gallery: ExampleGallery }): GalleryCard => ({
  id,
  title: gallery.title,
  blurb: gallery.blurb,
  ...(gallery.controls ? { controls: gallery.controls } : {}),
  play: `sandbox.html?example=${encodeURIComponent(id)}`,
  poster: `media/box-art/${id}.png`,
  animation: `media/box-art/${id}.gif`,
});

const escapeHtml = (text: string): string =>
  text.replace(
    /[&<>"]/g,
    (char) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[char]!
  );

/**
 * The card list. The art is a POSTER plus a second, empty image the island fills
 * in with the animation — both carry explicit `width`/`height`, and the frame
 * holds a fixed aspect ratio, so a card occupies its final box before either
 * image has loaded (no layout shift, and none when the animation swaps in).
 *
 * The poster's alt is empty on purpose: the link already carries the game's
 * title, blurb and controls as text, so describing the image again only makes a
 * screen reader say the title twice.
 */
export const renderGalleryCards = (cards: GalleryCard[]): string =>
  cards
    .map(
      (card) => `        <li class="game-card">
          <a class="game-case" href="${escapeHtml(card.play)}">
            <span class="game-spine" aria-hidden="true"></span>
            <span class="game-art">
              <img
                class="game-poster"
                src="${escapeHtml(card.poster)}"
                width="320"
                height="200"
                alt=""
                loading="lazy"
                decoding="async"
              />
              <img
                class="game-anim"
                data-anim="${escapeHtml(card.animation)}"
                width="320"
                height="200"
                alt=""
                aria-hidden="true"
              />
            </span>
            <span class="game-meta">
              <span class="game-title">${escapeHtml(card.title)}</span>
              <span class="game-blurb">${escapeHtml(card.blurb)}</span>
              ${
                card.controls
                  ? `<span class="game-controls">${escapeHtml(card.controls)}</span>`
                  : ""
              }
              <span class="game-play">Play in the sandbox<span aria-hidden="true"> ▸</span></span>
            </span>
          </a>
        </li>`
    )
    .join("\n");
