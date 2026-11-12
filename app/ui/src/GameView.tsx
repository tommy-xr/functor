import {
  useEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent,
  type ReactNode,
  type RefObject,
} from "react";
import type { AppState, ClientMsg, GameInput, MouseButton } from "./generated/protocol";
import { fitRect, streamUrl, toSurface, wireKey, type Rect, type Size } from "./game-view";

type Send = (msg: ClientMsg) => void;

const BUTTONS: Record<number, MouseButton> = { 0: "left", 1: "middle", 2: "right" };
const RESIZE_DEBOUNCE_MS = 200;

/** The Game tab: the session's state in one line, or its live frames. */
export function GameView({ state, online, send }: { state: AppState | null; online: boolean; send: Send }) {
  const status = state?.session?.status ?? { kind: "stopped" };
  if (status.kind === "running") {
    if (!state!.no_gpu && state!.session_mode === "window") {
      return <GameMessage text="Running in its own window" />;
    }
    return (
      <LiveGame
        debugUrl={status.debug_url}
        surface={status.surface}
        noGpu={state!.no_gpu}
        online={online}
        send={send}
      />
    );
  }
  if (status.kind === "starting") return <GameMessage text="Starting…" />;
  const reason = status.kind === "failed" || status.kind === "exited" ? status.reason : null;
  return (
    <GameMessage text={reason ?? "No game running"} error={reason !== null}>
      <button
        type="button"
        className="button button-primary"
        disabled={!online || !state?.active}
        onClick={() => send({ type: "start_session" })}
      >
        Start
      </button>
    </GameMessage>
  );
}

function GameMessage({ text, error = false, children }: { text: string; error?: boolean; children?: ReactNode }) {
  return (
    <div className="game" data-testid="game-view">
      <div className="game-message">
        <p className={error ? "reason" : undefined}>{text}</p>
        {children}
      </div>
    </div>
  );
}

/**
 * A running hidden session: its `/stream` in an `<img>` while the page is
 * shown, and keyboard and mouse forwarded once clicked. Esc always releases
 * and is never forwarded, so a game that uses Esc needs its own window.
 */
function LiveGame({
  debugUrl,
  surface,
  noGpu,
  online,
  send,
}: {
  debugUrl: string;
  surface: [number, number] | null;
  noGpu: boolean;
  online: boolean;
  send: Send;
}) {
  const area = useRef<HTMLDivElement>(null);
  const img = useRef<HTMLImageElement>(null);
  const box = useBoxSize(area);
  const request = useDebounced(box, RESIZE_DEBOUNCE_MS);
  const visible = usePageVisible();
  const [frame, setFrame] = useState<Size | null>(null);
  const dpr = window.devicePixelRatio || 1;
  const url =
    !noGpu && visible && request
      ? streamUrl(debugUrl, { w: request.w * dpr, h: request.h * dpr })
      : null;

  // `src` is set here rather than rendered, so the old stream's request is
  // aborted (by removing `src`) before a new one opens, and on unmount.
  useEffect(() => {
    const el = img.current!;
    if (!url) return;
    el.src = url;
    return () => el.removeAttribute("src");
  }, [url]);

  // The frame's own pixels in CSS pixels cap its size, so it is never scaled up.
  const rect = box && frame ? fitRect(box, frame, { w: frame.w / dpr, h: frame.h / dpr }) : null;
  const capture = useCapture(send, area, rect ?? (box && { x: 0, y: 0, ...box }), surface);
  const { release } = capture;
  // Offline, input would be dropped, so the view lets go of it.
  useEffect(() => {
    if (!visible || !online) release();
  }, [visible, online, release]);

  return (
    <div
      ref={area}
      className={`game game-live${capture.captured ? " game-captured" : ""}`}
      data-testid="game-view"
      tabIndex={0}
      aria-label="Game"
      {...(online && capture.handlers)}
    >
      {noGpu && (
        <div className="game-message">
          <p>Frames need a GPU session</p>
        </div>
      )}
      <img
        ref={img}
        alt=""
        draggable={false}
        hidden={!url || !rect}
        style={rect ? { left: rect.x, top: rect.y, width: rect.w, height: rect.h } : undefined}
        onLoad={(e) => {
          const { naturalWidth: w, naturalHeight: h } = e.currentTarget;
          setFrame((f) => (f?.w === w && f.h === h ? f : { w, h }));
        }}
      />
      <span className="game-banner" role="status">
        {capture.captured ? "Esc to release" : "Click to play"}
      </span>
    </div>
  );
}

type Point = { x: number; y: number };

/**
 * Click-to-capture input. While captured, keys and mouse go to the session
 * (moves and wheel coalesced to one per animation frame); on release every
 * held key and button is let go on the wire, so nothing sticks.
 */
function useCapture(
  send: Send,
  area: RefObject<HTMLDivElement | null>,
  rect: Rect | null,
  surface: [number, number] | null,
) {
  const [captured, setCaptured] = useState(false);
  const live = useRef({ captured, rect, surface, send });
  live.current = { captured, rect, surface, send };
  const held = useRef({ keys: new Set<string>(), buttons: new Set<MouseButton>() });
  const pending = useRef<{ move: Point | null; wheel: number; raf: number }>({ move: null, wheel: 0, raf: 0 });

  // Stable across renders, reading the latest props through `live`.
  const [ops] = useState(() => {
    const input = (input: GameInput) => live.current.send({ type: "input", input });
    const flush = () => {
      const p = pending.current;
      cancelAnimationFrame(p.raf);
      p.raf = 0;
      if (p.move) input({ type: "mouse_move", ...p.move });
      if (p.wheel) input({ type: "mouse_wheel", delta: p.wheel });
      p.move = null;
      p.wheel = 0;
    };
    const schedule = () => {
      if (!pending.current.raf) pending.current.raf = requestAnimationFrame(flush);
    };
    const queueMove = (e: { clientX: number; clientY: number }) => {
      const { rect, surface } = live.current;
      const el = area.current;
      if (!rect || !surface || !el) return;
      // `rect` is in the padding box, inside the area's border.
      const at = el.getBoundingClientRect();
      const x = e.clientX - at.left - el.clientLeft;
      const y = e.clientY - at.top - el.clientTop;
      pending.current.move = toSurface(x, y, rect, {
        w: surface[0],
        h: surface[1],
      });
      schedule();
    };
    const release = () => {
      if (!live.current.captured) return;
      flush();
      for (const key of held.current.keys) input({ type: "key", key, down: false });
      for (const button of held.current.buttons) input({ type: "mouse_button", button, down: false });
      held.current.keys.clear();
      held.current.buttons.clear();
      live.current.captured = false;
      setCaptured(false);
    };
    return { input, flush, schedule, queueMove, release };
  });

  // Let go when the view goes away (tab switch, session stop).
  useEffect(() => ops.release, [ops]);

  useEffect(() => {
    const el = area.current;
    if (!el) return;
    // React's wheel listener is passive; this one may cancel the page scroll.
    const onWheel = (e: WheelEvent) => {
      if (!live.current.captured || !e.deltaY) return;
      e.preventDefault();
      pending.current.wheel -= Math.sign(e.deltaY);
      ops.schedule();
    };
    // On the window, so a button let go outside the view is still released.
    const onMouseUp = (e: MouseEvent) => {
      const button = BUTTONS[e.button];
      if (!button || !held.current.buttons.delete(button)) return;
      ops.flush();
      ops.input({ type: "mouse_button", button, down: false });
    };
    // Another app taking focus, or the page going away, keeps the view focused
    // but its key-ups never arrive.
    el.addEventListener("wheel", onWheel, { passive: false });
    window.addEventListener("mouseup", onMouseUp);
    window.addEventListener("blur", ops.release);
    window.addEventListener("pagehide", ops.release);
    return () => {
      el.removeEventListener("wheel", onWheel);
      window.removeEventListener("mouseup", onMouseUp);
      window.removeEventListener("blur", ops.release);
      window.removeEventListener("pagehide", ops.release);
    };
  }, [area, ops]);

  const handlers = {
    // Mouse events, not pointer events: those report only the first button
    // of a chord going down and the last coming up.
    onMouseDown: (e: ReactMouseEvent<HTMLDivElement>) => {
      // The click that captures is not forwarded.
      if (!captured) {
        live.current.captured = true;
        setCaptured(true);
        e.currentTarget.focus();
        return;
      }
      const button = BUTTONS[e.button];
      if (!button || held.current.buttons.has(button)) return;
      ops.queueMove(e);
      ops.flush();
      held.current.buttons.add(button);
      ops.input({ type: "mouse_button", button, down: true });
    },
    onPointerMove: (e: PointerEvent<HTMLDivElement>) => {
      if (captured) ops.queueMove(e);
    },
    onKeyDown: (e: KeyboardEvent<HTMLDivElement>) => {
      if (!captured) return;
      if (e.code === "Escape") {
        e.preventDefault();
        ops.release();
        return;
      }
      // Browser and system shortcuts keep working; the wire has no modifiers yet.
      const key = wireKey(e.code);
      if (!key || e.metaKey || e.ctrlKey) return;
      e.preventDefault();
      if (e.repeat || held.current.keys.has(key)) return;
      ops.flush();
      held.current.keys.add(key);
      ops.input({ type: "key", key, down: true });
    },
    onKeyUp: (e: KeyboardEvent<HTMLDivElement>) => {
      const key = wireKey(e.code);
      if (!key || !held.current.keys.delete(key)) return;
      e.preventDefault();
      ops.flush();
      ops.input({ type: "key", key, down: false });
    },
    onBlur: ops.release,
    onContextMenu: (e: ReactMouseEvent) => {
      if (captured) e.preventDefault();
    },
  };
  return { captured, release: ops.release, handlers };
}

/** `el`'s content size in CSS pixels, following resizes. */
function useBoxSize(el: RefObject<HTMLElement | null>): Size | null {
  const [size, setSize] = useState<Size | null>(null);
  useEffect(() => {
    const target = el.current;
    if (!target) return;
    const observer = new ResizeObserver(([entry]) => {
      const { width: w, height: h } = entry!.contentRect;
      setSize((s) => (s?.w === w && s.h === h ? s : { w, h }));
    });
    observer.observe(target);
    return () => observer.disconnect();
  }, [el]);
  return size;
}

/** `value`, first at once and then only after it has held still for `ms`. */
function useDebounced<T>(value: T, ms: number): T {
  const [settled, setSettled] = useState(value);
  useEffect(() => {
    if (settled === null) {
      setSettled(value);
      return;
    }
    const t = setTimeout(() => setSettled(value), ms);
    return () => clearTimeout(t);
    // Only a new value restarts the wait.
  }, [value, ms]);
  return settled;
}

function usePageVisible(): boolean {
  const [visible, setVisible] = useState(document.visibilityState === "visible");
  useEffect(() => {
    const update = () => setVisible(document.visibilityState === "visible");
    document.addEventListener("visibilitychange", update);
    return () => document.removeEventListener("visibilitychange", update);
  }, []);
  return visible;
}
