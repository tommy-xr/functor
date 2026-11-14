import {
  useEffect,
  useRef,
  useState,
  type FocusEvent,
  type KeyboardEvent,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent,
  type RefObject,
} from "react";
import type { AppState, Device, GameInput, Mirror, MouseButton, Pointer } from "./generated/protocol";
import { Held, keyDown, LockedCursor, wireKey } from "./game-input";
import {
  fitRect,
  headsetNote,
  MIRROR_CAMERAS,
  mirrorCamera,
  mirrorProblem,
  mirrorRate,
  streamUrl,
  toSurface,
  type HeadsetNote,
  type MirrorCamera,
  type MirrorStats,
  type Rect,
  type Size,
} from "./game-view";
import { placementOf } from "./layout-model";
import type { Send } from "./socket";
import { TransportBar, useTransport } from "./TransportBar";


const BUTTONS: Record<number, MouseButton> = { 0: "left", 1: "middle", 2: "right" };
const RESIZE_DEBOUNCE_MS = 200;

/**
 * The native window locks the pointer itself (WKWebView refuses the Pointer
 * Lock API): it holds the cursor still and sends the mouse's deltas to
 * `window.functorPointer`. A browser uses `requestPointerLock`.
 */
const NATIVE_POINTER = (window as { functorNativePointer?: boolean }).functorNativePointer === true;
const nativePointer = (lock: boolean) =>
  (window as { ipc?: { postMessage(m: string): void } }).ipc?.postMessage(JSON.stringify({ pointer: lock ? "lock" : "unlock" }));
const RELEASE_KEYS = /Mac|iPhone|iPad/.test(navigator.platform) ? "⌘Esc" : "Meta+Esc";

/** The Game tab: the session's state in one line, or its live frames. */
export function GameView({ state, online, send }: { state: AppState | null; online: boolean; send: Send }) {
  const status = state?.session?.status ?? { kind: "stopped" };
  const layout = state?.active ? state.layouts[state.active] : null;
  if (placementOf(layout) === "headset") {
    const device = state!.devices.list.find((d) => d.id === layout!.game.headset) ?? null;
    return <HeadsetGame state={state!} device={device} online={online} send={send} />;
  }
  // Shown only in a pane or Mini, so a running session is a hidden one.
  if (status.kind === "running") {
    return (
      <LiveGame
        debugUrl={status.debug_url}
        surface={status.surface}
        pointer={status.pointer}
        noGpu={state!.no_gpu}
        online={online}
        send={send}
        compact={placementOf(state?.active ? state.layouts[state.active] : null) === "mini"}
      />
    );
  }
  if (status.kind === "starting") return <GameMessage text="Starting…" />;
  // Start game sits beside the placement control (D22, D37).
  const reason = status.kind === "failed" || status.kind === "exited" ? status.reason : null;
  return <GameMessage text={reason ?? "The game is stopped"} error={reason !== null} />;
}

/**
 * The game on a headset (D5, D21): it owns the state, so this view says
 * where it runs and how its clock stands (read through the tunnel), with
 * the link's one fix while it is down, the transport bar's controls that
 * work there, and the way back to this computer.
 */
function HeadsetGame({ state, device, online, send }: { state: AppState; device: Device | null; online: boolean; send: Send }) {
  const { transport, marks } = useTransport();
  const visible = usePageVisible();
  const note = headsetNote(device, state.session, transport);
  const running = state.session?.status.kind === "running" && device?.state.state === "connected";
  const mirror = running ? (state.session?.mirror ?? null) : null;
  const bar = running && transport?.clock && visible && (
    <TransportBar transport={transport} marks={marks} online={online} send={send} compact={false} extrapolates={false} hideExtrapolate />
  );
  if (mirror) {
    return (
      <div className="game-stack">
        <MirrorGame mirror={mirror} note={note} visible={visible} online={online} send={send} />
        {bar}
      </div>
    );
  }
  return (
    <div className="game-stack">
      <div className="game game-headset" data-testid="game-view">
        <div className="game-message">
          <p className="headset-title">{note.title}</p>
          {note.detail && <p className={note.fix ? undefined : "headset-detail"}>{note.detail}</p>}
          {note.fix && <p className="reason">{note.fix}</p>}
          <button
            type="button"
            className="button button-compact"
            disabled={!online}
            onClick={() => send({ type: "layout", op: { type: "place", placement: "pane" } })}
          >
            Run it on this computer
          </button>
        </div>
        <span className="game-banner" role="status">
          On the headset
        </span>
      </div>
      {bar}
    </div>
  );
}

/** Between reads of the mirror's `/state.mirror`. */
const MIRROR_POLL_MS = 1000;

/**
 * The headset's game drawn here from its frames (D5): the mirror session's
 * `/stream`, with where it runs, the camera it is seen from (D18: the head,
 * or one parked near it), the rate it arrives at, and why it does not when
 * it does not. View-only: the game's input stays in the headset.
 */
function MirrorGame({
  mirror,
  note,
  visible,
  online,
  send,
}: {
  mirror: Mirror;
  note: HeadsetNote;
  visible: boolean;
  online: boolean;
  send: Send;
}) {
  const debugUrl = mirror.kind === "running" ? mirror.debug_url : null;
  const area = useRef<HTMLDivElement>(null);
  const img = useRef<HTMLImageElement>(null);
  const box = useBoxSize(area);
  const request = useDebounced(box, RESIZE_DEBOUNCE_MS);
  const [frame, setFrame] = useState<Size | null>(null);
  const [stats, setStats] = useState<MirrorStats | null>(null);
  const [camera, setCamera] = useState<MirrorCamera>("head");
  const dpr = window.devicePixelRatio || 1;
  const url = debugUrl && visible && request ? streamUrl(debugUrl, { w: request.w * dpr, h: request.h * dpr }) : null;

  useEffect(() => {
    const el = img.current!;
    if (!url) return;
    el.src = url;
    return () => el.removeAttribute("src");
  }, [url]);

  // The rate and the view to park cameras from, read while the view shows.
  useEffect(() => {
    if (!debugUrl || !visible) return;
    let stop = false;
    const read = async () => {
      try {
        const res = await fetch(new URL("/state?model=0", debugUrl));
        const next = ((await res.json()) as { mirror?: MirrorStats }).mirror ?? null;
        if (!stop) setStats(next);
      } catch {
        if (!stop) setStats({ frames: 0, fps: 0, latency_ms: null, view: null, error: "the mirror does not answer" });
      }
    };
    void read();
    const timer = setInterval(read, MIRROR_POLL_MS);
    return () => {
      stop = true;
      clearInterval(timer);
    };
  }, [debugUrl, visible]);

  // The mirror keeps its camera across pages; each starts on the head.
  useEffect(() => {
    if (!debugUrl) return;
    setCamera("head");
    void fetch(new URL("/camera", debugUrl), { method: "POST", body: JSON.stringify({ type: "attach" }) }).catch(() => {});
  }, [debugUrl]);

  const choose = (next: MirrorCamera) => {
    const body = debugUrl && mirrorCamera(next, stats?.view ?? null);
    if (!body) return;
    setCamera(next);
    void fetch(new URL("/camera", debugUrl), { method: "POST", body: JSON.stringify(body) }).catch(() => {});
  };

  const rect = box && frame ? fitRect(box, frame, { w: frame.w / dpr, h: frame.h / dpr }) : null;
  const problem = mirrorProblem(mirror, stats);
  const rate = mirrorRate(stats);
  return (
    <div className="mirror" data-testid="game-view">
      <div className="mirror-bar">
        <div className="mirror-where">
          <p className="headset-title">{note.title}</p>
          {note.detail && <p className="headset-detail" title={note.detail}>{note.detail}</p>}
        </div>
        <button
          type="button"
          className="button button-compact"
          disabled={!online}
          onClick={() => send({ type: "layout", op: { type: "place", placement: "pane" } })}
        >
          Run it on this computer
        </button>
        <div className="mirror-view">
          {rate && (
            <span className="mirror-rate" title="Frames from the headset per second, and one's round trip over the link">
              {rate}
            </span>
          )}
          <div className="segmented" role="group" aria-label="Camera">
            {MIRROR_CAMERAS.map((c) => (
              <button
                key={c.id}
                type="button"
                aria-pressed={camera === c.id}
                disabled={!debugUrl || (c.id !== "head" && !stats?.view)}
                onClick={() => choose(c.id)}
              >
                {c.label}
              </button>
            ))}
          </div>
        </div>
      </div>
      <div ref={area} className="game game-mirror" aria-label="Game, mirrored from the headset">
        <img
          ref={img}
          alt=""
          draggable={false}
          hidden={!url || !rect}
          className={problem ? "mirror-stale" : undefined}
          style={rect ? { left: rect.x, top: rect.y, width: rect.w, height: rect.h } : undefined}
          onLoad={(e) => {
            const { naturalWidth: w, naturalHeight: h } = e.currentTarget;
            setFrame((f) => (f?.w === w && f.h === h ? f : { w, h }));
          }}
        />
        {problem && (
          <div className="game-message">
            <p className="reason">{problem}</p>
          </div>
        )}
        <span className="game-banner" role="status">
          Mirrored from headset
        </span>
      </div>
    </div>
  );
}

function GameMessage({ text, error = false }: { text: string; error?: boolean }) {
  return (
    <div className="game" data-testid="game-view">
      <div className="game-message">
        <p className={error ? "reason" : undefined}>{text}</p>
      </div>
    </div>
  );
}

/**
 * A running hidden session: its `/stream` in an `<img>` while the page is
 * shown, and keyboard and mouse forwarded once clicked (the rules are in
 * `game-input.ts`). Paused, input is not forwarded, as the game's own window
 * ignores it then. The transport bar sits under it (`compact` in the mini
 * view).
 */
function LiveGame({
  debugUrl,
  surface,
  pointer,
  noGpu,
  online,
  send,
  compact,
}: {
  debugUrl: string;
  surface: [number, number] | null;
  pointer: Pointer;
  noGpu: boolean;
  online: boolean;
  send: Send;
  compact: boolean;
}) {
  const { transport, marks } = useTransport();
  const paused = !!transport?.clock && transport.paused;
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
  const capture = useCapture(send, area, rect ?? (box && { x: 0, y: 0, ...box }), surface, pointer, paused);
  const { release } = capture;
  // Offline or paused, input would be dropped or ignored, so the view lets go of it.
  useEffect(() => {
    if (!visible || !online || paused) release();
  }, [visible, online, paused, release]);

  return (
    <div className="game-stack">
    <div
      ref={area}
      className={`game game-live${capture.captured ? " game-captured" : ""}${capture.locked ? " game-locked" : ""}`}
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
        {paused ? "Paused" : capture.captured ? `${RELEASE_KEYS} to release` : "Click to play"}
      </span>
    </div>
    {transport?.clock && visible && (
      <TransportBar
        transport={transport}
        marks={marks}
        online={online}
        send={send}
        compact={compact}
        extrapolates={!noGpu}
      />
    )}
    </div>
  );
}

type Point = { x: number; y: number };

/**
 * Click-to-capture input. While captured, keys and mouse go to the session
 * (moves and wheel coalesced to one per animation frame), and a
 * captured-mouse game gets the pointer locked, its moves then relative.
 * Releasing (⌘Esc, the browser's Esc out of a lock, focus leaving the view
 * or the window, the view going away) lets go of every held key and button
 * on the wire, so nothing sticks.
 */
function useCapture(
  send: Send,
  area: RefObject<HTMLDivElement | null>,
  rect: Rect | null,
  surface: [number, number] | null,
  pointer: Pointer,
  paused: boolean,
) {
  const [captured, setCaptured] = useState(false);
  const [locked, setLocked] = useState(false);
  const live = useRef({ captured, rect, surface, pointer, send });
  live.current = { captured, rect, surface, pointer, send };
  const held = useRef(new Held());
  const pending = useRef<{ move: Point | null; wheel: number; raf: number }>({ move: null, wheel: 0, raf: 0 });
  // The cursor the lock moves, from the press that took it; null unlocked.
  const cursor = useRef<LockedCursor | null>(null);

  // Stable across renders, reading the latest props through `live`.
  const [ops] = useState(() => {
    const input = (input: GameInput) => live.current.send({ type: "input", input });
    // The game's last pointer position: the next lock moves on from it, so
    // a free-look game does not turn by the jump to the new click.
    let last: Point | null = null;
    const flush = () => {
      const p = pending.current;
      cancelAnimationFrame(p.raf);
      p.raf = 0;
      if (p.move) {
        last = p.move;
        input({ type: "mouse_move", ...p.move });
      }
      if (p.wheel) input({ type: "mouse_wheel", delta: p.wheel });
      p.move = null;
      p.wheel = 0;
    };
    const schedule = () => {
      if (!pending.current.raf) pending.current.raf = requestAnimationFrame(flush);
    };
    /** Where `e` lands in the game's surface, through the letterbox. */
    const at = (e: { clientX: number; clientY: number }): Point | null => {
      const { rect, surface } = live.current;
      const el = area.current;
      if (!rect || !surface || !el) return null;
      // `rect` is in the padding box, inside the area's border.
      const box = el.getBoundingClientRect();
      const x = e.clientX - box.left - el.clientLeft;
      const y = e.clientY - box.top - el.clientTop;
      return toSurface(x, y, rect, { w: surface[0], h: surface[1] });
    };
    // Unlocked, the pointer's position; locked, the pointer does not move.
    const queueMove = (e: { clientX: number; clientY: number }) => {
      const point = cursor.current ? null : at(e);
      if (!point) return;
      pending.current.move = point;
      schedule();
    };
    const queueDelta = (dx: number, dy: number) => {
      if (!cursor.current || (!dx && !dy)) return;
      pending.current.move = cursor.current.move(dx, dy);
      schedule();
    };
    // Edges go at once, after the moves queued before them.
    const edges = (inputs: (GameInput | null)[]) => {
      const sent = inputs.filter((i) => i !== null);
      if (sent.length) flush();
      sent.forEach(input);
    };
    const lock = (e: { clientX: number; clientY: number }) => {
      const { pointer, surface } = live.current;
      if (pointer !== "captured") return;
      cursor.current = new LockedCursor(last ?? at(e) ?? { x: (surface?.[0] ?? 0) / 2, y: (surface?.[1] ?? 0) / 2 });
      if (NATIVE_POINTER) {
        nativePointer(true);
        setLocked(true);
        return;
      }
      // Refused (a rejected promise here, `pointerlockerror` in other
      // browsers), moves stay absolute.
      try {
        (area.current?.requestPointerLock() as Promise<void> | undefined)?.catch?.(fallBack);
      } catch {
        fallBack();
      }
    };
    const fallBack = () => {
      cursor.current = null;
      setLocked(false);
    };
    const unlock = () => {
      if (!cursor.current) return;
      cursor.current = null;
      setLocked(false);
      if (NATIVE_POINTER) nativePointer(false);
      else if (document.pointerLockElement === area.current) document.exitPointerLock();
    };
    const release = () => {
      if (!live.current.captured) return;
      flush();
      edges(held.current.releaseAll());
      unlock();
      live.current.captured = false;
      setCaptured(false);
    };
    return { flush, schedule, queueMove, queueDelta, edges, lock, fallBack, release };
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
      if (button) ops.edges([held.current.button(button, false)]);
    };
    // The browser's lock: taken (moves become relative; dropped if the
    // capture ended while it was asked for), or ended by the browser (its
    // Esc, the view moving to another slot): the capture goes on, absolute,
    // while the view keeps focus.
    const onLockChange = () => {
      if (document.pointerLockElement === el) {
        if (cursor.current) setLocked(true);
        else document.exitPointerLock();
      } else if (cursor.current) {
        ops.fallBack();
        setTimeout(() => document.activeElement !== el && ops.release());
      }
    };
    // The native window's lock: deltas, or the window letting go on its own.
    const onNative = (msg: { dx?: number; dy?: number; lost?: boolean }) => {
      if (msg.lost) ops.release();
      else ops.queueDelta(msg.dx ?? 0, msg.dy ?? 0);
    };
    const win = window as { functorPointer?: typeof onNative };
    win.functorPointer = onNative;
    // Another app taking focus, or the page going away, keeps the view focused
    // but its key-ups never arrive.
    el.addEventListener("wheel", onWheel, { passive: false });
    window.addEventListener("mouseup", onMouseUp);
    window.addEventListener("blur", ops.release);
    window.addEventListener("pagehide", ops.release);
    document.addEventListener("pointerlockchange", onLockChange);
    document.addEventListener("pointerlockerror", ops.fallBack);
    return () => {
      if (win.functorPointer === onNative) delete win.functorPointer;
      el.removeEventListener("wheel", onWheel);
      window.removeEventListener("mouseup", onMouseUp);
      window.removeEventListener("blur", ops.release);
      window.removeEventListener("pagehide", ops.release);
      document.removeEventListener("pointerlockchange", onLockChange);
      document.removeEventListener("pointerlockerror", ops.fallBack);
    };
  }, [area, ops]);

  const handlers = {
    // Mouse events, not pointer events: those report only the first button
    // of a chord going down and the last coming up.
    onMouseDown: (e: ReactMouseEvent<HTMLDivElement>) => {
      // Paused, there is nothing to capture for.
      if (paused) return;
      // The click that captures is not forwarded.
      if (!captured) {
        live.current.captured = true;
        setCaptured(true);
        e.currentTarget.focus();
        ops.lock(e);
        return;
      }
      const button = BUTTONS[e.button];
      const press = button ? held.current.button(button, true) : null;
      if (!press) return;
      ops.queueMove(e);
      ops.edges([press]);
    },
    onPointerMove: (e: PointerEvent<HTMLDivElement>) => {
      if (!captured) return;
      if (document.pointerLockElement === e.currentTarget) ops.queueDelta(e.movementX, e.movementY);
      else ops.queueMove(e);
    },
    onKeyDown: (e: KeyboardEvent<HTMLDivElement>) => {
      if (!captured) return;
      const { inputs, release, handled } = keyDown(held.current, e);
      if (handled) {
        // Kept from the app's shortcuts and the browser (Tab, F5, Space).
        e.preventDefault();
        e.stopPropagation();
      }
      if (release) ops.release();
      else ops.edges(inputs);
    },
    onKeyUp: (e: KeyboardEvent<HTMLDivElement>) => {
      const key = wireKey(e.code);
      const up = key ? held.current.key(key, false) : null;
      if (!up) return;
      e.preventDefault();
      ops.edges([up]);
    },
    // Focus moving elsewhere in the page ends the capture; the view moving
    // to another slot (an agent splitting beside it) gets it back first
    // (`GameHost`), so it does not.
    onBlur: (e: FocusEvent<HTMLDivElement>) => {
      const el = e.currentTarget;
      setTimeout(() => {
        if (document.activeElement !== el) ops.release();
      });
    },
    onContextMenu: (e: ReactMouseEvent) => {
      if (captured) e.preventDefault();
    },
  };
  return { captured, locked, release: ops.release, handlers };
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
