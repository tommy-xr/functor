// The landing page's games carousel, rendered at BUILD time from the same
// `GALLERY` list that site/demos/box-art.mjs captures the media from (see
// src/examples.ts). It is prerendered rather than fetched so the cards are in the
// HTML a no-JS visitor gets — the only thing the runtime island adds is swapping
// each poster for its animation (src/gallery.ts).
//
// Every card is ONE link to the sandbox, so a card is one tab stop and the whole
// shelf is keyboard-reachable without any script.
import type { Example, ExampleGallery } from "./examples.js";

type GalleryEntry = Example & { gallery: ExampleGallery };

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
 */
export const renderGalleryCards = (entries: GalleryEntry[]): string =>
  entries
    .map(({ id, gallery }) => {
      const title = escapeHtml(gallery.title);
      const href = `sandbox.html?example=${encodeURIComponent(id)}`;
      return `        <li class="game-card">
          <a class="game-case" href="${href}">
            <span class="game-spine" aria-hidden="true"></span>
            <span class="game-art">
              <img
                class="game-poster"
                src="media/box-art/${id}.png"
                width="320"
                height="200"
                alt="${title} gameplay"
                loading="lazy"
                decoding="async"
              />
              <img
                class="game-anim"
                data-anim="media/box-art/${id}.gif"
                width="320"
                height="200"
                alt=""
                aria-hidden="true"
              />
            </span>
            <span class="game-meta">
              <span class="game-title">${title}</span>
              <span class="game-blurb">${escapeHtml(gallery.blurb)}</span>
              ${
                gallery.controls
                  ? `<span class="game-controls">${escapeHtml(gallery.controls)}</span>`
                  : ""
              }
              <span class="game-play">Play in the sandbox<span aria-hidden="true"> ▸</span></span>
            </span>
          </a>
        </li>`;
    })
    .join("\n");
