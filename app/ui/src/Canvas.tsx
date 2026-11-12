import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type PointerEvent,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";
import type { AppState, Direction, LayoutOp, Node, Pane, Session, SessionMode, Task, View } from "./generated/protocol";
import { DiffView, type DiffTarget } from "./DiffView";
import { hasChange } from "./diff-model";
import { FileView } from "./FileView";
import { GameView } from "./GameView";
import { Icon } from "./Icon";
import { useMenu } from "./menu";
import {
  focusedPane,
  isTyping,
  keysFor,
  panesOf,
  shortcutFor,
  shortcutHelp,
  tabTitles,
  viewKey,
  type Shortcut,
} from "./layout-model";
import { fileKey, type DiffMsg, type FileMsg, type Notice, type Send } from "./socket";

/** The drag payload of a tab: its view as JSON. */
const DRAG_TYPE = "application/x-functor-view";
const PLATFORM = navigator.platform;
/** A splitter moves this far (thousandths) per arrow key. */
const RESIZE_STEP = 50;
const RATIO_MIN = 100;
const RATIO_MAX = 900;

export type CanvasProps = {
  state: AppState | null;
  online: boolean;
  send: Send;
  diffs: ReadonlyMap<number, DiffMsg>;
  fetchDiff: (task: number) => void;
  notice: Notice | null;
  /** Where "View diff" last asked a task's changes to scroll. */
  target: (DiffTarget & { task: number }) | null;
  files: ReadonlyMap<string, FileMsg>;
  fetchFile: (path: string) => void;
  /** The chat's current task, whose changes the open-view menu offers. */
  currentTask: Task | null;
  onNewTask: () => void;
};

/** What every pane needs, passed down the split tree. */
type Ctx = CanvasProps & {
  op: (op: LayoutOp) => boolean;
  titles: Map<string, string>;
  focus: number | null;
  /** Moves DOM focus into the focused pane once the next layout arrives. */
  refocus: () => void;
};

export function isLive(session: Session | null | undefined): boolean {
  const kind = session?.status.kind;
  return kind === "starting" || kind === "running";
}

/**
 * The canvas: the active project's layout (core state) as panes of tabs.
 * Every gesture is a `LayoutOp` to the core; the page re-renders from the
 * state it sends back, so a reload or a second window shows the same.
 */
export function Canvas(props: CanvasProps) {
  const { state, send } = props;
  const layout = state?.active ? (state.layouts[state.active] ?? null) : null;
  const op = useCallback((op: LayoutOp) => send({ type: "layout", op }), [send]);
  const views = panesOf(layout).flatMap((p) => p.tabs);
  const titles = tabTitles(views, state?.tasks ?? []);

  // After a keyboard action, focus follows the layout the core sends back.
  const main = useRef<HTMLElement>(null);
  const wantFocus = useRef(false);
  const refocus = useCallback(() => {
    wantFocus.current = true;
  }, []);
  useLayoutEffect(() => {
    if (!wantFocus.current || !main.current) return;
    wantFocus.current = false;
    const pane = main.current.querySelector(`[data-pane="${layout?.focus}"]`) ?? main.current;
    pane.querySelector<HTMLElement>("[role=tab][aria-selected=true], .pane-actions button, .empty-canvas button")?.focus();
  }, [layout]);

  useShortcuts(layout, op, refocus);
  const ctx: Ctx = { ...props, op, titles, focus: layout?.focus ?? null, refocus };
  return (
    <main className="canvas" ref={main} aria-label="Canvas">
      <GameHost state={state} online={props.online} send={send}>
        {layout?.root ? <NodeView node={layout.root} ctx={ctx} /> : <EmptyCanvas ctx={ctx} />}
      </GameHost>
    </main>
  );
}

/** Global layout shortcuts (see `shortcutFor`), acting on the focused pane. */
function useShortcuts(layout: AppState["layouts"][string] | null, op: (op: LayoutOp) => void, refocus: () => void) {
  const live = useRef(layout);
  live.current = layout;
  useEffect(() => {
    const onKeyDown = (e: globalThis.KeyboardEvent) => {
      const target = e.target instanceof HTMLElement ? e.target : null;
      const action = shortcutFor(e, {
        typing: isTyping(target),
        captured: target?.closest(".game-captured") != null,
      });
      if (!action) return;
      e.preventDefault();
      run(action);
    };
    const run = (action: Shortcut) => {
      const layout = live.current;
      const panes = panesOf(layout);
      const pane = focusedPane(layout);
      if (!pane) return;
      const step = action === "next_tab" ? 1 : action === "previous_tab" ? -1 : 0;
      if (step && pane.tabs.length > 1) {
        const view = pane.tabs[(pane.active + step + pane.tabs.length) % pane.tabs.length]!;
        op({ type: "open", view, target: { kind: "pane", pane: pane.id } });
      } else if (action === "close_tab") {
        const view = pane.tabs[pane.active];
        op(view ? { type: "close", view } : { type: "close_pane", pane: pane.id });
      } else if (action === "split_right") {
        op({ type: "split", pane: pane.id, direction: "right" });
      } else if (action === "next_pane" && panes.length > 1) {
        const next = panes[(panes.indexOf(pane) + 1) % panes.length]!;
        op({ type: "focus", pane: next.id });
      } else {
        return;
      }
      refocus();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [op, refocus]);
}

/**
 * The one game view, kept mounted in a detached element that moves into
 * whichever pane shows the Game tab: moving the tab moves the view, so its
 * stream and input capture carry over. Unmounted while no pane shows it.
 */
const GameSlotContext = createContext<(el: HTMLElement, on: boolean) => void>(() => {});

function GameHost({ state, online, send, children }: { state: AppState | null; online: boolean; send: Send; children: ReactNode }) {
  const [host] = useState(() => {
    const el = document.createElement("div");
    el.className = "game-host";
    return el;
  });
  const [slot, setSlot] = useState<HTMLElement | null>(null);
  const claim = useCallback((el: HTMLElement, on: boolean) => setSlot((s) => (on ? el : s === el ? null : s)), []);
  useLayoutEffect(() => {
    if (slot) slot.appendChild(host);
    else host.remove();
  }, [slot, host]);
  return (
    <GameSlotContext.Provider value={claim}>
      {children}
      {slot && createPortal(<GameView state={state} online={online} send={send} />, host)}
    </GameSlotContext.Provider>
  );
}

/** Where the game view shows in the pane whose Game tab is shown. */
function GameSlot() {
  const ref = useRef<HTMLDivElement>(null);
  const claim = useContext(GameSlotContext);
  useLayoutEffect(() => {
    const el = ref.current!;
    claim(el, true);
    return () => claim(el, false);
  }, [claim]);
  return <div className="game-slot" ref={ref} />;
}

function NodeView({ node, ctx }: { node: Node; ctx: Ctx }) {
  if (node.kind === "pane") return <PaneView pane={node} ctx={ctx} />;
  return <SplitView split={node} ctx={ctx} />;
}

type SplitNode = Extract<Node, { kind: "split" }>;

/** Two children and the splitter between them; a drag is local until let go. */
function SplitView({ split, ctx }: { split: SplitNode; ctx: Ctx }) {
  const box = useRef<HTMLDivElement>(null);
  const [dragged, setDragged] = useState<number | null>(null);
  // The core's ratio, once it has the dragged one, replaces it.
  useEffect(() => setDragged(null), [split.ratio]);
  const ratio = dragged ?? split.ratio;
  const right = split.direction === "right";
  /**
   * Sends `r` (clamped) unless it is `from`; the ratio to show until the
   * core's arrives: the clamped one, or null (the core's) when unsent.
   */
  const resize = (r: number, from: number) => {
    const clamped = Math.round(Math.min(Math.max(r, RATIO_MIN), RATIO_MAX));
    if (clamped === from) return from === split.ratio ? null : from;
    return ctx.op({ type: "resize", split: split.id, ratio: clamped }) ? clamped : null;
  };
  const at = (e: PointerEvent) => {
    const r = box.current!.getBoundingClientRect();
    return right ? ((e.clientX - r.left) / r.width) * 1000 : ((e.clientY - r.top) / r.height) * 1000;
  };
  const onKeyDown = (e: KeyboardEvent) => {
    const less = right ? "ArrowLeft" : "ArrowUp";
    const more = right ? "ArrowRight" : "ArrowDown";
    const to =
      e.key === less ? ratio - RESIZE_STEP
      : e.key === more ? ratio + RESIZE_STEP
      : e.key === "Home" ? RATIO_MIN
      : e.key === "End" ? RATIO_MAX
      : null;
    if (to === null) return;
    e.preventDefault();
    // Shown at once, so keys pressed faster than the core answers add up.
    setDragged(resize(to, ratio));
  };
  return (
    <div ref={box} className={`split split-${split.direction}`}>
      <div className="split-child" style={{ flexGrow: ratio }}>
        <NodeView node={split.first} ctx={ctx} />
      </div>
      <div
        className="splitter"
        role="separator"
        tabIndex={0}
        aria-label="Resize panes"
        aria-orientation={right ? "vertical" : "horizontal"}
        aria-valuenow={Math.round(ratio / 10)}
        aria-valuemin={RATIO_MIN / 10}
        aria-valuemax={RATIO_MAX / 10}
        onKeyDown={onKeyDown}
        onPointerDown={(e) => {
          e.preventDefault();
          e.currentTarget.setPointerCapture(e.pointerId);
          setDragged(Math.min(Math.max(at(e), RATIO_MIN), RATIO_MAX));
        }}
        onPointerMove={(e) => {
          if (e.currentTarget.hasPointerCapture(e.pointerId)) setDragged(Math.min(Math.max(at(e), RATIO_MIN), RATIO_MAX));
        }}
        onPointerUp={(e) => {
          if (e.currentTarget.hasPointerCapture(e.pointerId)) setDragged(resize(at(e), split.ratio));
        }}
        // A drag cancelled before it was let go sent nothing: back to the core's ratio.
        onPointerCancel={() => setDragged(null)}
      />
      <div className="split-child" style={{ flexGrow: 1000 - ratio }}>
        <NodeView node={split.second} ctx={ctx} />
      </div>
    </div>
  );
}

function tabId(pane: number, view: View) {
  return `tab-${pane}-${viewKey(view).replace(/[^\w-]/g, "_")}`;
}

/** The view dragged in `e`, if it is a tab. Another page's drag may carry anything; the core moves only open views. */
function dragged(e: globalThis.DragEvent): View | null {
  try {
    return JSON.parse(e.dataTransfer?.getData(DRAG_TYPE) ?? "") as View;
  } catch {
    return null;
  }
}

function PaneView({ pane, ctx }: { pane: Pane; ctx: Ctx }) {
  const { op, titles } = ctx;
  const focused = ctx.focus === pane.id;
  const active = pane.tabs[pane.active] ?? null;
  const [over, setOver] = useState(false);
  // The shown tab stays in view in a bar that scrolls.
  useEffect(() => {
    if (active) document.getElementById(tabId(pane.id, active))?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }, [pane.id, active && viewKey(active)]);
  // Native listeners, not React's: the game view is portalled in from
  // `GameHost`, so its React events never pass through this pane.
  const box = useRef<HTMLElement>(null);
  const live = useRef({ focused, pane, op });
  live.current = { focused, pane, op };
  useEffect(() => {
    const el = box.current!;
    const focus = () => {
      const { focused, pane, op } = live.current;
      if (!focused) op({ type: "focus", pane: pane.id });
    };
    const dragOver = (e: globalThis.DragEvent) => {
      if (!e.dataTransfer?.types.includes(DRAG_TYPE)) return;
      e.preventDefault();
      e.dataTransfer.dropEffect = "move";
      setOver(true);
    };
    const dragLeave = (e: globalThis.DragEvent) => {
      if (!el.contains(e.relatedTarget as globalThis.Node | null)) setOver(false);
    };
    // Drops anywhere on the pane; on a tab, before or after it by its middle.
    const drop = (e: globalThis.DragEvent) => {
      setOver(false);
      const view = dragged(e);
      if (!view) return;
      e.preventDefault();
      const { pane, op } = live.current;
      const tab = (e.target as HTMLElement).closest<HTMLElement>("[data-tab-index]");
      let index = pane.tabs.length;
      if (tab) {
        const r = tab.getBoundingClientRect();
        index = Number(tab.dataset.tabIndex) + (e.clientX > r.left + r.width / 2 ? 1 : 0);
      }
      op({ type: "move", view, pane: pane.id, index });
    };
    const listeners: [string, EventListener, boolean][] = [
      ["pointerdown", focus, true],
      ["focusin", focus, true],
      ["dragover", dragOver as EventListener, false],
      ["dragleave", dragLeave as EventListener, false],
      ["drop", drop as EventListener, false],
    ];
    for (const [type, f, capture] of listeners) el.addEventListener(type, f, capture);
    return () => {
      for (const [type, f, capture] of listeners) el.removeEventListener(type, f, capture);
    };
  }, []);
  const show = (view: View) => op({ type: "open", view, target: { kind: "pane", pane: pane.id } });
  // Arrow keys move between tabs (and show them), as in the ARIA tabs pattern.
  const onTabKey = (e: KeyboardEvent, i: number) => {
    const n = pane.tabs.length;
    const to =
      e.key === "ArrowRight" ? (i + 1) % n
      : e.key === "ArrowLeft" ? (i - 1 + n) % n
      : e.key === "Home" ? 0
      : e.key === "End" ? n - 1
      : null;
    if (e.key === "Delete") {
      e.preventDefault();
      op({ type: "close", view: pane.tabs[i]! });
      ctx.refocus();
      return;
    }
    if (to === null) return;
    e.preventDefault();
    show(pane.tabs[to]!);
    document.getElementById(tabId(pane.id, pane.tabs[to]!))?.focus();
  };
  return (
    <section
      className={`pane-box${focused ? " pane-focused" : ""}${over ? " pane-drop" : ""}`}
      data-pane={pane.id}
      aria-label={active ? (titles.get(viewKey(active)) ?? "Pane") : "Empty pane"}
      ref={box}
    >
      <div className="pane-bar">
        <div
          className="pane-tabs"
          role="tablist"
          aria-label="Tabs"
          // A plain wheel scrolls the tabs sideways.
          onWheel={(e) => {
            if (e.deltaY && !e.deltaX) e.currentTarget.scrollLeft += e.deltaY;
          }}
        >
          {pane.tabs.map((view, i) => {
            const title = titles.get(viewKey(view)) ?? "";
            const selected = i === pane.active;
            return (
              <div
                key={viewKey(view)}
                className="tab"
                role="presentation"
                data-tab-index={i}
                draggable
                onDragStart={(e) => {
                  e.dataTransfer.setData(DRAG_TYPE, JSON.stringify(view));
                  e.dataTransfer.effectAllowed = "move";
                }}
                onMouseDown={(e) => e.button === 1 && e.preventDefault()}
                onAuxClick={(e) => e.button === 1 && op({ type: "close", view })}
              >
                <button
                  type="button"
                  role="tab"
                  id={tabId(pane.id, view)}
                  className={`tab-title${view.kind === "file" ? " mono" : ""}`}
                  aria-selected={selected}
                  aria-controls={`panel-${pane.id}`}
                  tabIndex={selected ? 0 : -1}
                  title={title}
                  onClick={() => show(view)}
                  onKeyDown={(e) => onTabKey(e, i)}
                >
                  {title}
                </button>
                <button
                  type="button"
                  className="tab-close"
                  tabIndex={-1}
                  aria-label={`Close ${title}`}
                  title={`Close (${keysFor("close_tab", PLATFORM)}, or middle-click the tab)`}
                  onClick={() => op({ type: "close", view })}
                >
                  ×
                </button>
              </div>
            );
          })}
        </div>
        {active?.kind === "game" && <GameControls ctx={ctx} />}
        <PaneActions pane={pane} ctx={ctx} />
      </div>
      <div
        className="pane-body"
        role="tabpanel"
        id={`panel-${pane.id}`}
        {...(active ? { "aria-labelledby": tabId(pane.id, active) } : { "aria-label": "Empty pane" })}
      >
        {active ? <ViewBody view={active} ctx={ctx} /> : <EmptyPane pane={pane} ctx={ctx} />}
      </div>
    </section>
  );
}

function ViewBody({ view, ctx }: { view: View; ctx: Ctx }) {
  const { state } = ctx;
  switch (view.kind) {
    case "game":
      return <GameSlot />;
    case "file":
      return (
        <FileView
          path={view.path}
          file={state?.active ? ctx.files.get(fileKey(state.active, view.path)) : undefined}
          fetchFile={ctx.fetchFile}
          online={ctx.online}
        />
      );
    case "changes": {
      const task = state?.tasks.find((t) => t.id === view.task) ?? null;
      return (
        <DiffView
          state={state}
          online={ctx.online}
          send={ctx.send}
          task={task}
          diff={ctx.diffs.get(view.task)}
          fetchDiff={ctx.fetchDiff}
          notice={ctx.notice}
          target={ctx.target?.task === view.task ? ctx.target : null}
        />
      );
    }
    case "task":
      return (
        <div className="placeholder">
          <p>The task page comes later; its conversation is in the chat.</p>
        </div>
      );
  }
}

/** Start/Stop beside where the game runs (D22), while the Game tab shows. */
function GameControls({ ctx }: { ctx: Ctx }) {
  const { state, online, send } = ctx;
  return (
    <div className="pane-game">
      {isLive(state?.session) && (
        <button type="button" className="button button-compact" disabled={!online} onClick={() => send({ type: "stop_session" })}>
          Stop game
        </button>
      )}
      <ModePicker state={state} online={online} send={send} />
    </div>
  );
}

const MODES: { mode: SessionMode; label: string }[] = [
  { mode: "hidden", label: "In this window" },
  { mode: "window", label: "Its own window" },
];

/** Where the game runs; a change restarts a live session. */
function ModePicker({ state, online, send }: { state: AppState | null; online: boolean; send: Send }) {
  return (
    <div className="switcher switcher-compact">
      <select
        aria-label="Where the game runs"
        value={state?.session_mode ?? "hidden"}
        disabled={!online}
        onChange={(e) => send({ type: "set_session_mode", mode: e.target.value as SessionMode })}
      >
        {MODES.map(({ mode, label }) => (
          <option key={mode} value={mode}>
            {label}
          </option>
        ))}
      </select>
      <Icon name="chevron" />
    </div>
  );
}

/** Views to offer in a pane: the game if it is not open, the files, the current task's changes. */
function offers(ctx: Ctx): { view: View; label: string }[] {
  const { state, currentTask } = ctx;
  const open = new Set(ctx.titles.keys());
  const out: { view: View; label: string }[] = [];
  if (!open.has("game")) out.push({ view: { kind: "game" }, label: "Game" });
  for (const path of state?.files ?? []) out.push({ view: { kind: "file", path }, label: path });
  if (currentTask && hasChange(currentTask)) {
    out.push({ view: { kind: "changes", task: currentTask.id }, label: `Changes · ${currentTask.title || "Untitled task"}` });
  }
  return out;
}

/** The four actions ending a pane's tab bar (D20). */
function PaneActions({ pane, ctx }: { pane: Pane; ctx: Ctx }) {
  const { op, online } = ctx;
  const split = (direction: Direction) => op({ type: "split", pane: pane.id, direction });
  return (
    <div className="pane-actions">
      <ViewMenu ctx={ctx} pane={pane} />
      <button
        type="button"
        className="icon-button"
        aria-label="Split right"
        title={`Split right (${keysFor("split_right", PLATFORM)})`}
        disabled={!online}
        onClick={() => split("right")}
      >
        <Icon name="split-right" />
      </button>
      <button type="button" className="icon-button" aria-label="Split down" title="Split down" disabled={!online} onClick={() => split("down")}>
        <Icon name="split-down" />
      </button>
      <button
        type="button"
        className="icon-button"
        aria-label="Pop out into its own window (coming later)"
        title="Popping a pane out into its own window comes later"
        aria-disabled="true"
      >
        <Icon name="pop-out" />
      </button>
    </div>
  );
}

/** "Open a view here": a menu of `offers`, then "Reset layout". */
function ViewMenu({ ctx, pane }: { ctx: Ctx; pane: Pane }) {
  const { open, setOpen, ref: menu, onKeyDown } = useMenu();
  const pick = (op: LayoutOp) => {
    setOpen(false);
    ctx.op(op);
    ctx.refocus();
  };
  return (
    <div className="pane-menu" ref={menu} onKeyDown={onKeyDown}>
      <button
        type="button"
        className="icon-button"
        aria-label="Open a view here"
        title={`Open a view here. Shortcuts: ${shortcutHelp(PLATFORM)}`}
        aria-haspopup="menu"
        aria-expanded={open}
        disabled={!ctx.online}
        onClick={() => setOpen((o) => !o)}
      >
        <Icon name="plus" />
      </button>
      {open && (
        <div className="menu" role="menu" aria-label="Open a view here">
          {offers(ctx).map(({ view, label }) => (
            <button
              key={viewKey(view)}
              type="button"
              role="menuitem"
              className={view.kind === "file" ? "mono" : undefined}
              onClick={() => pick({ type: "open", view, target: { kind: "pane", pane: pane.id } })}
            >
              <span className="menu-title">{label}</span>
            </button>
          ))}
          <hr className="menu-rule" />
          <button type="button" role="menuitem" onClick={() => pick({ type: "reset" })}>
            <span className="menu-title">Reset layout</span>
          </button>
          <p className="menu-help">{shortcutHelp(PLATFORM)}</p>
        </div>
      )}
    </div>
  );
}

/** A pane a split just made: what it can show, or close it. */
function EmptyPane({ pane, ctx }: { pane: Pane; ctx: Ctx }) {
  const { op, online } = ctx;
  return (
    <div className="placeholder empty-pane">
      <p>Open a view here</p>
      <div className="empty-actions">
        {offers(ctx).map(({ view, label }) => (
          <button
            key={viewKey(view)}
            type="button"
            className={`button button-compact${view.kind === "file" ? " mono" : ""}`}
            disabled={!online}
            onClick={() => op({ type: "open", view, target: { kind: "pane", pane: pane.id } })}
          >
            {label}
          </button>
        ))}
      </div>
      <button type="button" className="link-button" disabled={!online} onClick={() => op({ type: "close_pane", pane: pane.id })}>
        Close pane
      </button>
    </div>
  );
}

/** No panes: start the game, open a file, or start a task (D20). */
function EmptyCanvas({ ctx }: { ctx: Ctx }) {
  const { state, online, send, op } = ctx;
  const [picking, setPicking] = useState(false);
  const focused = { kind: "focused" } as const;
  return (
    <div className="placeholder empty-canvas">
      <p>Nothing open</p>
      <div className="empty-actions">
        <button
          type="button"
          className="button button-primary"
          disabled={!online || !state?.active}
          onClick={() => {
            op({ type: "open", view: { kind: "game" }, target: focused });
            if (!isLive(state?.session)) send({ type: "start_session" });
          }}
        >
          Start game
        </button>
        <button type="button" className="button" disabled={!online || !state?.files.length} aria-expanded={picking} onClick={() => setPicking((p) => !p)}>
          Open a file
        </button>
        <button type="button" className="button" disabled={!state} onClick={ctx.onNewTask}>
          New task
        </button>
      </div>
      {picking && (
        <ul className="files empty-files" aria-label="Project files">
          {state?.files.map((path) => (
            <li key={path}>
              <button type="button" className="file-button" onClick={() => op({ type: "open", view: { kind: "file", path }, target: focused })}>
                {path}
              </button>
            </li>
          ))}
        </ul>
      )}
      <p className="hint">{shortcutHelp(PLATFORM)}</p>
    </div>
  );
}
