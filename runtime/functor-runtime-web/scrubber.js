// Shared web time-travel timeline for the site player, browser IDE, CLI wasm
// server, and VS Code preview. Semantics live in timeline-model.js; this module
// is the imperative DOM/WASM shell.

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
import {
  PREVIEW_SECONDS_MAX,
  PREVIEW_SECONDS_MIN,
  TIMELINE_FPS,
  createTimelineState,
  deriveTimelineView,
  reduceTimeline,
  unitToFrame,
} from "./timeline-model.js";

const STYLE = `
#scrubber {
  --sb-bg: var(--scrub-bg, rgba(30, 24, 51, 0.92));
  --sb-line: var(--scrub-line, #2b2542);
  --sb-text: var(--scrub-text, #e9e6f2);
  --sb-dim: var(--scrub-dim, #9b94b3);
  --sb-accent: var(--scrub-accent, #41d8e6);
  --sb-future: var(--scrub-future, #e858b8);
  --sb-font: var(--scrub-font, ui-monospace, SFMono-Regular, Menlo, Consolas, monospace);
  position: fixed; left: 0; right: 0; bottom: 0; z-index: 10;
  display: none; align-items: center; gap: 8px; flex-wrap: nowrap;
  padding: 8px 12px 18px; color: var(--sb-text); background: var(--sb-bg);
  border-top: 1px solid var(--sb-line); box-shadow: 0 -3px 16px rgba(0, 0, 0, 0.35);
  font: 12px/1 var(--sb-font);
}
#scrubber button, #scrub-adv > summary {
  font: 14px/1 var(--sb-font); color: var(--sb-text); cursor: pointer;
  background: rgba(65, 216, 230, 0.10); border: 1px solid var(--sb-line);
  border-radius: 6px; padding: 6px 9px; box-shadow: 0 1px 3px rgba(0, 0, 0, 0.35);
  transition: box-shadow 0.12s ease, border-color 0.12s ease, transform 0.12s ease;
}
#scrubber button:hover, #scrub-adv > summary:hover {
  border-color: var(--sb-accent); box-shadow: 0 3px 10px rgba(0, 0, 0, 0.5);
  transform: translateY(-1px);
}
#scrubber button:active, #scrub-adv > summary:active {
  transform: translateY(0); box-shadow: 0 1px 2px rgba(0, 0, 0, 0.4);
}
#scrub-rail {
  position: relative; flex: 1; min-width: 80px; height: 30px; cursor: ew-resize;
  touch-action: none; user-select: none;
}
#scrub-timeline { position: absolute; inset: 0; width: 100%; height: 100%; overflow: visible; }
#scrub-track-bg { fill: rgba(155, 148, 179, 0.18); }
#scrub-recorded { fill: rgba(65, 216, 230, 0.30); }
#scrub-played { fill: var(--sb-accent); opacity: 0.62; }
#scrub-future { fill: var(--sb-future); opacity: 0.9; }
.scrub-handle {
  position: absolute; top: 15px; z-index: 3; width: 14px; height: 20px;
  box-sizing: border-box; padding: 0 !important; transform: translate(-50%, -50%) !important;
  border-radius: 4px !important; touch-action: none; cursor: ew-resize !important;
}
.scrub-handle:focus-visible { outline: 2px solid white; outline-offset: 2px; }
#scrub-playhead { background: var(--sb-accent); border-color: #b9f8ff; }
#scrub-preview-handle { background: var(--sb-future); border-color: #ffd0ee; }
#scrub-preview-handle.clipped { border-radius: 3px 0 0 3px !important; }
#scrub-overflow {
  position: absolute; right: 0; top: -5px; z-index: 4; display: none;
  padding: 2px 4px; border: 1px solid var(--sb-future); border-radius: 5px;
  color: #ffd0ee; background: rgba(30, 24, 51, 0.96); font-size: 9px;
  pointer-events: none;
}
#scrub-label {
  position: absolute; top: calc(100% + 2px); left: 50%; transform: translateX(-50%);
  color: var(--sb-dim); opacity: 0.78; font-size: 9px; white-space: nowrap;
  pointer-events: none;
}
#scrub-label .fut { color: var(--sb-future); }
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
  box-shadow: 0 8px 28px rgba(0, 0, 0, 0.5); white-space: nowrap;
}
#scrub-adv-pop label { display: flex; align-items: center; gap: 6px; justify-content: space-between; }
#scrub-adv-pop select, #scrub-adv-pop input[type="number"] {
  font: 12px/1 var(--sb-font); color: var(--sb-text); background: rgba(65, 216, 230, 0.10);
  border: 1px solid var(--sb-line); border-radius: 5px; padding: 4px 5px;
}
#scrub-adv-pop input[type="number"] { width: 46px; }
@media (max-width: 520px) {
  #scrubber { gap: 6px; padding: 7px 8px 17px; }
  #scrubber button, #scrub-adv > summary { padding: 6px 7px; }
  #scrub-rail { min-width: 48px; }
}
@media (max-width: 380px) {
  #scrubber { gap: 4px; }
  #scrubber button, #scrub-adv > summary { padding: 6px 5px; font-size: 13px; }
}`;

const HTML = `
  <button id="scrub-pause" title="Pause / resume">⏸</button>
  <button id="scrub-step" title="Step one frame forward">⏭</button>
  <span id="scrub-rail" aria-label="Time-travel timeline">
    <svg id="scrub-timeline" viewBox="0 0 1000 30" preserveAspectRatio="none" aria-hidden="true">
      <rect id="scrub-track-bg" x="0" y="12" width="1000" height="6" rx="3" />
      <rect id="scrub-recorded" x="0" y="12" width="1000" height="6" rx="3" />
      <rect id="scrub-played" x="0" y="12" width="0" height="6" rx="3" />
      <rect id="scrub-future" x="0" y="11" width="0" height="8" rx="3" />
    </svg>
    <button id="scrub-playhead" class="scrub-handle" role="slider"
      aria-label="Selected frame" aria-orientation="horizontal"></button>
    <button id="scrub-preview-handle" class="scrub-handle" role="slider"
      aria-label="Extrapolation endpoint" aria-orientation="horizontal"></button>
    <span id="scrub-overflow"></span>
    <span id="scrub-label"><span id="scrub-count"></span></span>
  </span>
  <button id="scrub-extrapolate" title="Extrapolate the paused game into the future">🔮</button>
  <details id="scrub-adv">
    <summary title="Extrapolation settings">⚙</summary>
    <div id="scrub-adv-pop">
      <label>show
        <select id="scrub-mode">
          <option value="1">trail</option><option value="2">strobe</option>
          <option value="3" selected>both</option><option value="4">ghost</option>
        </select>
      </label>
      <label>window <input id="scrub-win" type="number" step="0.5" min="0.5" max="5" value="2" />s</label>
      <label>rate <input id="scrub-rate" type="number" min="1" max="30" value="5" />/s</label>
    </div>
  </details>`;

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
  const rail = $("scrub-rail");
  const pause = $("scrub-pause");
  const step = $("scrub-step");
  const label = $("scrub-count");
  const played = $("scrub-played");
  const future = $("scrub-future");
  const playhead = $("scrub-playhead");
  const previewHandle = $("scrub-preview-handle");
  const overflow = $("scrub-overflow");
  const extrapolate = $("scrub-extrapolate");
  const mode = $("scrub-mode");
  const win = $("scrub-win");
  const rate = $("scrub-rate");

  let state = createTimelineState();
  let pendingSeek = null;
  let raf = 0;

  const dispatch = (action) => {
    state = reduceTimeline(state, action);
    render();
  };

  const view = () => deriveTimelineView(state);
  const canonicalConfig = () => state.preview;
  const pushPreview = () =>
    functor_lang_scrub_set_preview(state.preview.enabled ? Number(mode.value) : 0);
  const pushConfig = () => {
    const config = canonicalConfig();
    functor_lang_scrub_set_preview_config(config.seconds, config.rate);
  };
  const syncInputs = () => {
    win.value = String(state.preview.seconds);
    rate.value = String(state.preview.rate);
  };

  const requestSeek = (frame) => {
    dispatch({ type: "seek-requested", frame });
    pendingSeek = state.requestedFrame;
  };

  const frameAtPointer = (event) => {
    const current = view();
    if (!current) return 0;
    const rect = rail.getBoundingClientRect();
    const unit = rect.width > 0 ? (event.clientX - rect.left) / rect.width : 0;
    return unitToFrame(unit, current.viewport);
  };

  const render = () => {
    const current = view();
    if (!current) return;

    const playheadPct = current.playheadUnit * 100;
    const previewPct = current.previewEndUnit * 100;
    const futureWidth = Math.max(previewPct - playheadPct, 0);
    const previewVisible = state.preview.enabled && current.paused;

    playhead.style.left = `${playheadPct}%`;
    played.setAttribute("width", String(current.playheadUnit * 1000));
    future.setAttribute("x", String(current.playheadUnit * 1000));
    future.setAttribute("width", String(previewVisible ? futureWidth * 10 : 0));
    previewHandle.style.left = `${previewPct}%`;
    previewHandle.style.display = previewVisible ? "block" : "none";
    previewHandle.classList.toggle("clipped", current.previewClippedFrames > 0);

    overflow.style.display = previewVisible && current.previewClippedFrames > 0 ? "block" : "none";
    overflow.textContent = `+${current.previewClippedFrames}`;

    playhead.setAttribute("aria-valuemin", String(current.viewport.lo));
    playhead.setAttribute("aria-valuemax", String(current.viewport.hi));
    playhead.setAttribute("aria-valuenow", String(current.selectedFrame));
    playhead.setAttribute("aria-valuetext", `frame ${current.selectedFrame}`);

    previewHandle.setAttribute(
      "aria-valuemin",
      String(current.selectedFrame + Math.round(PREVIEW_SECONDS_MIN * TIMELINE_FPS))
    );
    previewHandle.setAttribute(
      "aria-valuemax",
      String(current.selectedFrame + Math.round(PREVIEW_SECONDS_MAX * TIMELINE_FPS))
    );
    previewHandle.setAttribute("aria-valuenow", String(current.previewEndFrame));
    previewHandle.setAttribute(
      "aria-valuetext",
      `${state.preview.seconds} seconds ahead` +
        (current.previewClippedFrames ? `, ${current.previewClippedFrames} frames clipped` : "")
    );

    label.innerHTML =
      `${current.selectedFrame}` +
      (state.preview.enabled ? ` <span class="fut">+${current.previewFrames}</span>` : "") +
      ` / ${current.viewport.hi}`;
    pause.textContent = current.paused ? "▶" : "⏸";
    extrapolate.classList.toggle("on", state.preview.enabled);
  };

  const beginAbsoluteDrag = (handle, move) => {
    handle.addEventListener("pointerdown", (event) => {
      event.preventDefault();
      event.stopPropagation();
      handle.setPointerCapture(event.pointerId);
      move(event, true);
    });
    handle.addEventListener("pointermove", (event) => {
      if (handle.hasPointerCapture(event.pointerId)) move(event, false);
    });
  };

  beginAbsoluteDrag(playhead, (event) => requestSeek(frameAtPointer(event)));

  let previewDrag = null;
  previewHandle.addEventListener("pointerdown", (event) => {
    event.preventDefault();
    event.stopPropagation();
    previewHandle.setPointerCapture(event.pointerId);
    previewDrag = { x: event.clientX, seconds: state.preview.seconds };
  });
  previewHandle.addEventListener("pointermove", (event) => {
    if (!previewDrag || !previewHandle.hasPointerCapture(event.pointerId)) return;
    const current = view();
    const width = rail.getBoundingClientRect().width;
    const span = current ? current.viewport.hi - current.viewport.lo : 0;
    if (width <= 0 || span <= 0) return;
    const deltaFrames = ((event.clientX - previewDrag.x) / width) * span;
    dispatch({
      type: "preview-changed",
      preview: { seconds: previewDrag.seconds + deltaFrames / TIMELINE_FPS },
    });
    syncInputs();
    pushConfig();
  });
  const endPreviewDrag = () => (previewDrag = null);
  previewHandle.addEventListener("pointerup", endPreviewDrag);
  previewHandle.addEventListener("pointercancel", endPreviewDrag);
  previewHandle.addEventListener("lostpointercapture", endPreviewDrag);

  rail.addEventListener("pointerdown", (event) => {
    if (event.target.closest(".scrub-handle")) return;
    event.preventDefault();
    rail.setPointerCapture(event.pointerId);
    requestSeek(frameAtPointer(event));
  });
  rail.addEventListener("pointermove", (event) => {
    if (rail.hasPointerCapture(event.pointerId)) requestSeek(frameAtPointer(event));
  });

  const seekKey = (event) => {
    const current = view();
    if (!current) return;
    const steps = event.shiftKey ? 10 : 1;
    const targets = {
      ArrowLeft: current.selectedFrame - steps,
      ArrowDown: current.selectedFrame - steps,
      ArrowRight: current.selectedFrame + steps,
      ArrowUp: current.selectedFrame + steps,
      PageDown: current.selectedFrame - TIMELINE_FPS,
      PageUp: current.selectedFrame + TIMELINE_FPS,
      Home: current.viewport.lo,
      End: current.viewport.hi,
    };
    if (!(event.key in targets)) return;
    event.preventDefault();
    requestSeek(targets[event.key]);
  };
  playhead.addEventListener("keydown", seekKey);

  previewHandle.addEventListener("keydown", (event) => {
    const steps = event.shiftKey ? TIMELINE_FPS : Math.round(TIMELINE_FPS / 2);
    const deltas = {
      ArrowLeft: -steps,
      ArrowDown: -steps,
      ArrowRight: steps,
      ArrowUp: steps,
      PageDown: -TIMELINE_FPS,
      PageUp: TIMELINE_FPS,
    };
    if (event.key === "Home" || event.key === "End") {
      event.preventDefault();
      dispatch({
        type: "preview-changed",
        preview: { seconds: event.key === "Home" ? PREVIEW_SECONDS_MIN : PREVIEW_SECONDS_MAX },
      });
    } else if (event.key in deltas) {
      event.preventDefault();
      dispatch({ type: "preview-delta-requested", frames: deltas[event.key] });
    } else {
      return;
    }
    syncInputs();
    pushConfig();
  });

  pause.addEventListener("click", () => functor_lang_scrub_toggle_pause());
  step.addEventListener("click", () => functor_lang_scrub_step());
  extrapolate.addEventListener("click", () => {
    dispatch({ type: "preview-changed", preview: { enabled: !state.preview.enabled } });
    pushPreview();
  });
  mode.addEventListener("change", pushPreview);

  const updateConfigFromInputs = () => {
    const seconds = win.valueAsNumber;
    const nextRate = rate.valueAsNumber;
    if (!win.validity.valid || !rate.validity.valid) return;
    dispatch({ type: "preview-changed", preview: { seconds, rate: nextRate } });
    pushConfig();
  };
  win.addEventListener("input", updateConfigFromInputs);
  rate.addEventListener("input", updateConfigFromInputs);
  win.addEventListener("change", syncInputs);
  rate.addEventListener("change", syncInputs);

  syncInputs();
  pushPreview();
  pushConfig();

  const seam = {
    paused: () => functor_lang_scrub_paused(),
    frame: () => functor_lang_scene_frame(),
    range: () => functor_lang_scene_range(),
    seek: requestSeek,
    togglePause: () => functor_lang_scrub_toggle_pause(),
    step: () => functor_lang_scrub_step(),
    model: () => state,
    view,
    setPreview: (preview) => {
      dispatch({ type: "preview-changed", preview });
      syncInputs();
      pushPreview();
      pushConfig();
    },
  };
  window.__scrub = seam;

  const update = () => {
    if (pendingSeek !== null) {
      functor_lang_seek_scene(pendingSeek);
      pendingSeek = null;
    }
    const range = functor_lang_scene_range();
    if (range.length === 2) {
      el.style.display = "flex";
      dispatch({
        type: "runtime-published",
        snapshot: {
          frame: functor_lang_scene_frame(),
          lo: range[0],
          hi: range[1],
          paused: functor_lang_scrub_paused(),
        },
      });
    } else {
      el.style.display = "none";
    }
    raf = requestAnimationFrame(update);
  };
  raf = requestAnimationFrame(update);

  return {
    destroy() {
      cancelAnimationFrame(raf);
      el.remove();
      if (window.__scrub === seam) delete window.__scrub;
    },
  };
}
