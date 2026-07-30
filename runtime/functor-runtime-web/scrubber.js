// Shared web time-travel timeline for the site player, browser IDE, CLI wasm
// server, and VS Code preview. Semantics live in timeline-model.js; this module
// is the imperative DOM/WASM shell.

import {
  functor_lang_scene_frame,
  functor_lang_scene_generation,
  functor_lang_scene_range,
  functor_lang_seek_scene,
  functor_lang_scrub_seek_result,
  functor_lang_scrub_toggle_pause,
  functor_lang_scrub_step,
  functor_lang_scrub_paused,
  functor_lang_scrub_set_preview,
  functor_lang_scrub_set_preview_config,
  functor_lang_timeline_events,
  functor_lang_timeline_events_gen,
  functor_lang_viewer_detached,
  functor_lang_viewer_detached_generation,
  functor_lang_viewer_authored_frustum,
  functor_lang_viewer_fov,
  functor_lang_viewer_game_ui,
  functor_lang_viewer_look,
  functor_lang_viewer_material,
  functor_lang_viewer_mode,
  functor_lang_viewer_move,
  functor_lang_viewer_physics,
  functor_lang_viewer_reset,
  functor_lang_viewer_set_authored_frustum,
  functor_lang_viewer_set_fov,
  functor_lang_viewer_set_game_ui,
  functor_lang_viewer_set_material,
  functor_lang_viewer_set_mode,
  functor_lang_viewer_set_physics,
  functor_lang_viewer_toggle_detached,
  functor_lang_viewer_zoom_2d,
  functor_lang_viewer_zoom,
} from "./pkg/functor_runtime_web.js";
import {
  PREVIEW_SECONDS_MAX,
  PREVIEW_SECONDS_MIN,
  TIMELINE_FPS,
  createTimelineState,
  describeRecordedAvailability,
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
  /* Docked TOP: the transport instrument sits above the scene it governs,
     matching the sandbox's chrono bar — one product-wide rule. */
  position: fixed; left: 0; right: 0; top: 0; z-index: 10;
  display: none; flex-direction: column; align-items: stretch; gap: 0;
  padding: 8px 12px 18px; color: var(--sb-text); background: var(--sb-bg);
  border-bottom: 1px solid var(--sb-line);
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.5), 0 14px 36px rgba(0, 0, 0, 0.38);
  font: 12px/1 var(--sb-font);
}
#scrub-main {
  display: flex; align-items: center; gap: 8px; flex-wrap: nowrap;
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
  /* The handles overhang the rail ends by half their width; reserve that so a
     fully-out playhead never crowds the step / extrapolate buttons. */
  margin: 0 7px;
}
#scrub-timeline { position: absolute; inset: 0; width: 100%; height: 100%; overflow: visible; }
#scrub-track-bg { fill: rgba(155, 148, 179, 0.18); }
.scrub-unavailable { fill: url(#scrub-unavailable-pattern); }
#scrub-recorded { fill: rgba(65, 216, 230, 0.30); }
#scrub-played { fill: var(--sb-accent); opacity: 0.62; }
#scrub-future { fill: var(--sb-future); opacity: 0.9; }
.scrub-event { pointer-events: none; }
.scrub-event.input { fill: #ffd166; fill-opacity: 0.75; }
.scrub-event.reload { fill: #b994ff; }
.scrub-event.reload-error { fill: #ff6b7d; }
.scrub-tick { fill: var(--sb-text); fill-opacity: 0.28; pointer-events: none; }
.scrub-tick.major { fill: var(--sb-text); fill-opacity: 0.5; }
.scrub-event-hit { cursor: pointer; outline: none; }
.scrub-event-hit.active .scrub-event,
.scrub-event-hit:focus .scrub-event { stroke: white; stroke-width: 2; }
.scrub-handle {
  position: absolute; top: 15px; z-index: 3; width: 14px; height: 20px;
  box-sizing: border-box; padding: 0 !important; transform: translate(-50%, -50%) !important;
  border-radius: 4px !important; touch-action: none; cursor: ew-resize !important;
}
.scrub-handle:focus-visible { outline: 2px solid white; outline-offset: 2px; }
#scrubber #scrub-playhead { background: var(--sb-accent); border-color: #b9f8ff; }
#scrubber #scrub-preview-handle { background: var(--sb-future); border-color: #ffd0ee; }
#scrub-preview-handle.clipped { border-radius: 3px 0 0 3px !important; }
#scrub-preview-handle.fully-clipped {
  top: 0; z-index: 5; height: 12px;
}
#scrub-playhead.outside { box-shadow: 0 0 0 2px rgba(255, 255, 255, 0.55); }
#scrub-overflow {
  position: absolute; right: 0; top: -5px; z-index: 4; display: none;
  padding: 2px 4px; border: 1px solid var(--sb-future); border-radius: 5px;
  color: #ffd0ee; background: rgba(30, 24, 51, 0.96); font-size: 9px;
  pointer-events: none;
}
#scrub-event-detail {
  /* Below the frame label (which hangs at 100%+2px..+11px). */
  position: absolute; z-index: 12; top: calc(100% + 13px); display: none;
  max-width: min(280px, 80vw); padding: 5px 7px; transform: translateX(-50%);
  border: 1px solid var(--sb-line); border-radius: 6px; color: var(--sb-text);
  background: rgba(30, 24, 51, 0.98); box-shadow: 0 4px 16px rgba(0, 0, 0, 0.45);
  font-size: 10px; line-height: 1.3; white-space: nowrap; overflow: hidden;
  text-overflow: ellipsis; pointer-events: none;
}
#scrub-label {
  position: absolute; top: calc(100% + 2px); left: 50%; transform: translateX(-50%);
  color: var(--sb-dim); opacity: 0.78; font-size: 9px; white-space: nowrap;
  pointer-events: none;
}
#scrub-label .fut { color: var(--sb-future); }
#scrub-label .out { color: #ffd166; }
#scrub-extrapolate.on {
  border-color: var(--sb-future);
  box-shadow: 0 0 0 1px var(--sb-future), 0 2px 10px rgba(232, 88, 184, 0.4);
}
#scrub-camera.on {
  border-color: var(--sb-accent);
  box-shadow: 0 0 0 1px var(--sb-accent), 0 2px 10px rgba(65, 216, 230, 0.35);
}
#scrub-debug {
  display: none; align-items: center; gap: 12px; flex-wrap: wrap;
  margin-top: 8px; padding-top: 8px; border-top: 1px solid var(--sb-line);
}
#scrubber.debug-active #scrub-debug { display: flex; }
#scrub-debug-title {
  color: var(--sb-accent); font-weight: 700; letter-spacing: 0.04em;
}
#scrub-debug label {
  display: inline-flex; align-items: center; gap: 6px; color: var(--sb-dim);
}
#scrub-debug label[hidden] { display: none; }
#scrub-debug select, #scrub-debug input[type="range"] {
  font: 12px/1 var(--sb-font); color: var(--sb-text);
  accent-color: var(--sb-accent); background: rgba(65, 216, 230, 0.10);
}
#scrub-debug select {
  border: 1px solid var(--sb-line); border-radius: 5px; padding: 4px 5px;
}
#scrub-debug input[type="checkbox"] { accent-color: var(--sb-accent); }
#scrub-debug-fov { width: 110px; }
#scrub-debug-lens { color: var(--sb-text); min-width: 42px; }
#scrub-adv { position: relative; }
#scrub-adv > summary { list-style: none; user-select: none; }
#scrub-adv > summary::-webkit-details-marker { display: none; }
#scrub-adv[open] > summary { border-color: var(--sb-accent); }
#scrub-adv-pop {
  position: absolute; top: calc(100% + 10px); right: 0; z-index: 11;
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
  #scrubber { padding: 7px 8px 17px; }
  #scrub-main { gap: 6px; }
  #scrub-debug { gap: 8px; }
  #scrubber button, #scrub-adv > summary { padding: 6px 7px; }
  #scrub-rail { min-width: 48px; }
}
@media (max-width: 380px) {
  #scrub-main { gap: 4px; }
  #scrubber button, #scrub-adv > summary { padding: 6px 5px; font-size: 13px; }
}`;

const HTML = `
  <div id="scrub-main">
    <button id="scrub-pause" title="Pause / resume">⏸</button>
    <button id="scrub-camera" title="Open the debug camera" hidden>📷</button>
    <button id="scrub-step" title="Step one frame forward">⏭</button>
    <span id="scrub-rail" aria-label="Time-travel timeline" title="Drag to seek">
    <svg id="scrub-timeline" viewBox="0 0 1000 30" preserveAspectRatio="none"
      role="group" aria-label="Timeline event markers">
      <defs>
        <pattern id="scrub-unavailable-pattern" width="12" height="12"
          patternUnits="userSpaceOnUse" patternTransform="rotate(20)">
          <rect width="12" height="12" fill="rgba(8, 7, 14, 0.72)" />
          <rect width="4" height="12" fill="rgba(155, 148, 179, 0.30)" />
        </pattern>
      </defs>
      <rect id="scrub-track-bg" x="0" y="12" width="1000" height="6" rx="3" aria-hidden="true" />
      <rect id="scrub-unavailable" class="scrub-unavailable" x="0" y="12" width="0" height="6" rx="3" aria-hidden="true" />
      <rect id="scrub-unavailable-after" class="scrub-unavailable" x="1000" y="12" width="0" height="6" rx="3" aria-hidden="true" />
      <rect id="scrub-recorded" x="0" y="12" width="1000" height="6" rx="3" aria-hidden="true" />
      <rect id="scrub-played" x="0" y="12" width="0" height="6" rx="3" aria-hidden="true" />
      <rect id="scrub-future" x="0" y="11" width="0" height="8" rx="3" aria-hidden="true" />
      <g id="scrub-ticks" aria-hidden="true"></g>
      <g id="scrub-events" aria-label="Recorded events"></g>
    </svg>
    <button id="scrub-playhead" class="scrub-handle" role="slider"
      aria-label="Selected frame" aria-orientation="horizontal"></button>
    <button id="scrub-preview-handle" class="scrub-handle" role="slider"
      aria-label="Extrapolation endpoint" aria-orientation="horizontal"></button>
    <span id="scrub-overflow"></span>
    <span id="scrub-event-detail" role="status"></span>
    <span id="scrub-label"><span id="scrub-count"></span></span>
    </span>
    <button id="scrub-extrapolate" title="Extrapolate the game into the future">🔮</button>
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
    </details>
  </div>
  <section id="scrub-debug" aria-label="Debug Camera controls">
    <span id="scrub-debug-title">Debug Camera</span>
    <label>View
      <select id="scrub-debug-mode">
        <option value="0">FPS</option>
        <option value="1">Orbit</option>
      </select>
      <span id="scrub-debug-pan2d" hidden>Pan 2D</span>
    </label>
    <label id="scrub-debug-lens-label"><span id="scrub-debug-lens-name">FOV</span>
      <input id="scrub-debug-fov" type="range" min="15" max="120" step="1" value="60" />
      <output id="scrub-debug-lens">60°</output>
    </label>
    <label id="scrub-debug-material-label">Material
      <select id="scrub-debug-material">
        <option value="0">Shaded</option>
        <option value="3">Transparent</option>
        <option value="1">Normals</option>
        <option value="2">Tangents</option>
      </select>
    </label>
    <label id="scrub-debug-physics-label"><input id="scrub-debug-physics" type="checkbox" /> Physics</label>
    <label id="scrub-debug-frustum-label"><input id="scrub-debug-frustum" type="checkbox" /> Authored frustum</label>
    <label><input id="scrub-debug-game-ui" type="checkbox" checked /> Game UI</label>
    <button id="scrub-debug-reset" type="button">Reset</button>
  </section>`;

// `hidden: true` mounts the SEAM without the chrome: the timeline model, the
// runtime poll loop, and `window.__scrub` all run exactly as usual, but the
// bar's DOM is never attached to the document — so nothing renders and
// nothing enters the accessibility tree. This is for host pages that dock
// their own transport UI over the seam (the sandbox's chrono bar) — an
// honest replacement for hiding the bar with injected CSS.
export function mountScrubber({ hidden = false } = {}) {
  if (!hidden && !document.getElementById("functor-scrubber-style")) {
    const style = document.createElement("style");
    style.id = "functor-scrubber-style";
    style.textContent = STYLE;
    document.head.appendChild(style);
  }

  // The element is always BUILT (every internal lookup and render targets
  // it); when hidden it simply stays detached.
  const el = document.createElement("div");
  el.id = "scrubber";
  el.innerHTML = HTML;
  if (!hidden) {
    document.body.appendChild(el);
    // Reserve the bar's height from MOUNT (not first display): host pages lay
    // the canvas out below `--functor-scrubber-h`, so the top-docked bar
    // never occludes in-game UI (Ui.topLeft anchors) and the layout never
    // jumps when a recording appears. 57px = 30px rail + 8/18px padding + 1px
    // border (the bar's fixed metrics above).
    document.documentElement.style.setProperty("--functor-scrubber-h", "57px");
  }

  const $ = (id) => el.querySelector(`#${id}`);
  const rail = $("scrub-rail");
  const pause = $("scrub-pause");
  const camera = $("scrub-camera");
  const step = $("scrub-step");
  const label = $("scrub-count");
  const unavailable = $("scrub-unavailable");
  const unavailableAfter = $("scrub-unavailable-after");
  const recorded = $("scrub-recorded");
  const played = $("scrub-played");
  const future = $("scrub-future");
  const playhead = $("scrub-playhead");
  const previewHandle = $("scrub-preview-handle");
  const overflow = $("scrub-overflow");
  const eventDetail = $("scrub-event-detail");
  const eventLayer = $("scrub-events");
  const extrapolate = $("scrub-extrapolate");
  const mode = $("scrub-mode");
  const win = $("scrub-win");
  const rate = $("scrub-rate");
  const debugMode = $("scrub-debug-mode");
  const debugPan2d = $("scrub-debug-pan2d");
  const debugFov = $("scrub-debug-fov");
  const debugLensName = $("scrub-debug-lens-name");
  const debugLens = $("scrub-debug-lens");
  const debugMaterial = $("scrub-debug-material");
  const debugMaterialLabel = $("scrub-debug-material-label");
  const debugPhysics = $("scrub-debug-physics");
  const debugPhysicsLabel = $("scrub-debug-physics-label");
  const debugFrustum = $("scrub-debug-frustum");
  const debugFrustumLabel = $("scrub-debug-frustum-label");
  const debugGameUi = $("scrub-debug-game-ui");
  const debugReset = $("scrub-debug-reset");

  let state = createTimelineState();
  let pendingSeek = null;
  let nextSeekId = 1;
  let lastSeekResultId = null;
  let lastEventsGeneration = null;
  let lastRuntimeSnapshotKey = "";
  let lastDebugSnapshotKey = "";
  let detachedActive = false;
  let pendingDetachedGeneration = null;
  let pendingPointerLock = false;
  let raf = 0;
  const markerNodes = new Map();

  const dispatch = (action) => {
    state = reduceTimeline(state, action);
    // Hidden mounts skip ALL rendering: state is owned by reduceTimeline and
    // the seam reads it directly, so the detached DOM never needs painting.
    if (!hidden) render();
  };

  const view = () => deriveTimelineView(state);
  const canonicalConfig = () => state.preview;
  const debugCamera = () => ({
    mode: functor_lang_viewer_mode(),
    material: functor_lang_viewer_material(),
    physics: functor_lang_viewer_physics(),
    authoredFrustum: functor_lang_viewer_authored_frustum(),
    gameUi: functor_lang_viewer_game_ui(),
    fov: functor_lang_viewer_fov(),
    zoom2d: functor_lang_viewer_zoom_2d(),
  });
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
    const id = nextSeekId++;
    dispatch({ type: "seek-requested", id, frame });
    if (state.requestedSeekId === id) {
      pendingSeek = { id, frame: state.requestedFrame };
    }
  };
  const flushPendingSeek = () => {
    if (pendingSeek === null) return;
    functor_lang_seek_scene(pendingSeek.frame, pendingSeek.id);
    pendingSeek = null;
  };

  const frameAtPointer = (event) => {
    const current = view();
    if (!current) return 0;
    const rect = rail.getBoundingClientRect();
    const unit = rect.width > 0 ? (event.clientX - rect.left) / rect.width : 0;
    return unitToFrame(unit, current.viewport);
  };

  const renderMarkers = (current) => {
    const ns = "http://www.w3.org/2000/svg";
    const desiredIds = new Set(current.eventMarkers.map((marker) => marker.id));
    for (const [id, group] of markerNodes) {
      if (!desiredIds.has(id)) {
        group.remove();
        markerNodes.delete(id);
      }
    }

    let nextChild = eventLayer.firstElementChild;
    for (const marker of current.eventMarkers) {
      let group = markerNodes.get(marker.id);
      if (!group) {
        group = document.createElementNS(ns, "g");
        const tick = document.createElementNS(ns, "rect");
        const hit = document.createElementNS(ns, "rect");
        const reload = marker.category === "reload";

        group.setAttribute("class", "scrub-event-hit");
        group.setAttribute("role", "button");
        group.setAttribute("tabindex", "0");
        group.dataset.eventId = String(marker.id);

        hit.setAttribute("x", "-9");
        hit.setAttribute("y", "0");
        hit.setAttribute("width", "18");
        hit.setAttribute("height", "30");
        hit.setAttribute("fill", "transparent");

        // Full-height lines across the rail (thin amber input, heavier
        // reload/error), matching the host chrono bar's markers.
        tick.setAttribute("x", String(-(reload ? 1.5 : 1)));
        tick.setAttribute("y", reload ? "2" : "3");
        tick.setAttribute("width", reload ? "3" : "2");
        tick.setAttribute("height", reload ? "26" : "24");
        tick.setAttribute("rx", "1");
        tick.setAttribute(
          "class",
          `scrub-event ${marker.category}${marker.kind === "reload-error" ? " reload-error" : ""}`
        );

        const activate = () => {
          dispatch({ type: "event-selected", id: marker.id });
          requestSeek(marker.frame);
        };
        group.addEventListener("mouseenter", () => dispatch({ type: "event-hovered", id: marker.id }));
        group.addEventListener("mouseleave", () => dispatch({ type: "event-hovered", id: null }));
        group.addEventListener("focus", () => dispatch({ type: "event-hovered", id: marker.id }));
        group.addEventListener("blur", () => dispatch({ type: "event-hovered", id: null }));
        group.addEventListener("pointerdown", (event) => event.stopPropagation());
        group.addEventListener("click", (event) => {
          event.stopPropagation();
          activate();
        });
        group.addEventListener("keydown", (event) => {
          if (event.key === "Enter" || event.key === " ") {
            event.preventDefault();
            activate();
          } else if (event.key === "Escape") {
            dispatch({ type: "event-selected", id: null });
          }
        });
        group.append(hit, tick);
        markerNodes.set(marker.id, group);
      }

      const suffix = marker.count > 1 ? `, ${marker.count} nearby events` : "";
      group.setAttribute("aria-label", `frame ${marker.frame}, ${marker.labels[0]}${suffix}`);
      group.setAttribute("transform", `translate(${marker.unit * 1000} 0)`);
      // Retain nodes/listeners and touch the DOM only when clustering changes
      // their chronological keyboard-navigation order.
      if (group === nextChild) {
        nextChild = nextChild.nextElementSibling;
      } else {
        eventLayer.insertBefore(group, nextChild);
      }
    }

    for (const group of eventLayer.querySelectorAll(".scrub-event-hit")) {
      const id = Number(group.dataset.eventId);
      group.classList.toggle(
        "active",
        id === current.selectedEventId || id === current.hoveredEventId
      );
    }

    if (current.activeEvent) {
      eventDetail.style.display = "block";
      eventDetail.style.left = `${Math.min(95, Math.max(5, current.activeEvent.unit * 100))}%`;
      const count = current.activeEvent.count > 1 ? ` · ${current.activeEvent.count} events` : "";
      const detail = `frame ${current.activeEvent.frame} · ${current.activeEvent.labels[0]}${count}`;
      if (eventDetail.textContent !== detail) eventDetail.textContent = detail;
    } else {
      eventDetail.style.display = "none";
    }
  };

  // Ticks along the rail: one per second (TIMELINE_FPS frames), heavier every
  // 5s. If a viewport ever spans enough seconds that ticks would smear
  // together, the step widens (5s/30s) to keep them ≥ ~20 viewBox units
  // apart. Skipped entirely when (lo, span) are unchanged since last render —
  // the common paused case — so steady-state cost is one key comparison.
  const ticksLayer = $("scrub-ticks");
  let lastTickKey = "";
  const renderTicks = (current) => {
    const ns = "http://www.w3.org/2000/svg";
    const lo = current.viewport.lo;
    const span = Math.max(current.viewport.hi - lo, 1);
    const perSecond = TIMELINE_FPS;
    const step =
      span / perSecond <= 50 ? perSecond : span / perSecond <= 250 ? 5 * perSecond : 30 * perSecond;
    const tickKey = `${lo}:${span}:${step}`;
    if (tickKey === lastTickKey) return;
    lastTickKey = tickKey;
    const frames = [];
    for (let f = Math.ceil(lo / step) * step; f <= current.viewport.hi; f += step) frames.push(f);
    while (ticksLayer.children.length > frames.length) ticksLayer.lastElementChild.remove();
    while (ticksLayer.children.length < frames.length) {
      const rect = document.createElementNS(ns, "rect");
      rect.setAttribute("width", "1.5");
      ticksLayer.appendChild(rect);
    }
    frames.forEach((frame, index) => {
      const rect = ticksLayer.children[index];
      const major = frame % (5 * step) === 0;
      rect.setAttribute("class", major ? "scrub-tick major" : "scrub-tick");
      rect.setAttribute("x", String(((frame - lo) / span) * 1000));
      rect.setAttribute("y", major ? "9" : "11");
      rect.setAttribute("height", major ? "12" : "8");
    });
  };

  const render = () => {
    const current = view();
    if (!current) return;

    const playheadPct = current.playheadUnit * 100;
    const previewPct = current.previewEndUnit * 100;
    const futureWidth = Math.max(previewPct - playheadPct, 0);
    const previewVisible = state.preview.enabled;

    playhead.style.left = `${playheadPct}%`;
    playhead.style.display = "block";
    unavailable.setAttribute(
      "width",
      String(current.hasUnavailableHistory ? current.unavailableEndUnit * 1000 : 0)
    );
    unavailableAfter.setAttribute("x", String(current.unavailableAfterStartUnit * 1000));
    unavailableAfter.setAttribute(
      "width",
      String(
        current.hasUnavailableHistory
          ? Math.max(1 - current.unavailableAfterStartUnit, 0) * 1000
          : 0
      )
    );
    recorded.setAttribute("x", String(current.recordedStartUnit * 1000));
    recorded.setAttribute(
      "width",
      String(Math.max(current.recordedEndUnit - current.recordedStartUnit, 0) * 1000)
    );
    played.setAttribute("x", String(current.recordedStartUnit * 1000));
    played.setAttribute(
      "width",
      String(
        Math.max(
          Math.min(current.playheadUnit, current.recordedEndUnit) - current.recordedStartUnit,
          0
        ) * 1000
      )
    );
    future.setAttribute("x", String(current.playheadUnit * 1000));
    future.setAttribute("width", String(previewVisible ? futureWidth * 10 : 0));
    previewHandle.style.left = `${previewPct}%`;
    previewHandle.style.display = previewVisible ? "block" : "none";
    previewHandle.classList.toggle("clipped", current.previewClippedFrames > 0);
    previewHandle.classList.toggle(
      "fully-clipped",
      previewVisible && current.previewFrames > 0 && previewPct <= playheadPct
    );
    playhead.classList.toggle(
      "outside",
      current.playheadClippedBefore || current.playheadClippedAfter
    );

    overflow.style.display = previewVisible && current.previewClippedFrames > 0 ? "block" : "none";
    overflow.textContent = `+${current.previewClippedFrames}`;

    playhead.setAttribute("aria-valuemin", String(current.recorded.lo));
    playhead.setAttribute(
      "aria-valuemax",
      String(Math.max(current.recorded.hi, current.selectedFrame))
    );
    playhead.setAttribute("aria-valuenow", String(current.selectedFrame));
    const availability = describeRecordedAvailability(current);
    playhead.setAttribute(
      "aria-valuetext",
      `frame ${current.selectedFrame}` +
        (current.playheadClippedBefore || current.playheadClippedAfter
          ? `, outside the frozen viewport ${current.viewport.lo} to ${current.viewport.hi}`
          : "") +
        (availability ? `, ${availability}` : "")
    );

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
      (current.playheadClippedBefore || current.playheadClippedAfter
        ? ` <span class="out">outside</span>`
        : "") +
      (state.preview.enabled ? ` <span class="fut">+${current.previewFrames}</span>` : "") +
      ` / ${Math.round(current.viewport.hi)}`;
    pause.textContent = current.paused ? "▶" : "⏸";
    pause.setAttribute("aria-label", current.paused ? "Resume" : "Pause");
    camera.hidden = false;
    camera.disabled = pendingPointerLock || pendingDetachedGeneration !== null;
    camera.textContent = detachedActive ? "🔗" : "📷";
    camera.title = detachedActive
      ? "Exit the debug camera"
      : "Debug camera — FPS in 3D, pan/zoom in 2D";
    camera.setAttribute("aria-label", camera.title);
    camera.setAttribute("aria-pressed", String(detachedActive));
    camera.classList.toggle("on", detachedActive);
    el.classList.toggle("debug-active", detachedActive);
    if (detachedActive) {
      const debug = debugCamera();
      debugMode.value = String(debug.mode);
      const pan2d = debug.mode === 2;
      debugMode.hidden = pan2d;
      debugPan2d.hidden = !pan2d;
      debugFov.hidden = pan2d;
      debugLensName.textContent = pan2d ? "Zoom" : "FOV";
      if (pan2d) {
        debugLens.value = debug.zoom2d > 0 ? `${debug.zoom2d.toFixed(2)}×` : "—";
      } else {
        const fov = debug.fov > 0 ? debug.fov : 60;
        if (document.activeElement !== debugFov) debugFov.value = String(fov);
        debugLens.value = `${Math.round(fov)}°`;
      }
      debugMaterial.value = String(debug.material);
      debugMaterialLabel.hidden = pan2d;
      debugPhysics.checked = debug.physics;
      debugPhysicsLabel.hidden = pan2d;
      debugFrustum.checked = debug.authoredFrustum;
      debugFrustumLabel.hidden = pan2d;
      debugGameUi.checked = debug.gameUi;
    }
    extrapolate.classList.toggle("on", state.preview.enabled);
    extrapolate.setAttribute("aria-pressed", String(state.preview.enabled));
    renderTicks(current);
    renderMarkers(current);
    if (!hidden) {
      document.documentElement.style.setProperty("--functor-scrubber-h", `${el.offsetHeight}px`);
    }
  };

  const beginAbsoluteDrag = (handle, move) => {
    handle.addEventListener("pointerdown", (event) => {
      if (event.button !== 0) return;
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
    if (event.button !== 0) return;
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
    if (event.button !== 0) return;
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
      Home: current.recorded.lo,
      End: current.recorded.hi,
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
  const queueDetachedToggle = () => {
    flushPendingSeek();
    pendingDetachedGeneration = functor_lang_viewer_detached_generation();
    camera.disabled = true;
    functor_lang_viewer_toggle_detached();
  };
  const ownsDetachedInput = () =>
    pendingPointerLock ||
    pendingDetachedGeneration !== null ||
    functor_lang_viewer_detached();
  camera.addEventListener("click", () => {
    const wasDetached = functor_lang_viewer_detached();
    if (wasDetached) {
      queueDetachedToggle();
      if (document.pointerLockElement) document.exitPointerLock();
      return;
    }
    const canvas = document.getElementById("canvas");
    if (!canvas) return;
    pendingPointerLock = true;
    camera.disabled = true;
    const accepted = () => {
      pendingPointerLock = false;
      queueDetachedToggle();
    };
    const refused = () => {
      pendingPointerLock = false;
      if (!hidden) render();
    };
    try {
      const request = canvas.requestPointerLock();
      if (request && typeof request.then === "function") {
        request.then(accepted, refused);
      } else {
        accepted();
      }
    } catch {
      refused();
    }
  });
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
  debugMode.addEventListener("change", () =>
    functor_lang_viewer_set_mode(Number(debugMode.value))
  );
  debugFov.addEventListener("input", () => {
    debugLens.value = `${Math.round(debugFov.valueAsNumber)}°`;
    functor_lang_viewer_set_fov(debugFov.valueAsNumber);
  });
  debugMaterial.addEventListener("change", () =>
    functor_lang_viewer_set_material(Number(debugMaterial.value))
  );
  debugPhysics.addEventListener("change", () =>
    functor_lang_viewer_set_physics(debugPhysics.checked)
  );
  debugFrustum.addEventListener("change", () =>
    functor_lang_viewer_set_authored_frustum(debugFrustum.checked)
  );
  debugGameUi.addEventListener("change", () =>
    functor_lang_viewer_set_game_ui(debugGameUi.checked)
  );
  debugReset.addEventListener("click", () => functor_lang_viewer_reset());

  syncInputs();
  pushPreview();
  pushConfig();

  const seam = {
    paused: () => functor_lang_scrub_paused(),
    frame: () => functor_lang_scene_frame(),
    range: () => functor_lang_scene_range(),
    seek: requestSeek,
    togglePause: () => functor_lang_scrub_toggle_pause(),
    canDetach: () => true,
    detached: () => functor_lang_viewer_detached(),
    detachedGeneration: () => functor_lang_viewer_detached_generation(),
    ownsDetachedInput,
    setDetachedPointerLockPending: (pending) => {
      pendingPointerLock = pending;
    },
    toggleDetached: queueDetachedToggle,
    lookDetached: (dx, dy) => functor_lang_viewer_look(dx, dy),
    moveDetached: (forward, right, vertical, elapsedSeconds) =>
      functor_lang_viewer_move(forward, right, vertical, elapsedSeconds),
    zoomDetached: (steps) => functor_lang_viewer_zoom(steps),
    debugCamera,
    setDebugCamera: ({
      mode: debugModeValue,
      fov,
      material,
      physics,
      authoredFrustum,
      gameUi,
      reset,
    } = {}) => {
      if (debugModeValue !== undefined) functor_lang_viewer_set_mode(debugModeValue);
      if (fov !== undefined) functor_lang_viewer_set_fov(fov);
      if (material !== undefined) functor_lang_viewer_set_material(material);
      if (physics !== undefined) functor_lang_viewer_set_physics(physics);
      if (authoredFrustum !== undefined) {
        functor_lang_viewer_set_authored_frustum(authoredFrustum);
      }
      if (gameUi !== undefined) functor_lang_viewer_set_game_ui(gameUi);
      if (reset) functor_lang_viewer_reset();
    },
    step: () => functor_lang_scrub_step(),
    model: () => state,
    view,
    events: () => state.events,
    selectEvent: (id) => dispatch({ type: "event-selected", id }),
    // Accepts the timeline-model preview fields plus `mode` (1 trail /
    // 2 strobe / 3 both / 4 ghost) — the ghost-mode select isn't part of the
    // reducer's state, so hosts driving a hidden mount set it through here.
    setPreview: ({ mode: previewMode, ...preview }) => {
      // Only accept a mode the select actually offers — assigning an unknown
      // value would blank the select and push mode 0 (renderer off) while the
      // model still says enabled.
      if (previewMode !== undefined && mode.querySelector(`option[value="${previewMode}"]`)) {
        mode.value = String(previewMode);
      }
      dispatch({ type: "preview-changed", preview });
      syncInputs();
      pushPreview();
      pushConfig();
    },
  };
  window.__scrub = seam;

  const update = () => {
    const nextDetached = functor_lang_viewer_detached();
    const detachedGeneration = functor_lang_viewer_detached_generation();
    if (
      pendingDetachedGeneration !== null &&
      detachedGeneration !== pendingDetachedGeneration
    ) {
      pendingDetachedGeneration = null;
      if (!nextDetached && document.pointerLockElement) document.exitPointerLock();
      // A refused detach keeps `nextDetached` false, so there is no state
      // transition below to trigger a render. Re-enable the retry button now.
      if (!hidden) render();
    }
    if (nextDetached !== detachedActive) {
      detachedActive = nextDetached;
      if (!detachedActive && document.pointerLockElement) document.exitPointerLock();
      if (!hidden) render();
    }
    const debug = debugCamera();
    const debugSnapshotKey =
      `${debug.mode}:${debug.material}:${debug.physics}:${debug.authoredFrustum}:` +
      `${debug.gameUi}:${debug.fov.toFixed(3)}:${debug.zoom2d.toFixed(3)}`;
    if (debugSnapshotKey !== lastDebugSnapshotKey) {
      lastDebugSnapshotKey = debugSnapshotKey;
      if (!hidden && detachedActive) render();
    }
    if (pendingSeek !== null) {
      functor_lang_seek_scene(pendingSeek.frame, pendingSeek.id);
      pendingSeek = null;
    }
    const seekResult = functor_lang_scrub_seek_result();
    if (seekResult.length === 2 && seekResult[0] !== lastSeekResultId) {
      lastSeekResultId = seekResult[0];
      dispatch({ type: "seek-resolved", id: seekResult[0], frame: seekResult[1] });
    }
    const eventsGeneration = functor_lang_timeline_events_gen();
    if (eventsGeneration !== lastEventsGeneration) {
      lastEventsGeneration = eventsGeneration;
      const eventsJson = functor_lang_timeline_events();
      try {
        dispatch({ type: "events-published", events: JSON.parse(eventsJson) });
      } catch {
        // A malformed marker payload must not stop the runtime poll loop.
      }
    }
    const range = functor_lang_scene_range();
    if (range.length === 2) {
      if (!hidden) el.style.display = "flex";
      const snapshot = {
        frame: functor_lang_scene_frame(),
        lo: range[0],
        hi: range[1],
        paused: functor_lang_scrub_paused(),
        generation: functor_lang_scene_generation(),
      };
      const snapshotKey =
        `${snapshot.frame}:${snapshot.lo}:${snapshot.hi}:` +
        `${snapshot.paused}:${snapshot.generation}`;
      if (snapshotKey !== lastRuntimeSnapshotKey) {
        lastRuntimeSnapshotKey = snapshotKey;
        dispatch({ type: "runtime-published", snapshot });
      }
    } else {
      const paused = functor_lang_scrub_paused();
      if (paused && state.runtime) {
        if (!hidden) el.style.display = "flex";
        if (state.recordingAvailable) dispatch({ type: "recording-cleared" });
      } else if (!hidden) {
        el.style.display = "none";
      }
      lastRuntimeSnapshotKey = "";
    }
    raf = requestAnimationFrame(update);
  };
  raf = requestAnimationFrame(update);

  return {
    destroy() {
      cancelAnimationFrame(raf);
      el.remove();
      document.documentElement.style.removeProperty("--functor-scrubber-h");
      if (window.__scrub === seam) delete window.__scrub;
    },
  };
}
