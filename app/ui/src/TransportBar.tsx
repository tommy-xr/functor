import { createContext, useContext, useEffect, useRef, useState, type CSSProperties, type PointerEvent } from "react";
import type { Mark, Transport, TransportCall } from "./generated/protocol";
import type { Send } from "./socket";
import {
  EXTRAPOLATE_BUTTON,
  PEEK_HOLD_MS,
  STEP_BUTTON,
  TIMELINE_COLORS,
  createTimelineState,
  deriveTimelineView,
  editPeeks,
  follow,
  followMarks,
  keyAction,
  markerDetail,
  markerLabel,
  pauseButton,
  reduceTimeline,
  timelineTicks,
  unitToFrame,
  type TimelineState,
  type TimelineView,
} from "./transport-model";

/** The active session's clock and marks (`ServerMsg::Transport`, `Marks`). */
export const TransportContext = createContext<{ transport: Transport | null; marks: Mark[] }>({
  transport: null,
  marks: [],
});

export const useTransport = () => useContext(TransportContext);

// The web scrubber's palette, as CSS variables for styles.css (`.transport`).
const COLORS = {
  "--tl-bg": TIMELINE_COLORS.bg,
  "--tl-line": TIMELINE_COLORS.line,
  "--tl-text": TIMELINE_COLORS.text,
  "--tl-dim": TIMELINE_COLORS.dim,
  "--tl-accent": TIMELINE_COLORS.accent,
  "--tl-future": TIMELINE_COLORS.future,
  "--tl-input": TIMELINE_COLORS.input,
  "--tl-reload": TIMELINE_COLORS.reload,
  "--tl-reload-error": TIMELINE_COLORS.reloadError,
} as CSSProperties;

/**
 * The game view's transport (D13): the web scrubber's pause/step, rail and
 * Extrapolate switch (D38), drawn from its own timeline model against the
 * native session. `compact` is the mini view's density: no frame count.
 * While it shows, the core polls the session often and reads its marks.
 * `extrapolates`: the session draws frames, so it can draw the path.
 */
export function TransportBar({
  transport,
  marks,
  online,
  send,
  compact,
  extrapolates,
}: {
  transport: Transport;
  marks: Mark[];
  online: boolean;
  send: Send;
  compact: boolean;
  extrapolates: boolean;
}) {
  const [timeline, dispatch] = useTimeline();
  const rail = useRef<HTMLDivElement>(null);
  // Seek ids echo back from the core to every page: start each page's apart.
  const nextSeek = useRef(Date.now() * 1000);
  const [hovered, setHovered] = useState<number | null>(null);

  useEffect(() => {
    if (!online) return;
    send({ type: "watch_transport", on: true });
    return () => void send({ type: "watch_transport", on: false });
  }, [online, send]);
  useEffect(() => void dispatch((s) => follow(s, transport)), [transport]);
  useEffect(() => void dispatch((s) => followMarks(s, marks)), [marks]);
  useEffect(() => void dispatch((s) => reduceTimeline(s, { type: "event-hovered", id: hovered })), [hovered]);

  const view = deriveTimelineView(timeline.current);
  const call = (call: TransportCall) => online && send({ type: "transport", call });
  const peek = usePeek(transport, extrapolates && online ? call : null);
  // Shown at once by the model; the core is asked for the frame it settled on.
  const seek = (frame: number) => {
    if (!online) return;
    const id = nextSeek.current++;
    const next = dispatch((s) => reduceTimeline(s, { type: "seek-requested", id, frame }));
    if (next.requestedSeekId === id) call({ type: "seek", frame: next.requestedFrame!, id });
  };
  const frameAt = (e: PointerEvent) => {
    const box = rail.current!.getBoundingClientRect();
    return unitToFrame(box.width > 0 ? (e.clientX - box.left) / box.width : 0, view!.viewport);
  };
  const drag = {
    onPointerDown: (e: PointerEvent<HTMLElement>) => {
      if (e.button !== 0 || !view || !online) return;
      e.preventDefault();
      e.currentTarget.setPointerCapture(e.pointerId);
      seek(frameAt(e));
    },
    onPointerMove: (e: PointerEvent<HTMLElement>) => {
      if (view && e.currentTarget.hasPointerCapture(e.pointerId)) seek(frameAt(e));
    },
  };
  const pause = pauseButton(transport.paused);
  const pct = (unit: number) => `${unit * 100}%`;
  const active = view?.activeEvent ?? null;
  return (
    <div className={`transport${compact ? " transport-compact" : ""}`} role="group" aria-label="Transport" style={COLORS}>
      <button
        type="button"
        className="transport-button"
        data-tip={pause.tip}
        aria-label={pause.label}
        disabled={!online}
        onClick={() => call({ type: transport.paused ? "resume" : "pause" })}
      >
        {pause.glyph}
      </button>
      <button
        type="button"
        className="transport-button"
        data-tip={STEP_BUTTON.tip}
        aria-label={STEP_BUTTON.label}
        disabled={!online}
        onClick={() => call({ type: "step" })}
      >
        {STEP_BUTTON.glyph}
      </button>
      <div ref={rail} className="transport-rail" title={view ? "Drag to seek" : undefined} {...drag}>
        <svg viewBox="0 0 1000 30" preserveAspectRatio="none" aria-hidden={view ? undefined : true}>
          <defs>
            <pattern id="transport-unavailable" width="12" height="12" patternUnits="userSpaceOnUse" patternTransform="rotate(20)">
              <rect width="12" height="12" fill="rgba(8, 7, 14, 0.72)" />
              <rect width="4" height="12" fill="rgba(155, 148, 179, 0.30)" />
            </pattern>
          </defs>
          <rect className="transport-track" x="0" y="12" width="1000" height="6" rx="3" />
          {view && (
            <>
              {view.hasUnavailableHistory && (
                <>
                  <rect className="transport-unavailable" x="0" y="12" width={view.unavailableEndUnit * 1000} height="6" rx="3" />
                  <rect
                    className="transport-unavailable"
                    x={view.unavailableAfterStartUnit * 1000}
                    y="12"
                    width={Math.max(1 - view.unavailableAfterStartUnit, 0) * 1000}
                    height="6"
                    rx="3"
                  />
                </>
              )}
              <rect
                className="transport-recorded"
                x={view.recordedStartUnit * 1000}
                y="12"
                width={Math.max(view.recordedEndUnit - view.recordedStartUnit, 0) * 1000}
                height="6"
                rx="3"
              />
              <rect
                className="transport-played"
                x={view.recordedStartUnit * 1000}
                y="12"
                width={Math.max(Math.min(view.playheadUnit, view.recordedEndUnit) - view.recordedStartUnit, 0) * 1000}
                height="6"
                rx="3"
              />
              {view.previewFrames > 0 && (
                <>
                  <rect
                    className="transport-past"
                    x={view.backwardStartUnit * 1000}
                    y="11"
                    width={Math.max(view.playheadUnit - view.backwardStartUnit, 0) * 1000}
                    height="8"
                    rx="3"
                  />
                  <rect
                    className="transport-future"
                    x={view.playheadUnit * 1000}
                    y="11"
                    width={Math.max(view.previewEndUnit - view.playheadUnit, 0) * 1000}
                    height="8"
                    rx="3"
                  />
                </>
              )}
              {timelineTicks(view.viewport).map((t) => (
                <rect
                  key={t.frame}
                  className={t.major ? "transport-tick major" : "transport-tick"}
                  x={t.x}
                  y={t.major ? 9 : 11}
                  width="1.5"
                  height={t.major ? 12 : 8}
                />
              ))}
              <g role="group" aria-label="Recorded events">
                {view.eventMarkers.map((m) => {
                  const reload = m.category === "reload";
                  const kind = m.kind === "reload-error" ? " reload-error" : "";
                  const on = m.id === active?.id;
                  return (
                    <g
                      key={m.id}
                      className={`transport-mark${on ? " active" : ""}`}
                      role="button"
                      tabIndex={0}
                      aria-label={markerLabel(m)}
                      transform={`translate(${m.unit * 1000} 0)`}
                      onMouseEnter={() => setHovered(m.id)}
                      onMouseLeave={() => setHovered(null)}
                      onFocus={() => setHovered(m.id)}
                      onBlur={() => setHovered(null)}
                      onPointerDown={(e) => e.stopPropagation()}
                      onClick={() => seek(m.frame)}
                      onKeyDown={(e) => {
                        if (e.key !== "Enter" && e.key !== " ") return;
                        e.preventDefault();
                        seek(m.frame);
                      }}
                    >
                      <rect x="-9" y="0" width="18" height="30" fill="transparent" />
                      <rect
                        className={`transport-mark-tick ${m.category}${kind}`}
                        x={reload ? -1.5 : -1}
                        y={reload ? 2 : 3}
                        width={reload ? 3 : 2}
                        height={reload ? 26 : 24}
                        rx="1"
                      />
                    </g>
                  );
                })}
              </g>
            </>
          )}
        </svg>
        {view && (
          <button
            type="button"
            className={`transport-playhead${view.playheadClippedBefore || view.playheadClippedAfter ? " outside" : ""}`}
            role="slider"
            aria-label="Selected frame"
            aria-orientation="horizontal"
            aria-valuemin={view.recorded.lo}
            aria-valuemax={Math.max(view.recorded.hi, view.selectedFrame)}
            aria-valuenow={view.selectedFrame}
            aria-valuetext={`frame ${view.selectedFrame}`}
            disabled={!online}
            style={{ left: pct(view.playheadUnit) }}
            onKeyDown={(e) => {
              const action = keyAction(view, e.key, e.shiftKey);
              if (!action) return;
              e.preventDefault();
              if ("step" in action) call({ type: "step" });
              else if ("past" in action) call({ type: "seek", frame: action.past, id: nextSeek.current++ });
              else seek(action.seek);
            }}
            {...drag}
          />
        )}
        {active && (
          <span className="transport-detail" role="status" style={{ left: pct(Math.min(0.95, Math.max(0.05, active.unit))) }}>
            {markerDetail(active)}
          </span>
        )}
        {!compact && (
          <span className="transport-frame" data-testid="transport-frame">
            {view ? frameLabel(view) : transport.frame}
          </span>
        )}
      </div>
      <button
        type="button"
        className={`transport-button transport-extrapolate${transport.extrapolate ? " on" : ""}${peek.peeking ? " peeking" : ""}`}
        data-tip={EXTRAPOLATE_BUTTON.tip}
        aria-label={EXTRAPOLATE_BUTTON.label}
        aria-pressed={transport.extrapolate}
        disabled={!online || !extrapolates}
        onClick={() => {
          // Pressed during a peek, it stays: the switch turns on.
          peek.cancel();
          call({ type: "extrapolate", on: !transport.extrapolate });
        }}
      >
        {EXTRAPOLATE_BUTTON.glyph}
      </button>
    </div>
  );
}

/**
 * The web scrubber's peek: an edit landing while paused with the switch off
 * shows the extrapolation by itself for `PEEK_HOLD_MS`, then the runtime goes
 * back to the switch. Another edit extends it; resuming or leaving ends it.
 * The switch and the rail stay as they are. `call`: `null` when the session
 * cannot draw the path.
 */
function usePeek(transport: Transport, call: ((call: TransportCall) => unknown) | null) {
  const [peeking, setPeeking] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const seen = useRef(transport.revision);
  const post = useRef(call);
  post.current = call;
  const end = (restore: boolean) => {
    if (timer.current === null) return;
    clearTimeout(timer.current);
    timer.current = null;
    setPeeking(false);
    if (restore) post.current?.({ type: "peek", on: false });
  };
  useEffect(() => {
    const since = seen.current;
    seen.current = transport.revision;
    if (post.current && editPeeks(since, transport)) {
      if (timer.current !== null) clearTimeout(timer.current);
      post.current({ type: "peek", on: true });
      timer.current = setTimeout(() => end(true), PEEK_HOLD_MS);
      setPeeking(true);
    } else if (!transport.paused) {
      end(true);
    }
  }, [transport]);
  useEffect(() => () => end(true), []);
  return { peeking, cancel: () => end(false) };
}

/** "120 / 300", as the web bar labels its rail; "outside" past a frozen window. */
function frameLabel(view: TimelineView) {
  const outside = view.playheadClippedBefore || view.playheadClippedAfter ? " outside" : "";
  return `${view.selectedFrame}${outside} / ${Math.round(view.viewport.hi)}`;
}

/**
 * The timeline model's state, stepped synchronously so a caller can read the
 * state its action produced (a seek posts the frame the model settled on).
 */
function useTimeline() {
  const state = useRef<TimelineState>(createTimelineState());
  const [, render] = useState(0);
  const [dispatch] = useState(() => (step: (s: TimelineState) => TimelineState) => {
    const next = step(state.current);
    if (next !== state.current) {
      state.current = next;
      render((n) => n + 1);
    }
    return next;
  });
  return [state, dispatch] as const;
}
