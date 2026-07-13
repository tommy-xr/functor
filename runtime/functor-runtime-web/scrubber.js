// The shared time-travel scrubber — ONE component for every wasm-preview
// surface: the site's sandbox/IDE (site/player.html) and the CLI dev server /
// VSCode live-preview (index-functor-lang.html, served by `functor run wasm`).
// Both pages `import { mountScrubber }` from here instead of hand-rolling the
// controls, so the two can't drift (they used to — different themes AND
// different feature sets).
//
// It's a dependency-free ES module (no framework): the codebase is Rust +
// vanilla JS, and neither host page is bundled, so a plain module that both
// pages import — exactly like they already import ./pkg — is the natural fit.
// It's served alongside pkg/ (the CLI embeds + serves it; the site build copies
// it; `build wasm` writes it into dist/web).
//
// Look: the site's calm violet/cyan theme, everywhere. Styles are injected here
// (self-contained) using `var(--scrub-*, <calm default>)`, so a host that sets
// those vars (player.html, for its mouselook button) matches, and one that
// doesn't (the runtime page) still renders the same calm look via the fallbacks.
//
// Features: the full time-travel UI — pause/resume, a seek slider whose rail
// extends into a cyan FUTURE bar with a draggable end-cap (resize the forward
// window in place), step, an extrapolate toggle (🔮), and a ⚙ popover for the
// extrapolation mode / window / rate (docs/time-travel.md T3/T6/T6d).

import {
  functor_lang_scene_frame,
  functor_lang_scene_range,
  functor_lang_seek_scene,
  functor_lang_scrub_toggle_pause,
  functor_lang_scrub_step,
  functor_lang_scrub_paused,
  functor_lang_scrub_set_preview,
  functor_lang_scrub_set_preview_config,
} from "./pkg/functor_runtime_web.js";

// Calm site theme (site/styles.css :root) as the DEFAULTS, so the runtime page
// — which sets no vars — renders identically to the site.
const STYLE = `
#scrubber {
  --sb-bg: var(--scrub-bg, rgba(30, 24, 51, 0.92));
  --sb-line: var(--scrub-line, #2b2542);
  --sb-text: var(--scrub-text, #e9e6f2);
  --sb-dim: var(--scrub-dim, #9b94b3);
  --sb-accent: var(--scrub-accent, #41d8e6);
  /* The extrapolated-future color — synthwave pink, distinct from the cyan
     accent of the timeline you actually scrub, and matching the 🔮 toggle. */
  --sb-future: var(--scrub-future, #e858b8);
  --sb-font: var(--scrub-font, ui-monospace, SFMono-Regular, Menlo, Consolas, monospace);
  position: fixed; left: 0; right: 0; bottom: 0; z-index: 10;
  display: none; align-items: center; gap: 8px; flex-wrap: nowrap;
  font: 12px/1 var(--sb-font); color: var(--sb-text);
  /* Extra bottom padding leaves room for the frame counter, which hangs
     centered UNDER the rail. */
  padding: 8px 12px 18px; background: var(--sb-bg);
  border-top: 1px solid var(--sb-line);
  box-shadow: 0 -3px 16px rgba(0, 0, 0, 0.35); /* depth: lift off the canvas */
}
/* Buttons + the ⚙ summary share one raised, tactile treatment — a resting
   drop shadow, a lift on hover, a press on active. */
#scrubber button, #scrub-adv > summary {
  font: 14px/1 var(--sb-font); color: var(--sb-text); cursor: pointer;
  background: rgba(65, 216, 230, 0.10);
  border: 1px solid var(--sb-line); border-radius: 6px; padding: 6px 9px;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.35);
  transition: box-shadow 0.12s ease, border-color 0.12s ease, transform 0.12s ease;
}
#scrubber button:hover, #scrub-adv > summary:hover {
  border-color: var(--sb-accent); box-shadow: 0 3px 10px rgba(0, 0, 0, 0.5); transform: translateY(-1px);
}
#scrubber button:active, #scrub-adv > summary:active {
  transform: translateY(0); box-shadow: 0 1px 2px rgba(0, 0, 0, 0.4);
}
/* The timeline rail: the range input spans the recorded part; a translucent
   cyan strip marks [handle, handle + preview window] — the rail's domain
   extends past the recorded end while a preview is on. */
#scrub-rail { position: relative; display: flex; align-items: center; flex: 1; min-width: 60px; }
#scrub-slider { width: 100%; accent-color: var(--sb-accent); }
#scrub-future {
  position: absolute; height: 5px; top: 50%; transform: translateY(-50%);
  border-radius: 2px; background: var(--sb-future); opacity: 0.85;
  pointer-events: none; display: none;
}
/* The future segment's end cap: drag to resize the forward window in place.
   Hangs level with the track but is grabbable (the seek thumb owns the middle). */
#scrub-future-cap {
  position: absolute; width: 8px; height: 14px; top: 50%; transform: translateY(-50%);
  border-radius: 2px; background: var(--sb-future);
  cursor: ew-resize; display: none; touch-action: none;
}
#scrub-future-cap:hover { filter: brightness(1.3); }
/* Frame counter: tiny, faded, centered UNDER the rail — positioned, so digit
   growth never reflows the controls. */
#scrub-label {
  position: absolute; top: calc(100% + 4px); left: 50%; transform: translateX(-50%);
  color: var(--sb-dim); opacity: 0.7; font-size: 9px; white-space: nowrap; pointer-events: none;
}
#scrub-label .fut { color: var(--sb-future); }
/* Extrapolate toggle: a normal (clearly-clickable) button when OFF, LIT in the
   future's pink when on — never a greyed-out "disabled" look. */
#scrub-extrapolate.on {
  border-color: var(--sb-future);
  box-shadow: 0 0 0 1px var(--sb-future), 0 2px 10px rgba(232, 88, 184, 0.4);
}
#scrub-adv { position: relative; }
#scrub-adv > summary { list-style: none; user-select: none; }
#scrub-adv > summary::-webkit-details-marker { display: none; }
#scrub-adv[open] > summary { border-color: var(--sb-accent); }
#scrub-adv-pop {
  position: absolute; bottom: calc(100% + 10px); right: 0; z-index: 11;
  display: flex; flex-direction: column; gap: 8px; padding: 10px 12px;
  background: var(--sb-bg); border-radius: 8px; border: 1px solid var(--sb-line);
  box-shadow: 0 8px 28px rgba(0, 0, 0, 0.5); /* depth: floats above the strip */
  white-space: nowrap;
}
#scrub-adv-pop label { display: flex; align-items: center; gap: 6px; justify-content: space-between; }
#scrub-adv-pop select, #scrub-adv-pop input[type="number"] {
  font: 12px/1 var(--sb-font); color: var(--sb-text);
  background: rgba(65, 216, 230, 0.10); border: 1px solid var(--sb-line);
  border-radius: 5px; padding: 4px 5px;
}
#scrub-adv-pop input[type="number"] { width: 46px; }
/* Responsive: narrow iframes (the hero card, a phone). This page is an iframe,
   so the parent's media queries can't reach in — these run against the iframe's
   own viewport. Tighten gaps/padding and let the rail flex smaller so the whole
   strip fits without clipping. */
@media (max-width: 520px) {
  #scrubber { gap: 6px; padding: 7px 8px 17px; }
  #scrubber button, #scrub-adv > summary { padding: 6px 7px; }
  #scrub-rail { min-width: 36px; }
}
@media (max-width: 380px) {
  #scrubber { gap: 4px; }
  #scrubber button, #scrub-adv > summary { padding: 6px 5px; font-size: 13px; }
}`;

const HTML = `
  <button id="scrub-pause" title="Pause / resume">⏸</button>
  <button id="scrub-step" title="Step one frame forward">⏭</button>
  <span id="scrub-rail">
    <input id="scrub-slider" type="range" min="0" max="0" value="0" step="1" />
    <span id="scrub-future"></span>
    <span id="scrub-future-cap" title="Drag to resize the preview window"></span>
    <span id="scrub-label"><span id="scrub-count"></span></span>
  </span>
  <button id="scrub-extrapolate" title="Extrapolate: forward-simulate the paused game and overlay where everything is headed (docs/time-travel.md T6)">🔮</button>
  <details id="scrub-adv">
    <summary title="Extrapolation settings — what it shows, how far ahead, how densely">⚙</summary>
    <div id="scrub-adv-pop">
      <label title="What the extrapolation shows: trail dots, scene-space strobe copies, both, or the screen-space ghost compositor">show
        <select id="scrub-mode">
          <option value="1">trail</option>
          <option value="2">strobe</option>
          <option value="3" selected>both</option>
          <option value="4">ghost</option>
        </select>
      </label>
      <label title="How far ahead the preview projects">window
        <input id="scrub-win" type="number" step="0.5" min="0.5" max="5" value="2" />s
      </label>
      <label title="Strobe copies per second — the trail samples finer; density holds as the window resizes (ghost composites at most 8 total)">rate
        <input id="scrub-rate" type="number" min="1" max="30" value="5" />/s
      </label>
    </div>
  </details>`;

// Mount the scrubber into the page (creates a fixed bottom strip on document.body,
// hidden until history is recorded). Returns a handle with destroy().
export function mountScrubber() {
  if (!document.getElementById("functor-scrubber-style")) {
    const style = document.createElement("style");
    style.id = "functor-scrubber-style";
    style.textContent = STYLE;
    document.head.appendChild(style);
  }

  const el = document.createElement("div");
  el.id = "scrubber";
  el.innerHTML = HTML;
  document.body.appendChild(el);

  const $ = (id) => el.querySelector(`#${id}`);
  const label = $("scrub-count");
  const pause = $("scrub-pause");
  const slider = $("scrub-slider");
  const step = $("scrub-step");
  const future = $("scrub-future");
  const futureCap = $("scrub-future-cap");
  const extrapolate = $("scrub-extrapolate");
  const mode = $("scrub-mode");
  const win = $("scrub-win");
  const rate = $("scrub-rate");

  let scrubbing = false;
  let pendingSeek = null;
  // Hoisted so destroy() can remove it — it's a WINDOW listener, not on `el`.
  const onPointerUp = () => (scrubbing = false);
  slider.addEventListener("pointerdown", () => (scrubbing = true));
  window.addEventListener("pointerup", onPointerUp);
  slider.addEventListener("input", () => (pendingSeek = Number(slider.value)));
  pause.addEventListener("click", () => functor_lang_scrub_toggle_pause());
  step.addEventListener("click", () => functor_lang_scrub_step());

  // Future extrapolation (docs/time-travel.md T6/T6d): ONE toggle on the bar
  // (default OFF — opt in to see where things are headed); the mode / window /
  // rate live in the ⚙ popover. JS owns the UI state and pushes the EFFECTIVE
  // mode (0 while off). Pushed once at setup too, since a fresh runtime starts
  // at its own defaults.
  let extrapolating = false;
  const syncExtrapolateIcon = () => extrapolate.classList.toggle("on", extrapolating);
  const pushPreview = () => functor_lang_scrub_set_preview(extrapolating ? +mode.value : 0);
  const pushConfig = () => functor_lang_scrub_set_preview_config(+win.value, +rate.value);
  extrapolate.addEventListener("click", () => {
    extrapolating = !extrapolating;
    syncExtrapolateIcon();
    pushPreview();
  });
  mode.addEventListener("change", pushPreview);
  win.addEventListener("input", pushConfig);
  rate.addEventListener("input", pushConfig);
  syncExtrapolateIcon();
  pushPreview();
  pushConfig();

  // Drag the cyan segment's end cap to resize the forward window in place —
  // the ⚙ window input's direct-manipulation twin.
  let capDrag = null; // { x: pointerdown clientX, w: window at start }
  futureCap.addEventListener("pointerdown", (e) => {
    e.preventDefault();
    futureCap.setPointerCapture(e.pointerId);
    capDrag = { x: e.clientX, w: +win.value };
  });
  futureCap.addEventListener("pointermove", (e) => {
    if (!capDrag) return;
    const inset = 7;
    const travel = slider.offsetWidth - 2 * inset;
    const domain = +slider.max - +slider.min;
    if (travel <= 0 || domain <= 0) return;
    const framesPerPx = domain / travel;
    const w = Math.min(5, Math.max(0.5, capDrag.w + ((e.clientX - capDrag.x) * framesPerPx) / 60));
    win.value = w.toFixed(1);
    pushConfig();
  });
  futureCap.addEventListener("pointerup", () => (capDrag = null));

  // Headless test seam (e2e/site-sandbox.mjs): drive/observe without the DOM.
  const seam = {
    paused: () => functor_lang_scrub_paused(),
    frame: () => functor_lang_scene_frame(),
    range: () => functor_lang_scene_range(),
    seek: (f) => functor_lang_seek_scene(f),
    togglePause: () => functor_lang_scrub_toggle_pause(),
    step: () => functor_lang_scrub_step(),
  };
  window.__scrub = seam;

  let raf = 0;
  const update = () => {
    if (pendingSeek !== null) {
      functor_lang_seek_scene(pendingSeek);
      pendingSeek = null;
    }
    const range = functor_lang_scene_range(); // [] or [lo, hi]
    if (range.length === 2) {
      el.style.display = "flex";
      const lo = range[0];
      const hi = range[1];
      slider.min = lo;
      const frame = functor_lang_scene_frame();
      if (!scrubbing) slider.value = frame; // don't tug the handle mid-drag
      // The rail's domain includes the preview window: the handle can be
      // dragged INTO the cyan segment — a seek beyond the recorded end steps
      // the game forward, input-free (the runtime animates the catch-up).
      const span = hi - lo;
      const futureFrames = extrapolating ? Math.round(+win.value * 60) : 0;
      slider.max = hi + futureFrames;
      // `227 +120 / 227` — the predicted-frames count sits with the frame.
      label.innerHTML =
        `${frame | 0}` +
        (futureFrames > 0 ? ` <span class="fut">+${futureFrames}</span>` : "") +
        ` / ${hi | 0}`;
      pause.textContent = functor_lang_scrub_paused() ? "▶" : "⏸";
      if (futureFrames > 0 && span > 0) {
        // The cyan segment [handle, handle + window] plus its draggable end
        // cap, anchored to the thumb (including mid-drag) and starting at its
        // right edge, so the cyan never paints over the traversed track.
        const inset = 7;
        const travel = slider.offsetWidth - 2 * inset;
        const domain = hi + futureFrames - lo;
        const thumb = +slider.value;
        const thumbPx = inset + ((thumb - lo) / domain) * travel;
        const leftPx = thumbPx + inset;
        const endPx = Math.min(thumbPx + (futureFrames / domain) * travel, inset + travel);
        const widthPx = Math.max(endPx - leftPx, 0);
        future.style.left = `${leftPx}px`;
        future.style.width = `${widthPx}px`;
        future.style.display = "block";
        futureCap.style.left = `${endPx - 4}px`;
        futureCap.style.display = "block";
      } else {
        future.style.display = "none";
        futureCap.style.display = "none";
      }
    } else {
      el.style.display = "none"; // nothing recorded yet
    }
    raf = requestAnimationFrame(update);
  };
  raf = requestAnimationFrame(update);

  return {
    destroy() {
      cancelAnimationFrame(raf);
      window.removeEventListener("pointerup", onPointerUp);
      el.remove();
      if (window.__scrub === seam) delete window.__scrub;
    },
  };
}
