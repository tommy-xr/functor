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
  type RefObject,
} from "react";
import { createPortal } from "react-dom";
import type { AppState, Direction, GamePlacement, Highlight, Item, LayoutOp, Mini, Node, Pane, Placement, Session, Task, View } from "./generated/protocol";
import { DiffView, type DiffTarget } from "./DiffView";
import { hasChange } from "./diff-model";
import { FileView } from "./FileView";
import { TaskPage } from "./TaskPage";
import { GameView } from "./GameView";
import { useTransport } from "./TransportBar";
import { Icon } from "./Icon";
import { useMenu } from "./menu";
import { DEFAULT_ASPECT, isLeft, isTop, miniKey, miniRect, nearestCorner, resizedWidth, clampWidth } from "./mini-model";
import {
  agentTabs,
  focusedPane,
  isTyping,
  STACK_WIDTH,
  placementOf,
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
  transcripts: ReadonlyMap<number, Item[]>;
  fetchTranscript: (task: number) => void;
  /** Makes `task` the chat's current task and puts the cursor in its composer. */
  onContinue: (task: number) => void;
};

/** What every pane needs, passed down the split tree. */
type Ctx = CanvasProps & {
  op: (op: LayoutOp) => boolean;
  titles: Map<string, string>;
  focus: number | null;
  /** Where the active project's game view shows. */
  placement: Placement;
  /** With an op's `send` result: DOM focus follows into the focused pane once its layout arrives. */
  refocus: (sent: boolean) => void;
  /** The views the agent opened, by `viewKey`. */
  agentTabs: Set<string>;
  /** What the agent or a walkthrough step points at in this project. */
  highlight: Highlight | null;
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

  // After a keyboard action, focus follows the layout the core sends back:
  // the first that differs from the one it acted on, within a second.
  const main = useRef<HTMLElement>(null);
  const shown = useRef(layout);
  shown.current = layout;
  const wantFocus = useRef<{ from: string; until: number } | null>(null);
  const refocus = useCallback((sent: boolean) => {
    if (sent) wantFocus.current = { from: JSON.stringify(shown.current), until: Date.now() + 1000 };
  }, []);
  useLayoutEffect(() => {
    const want = wantFocus.current;
    if (!want || !main.current || JSON.stringify(layout) === want.from) return;
    wantFocus.current = null;
    // Never out of a field the user has gone on to type in.
    if (Date.now() > want.until || isTyping(document.activeElement)) return;
    const pane = main.current.querySelector(`[data-pane="${layout?.focus}"]`) ?? main.current;
    pane.querySelector<HTMLElement>("[role=tab][aria-selected=true], .pane-actions button, .empty-canvas button")?.focus();
  }, [layout]);

  useShortcuts(layout, op, refocus);
  useStackedReport(main, props.online, send);
  useEffect(() => {
    const track = (e: FocusEvent) => {
      const el = e.target instanceof HTMLElement ? e.target : null;
      const pane = el?.closest<HTMLElement>("[data-pane]");
      lastFocus = pane ? { pane: Number(pane.dataset.pane), id: el?.id ?? "" } : null;
    };
    document.addEventListener("focusin", track);
    return () => document.removeEventListener("focusin", track);
  }, []);
  const placement = placementOf(layout);
  const highlight = state?.highlight?.project === state?.active ? (state?.highlight ?? null) : null;
  const ctx: Ctx = { ...props, op, titles, focus: layout?.focus ?? null, placement, refocus, agentTabs: agentTabs(layout), highlight };
  return (
    <main className="canvas" ref={main} aria-label="Canvas">
      {placement === "window" && <WindowNote ctx={ctx} />}
      {layout && (layout.agent.restore || ctx.agentTabs.size > 0) && <AgentBar ctx={ctx} restorable={!!layout.agent.restore} />}
      <GameHost state={state} online={props.online} send={send}>
        <div className="canvas-body">
          {layout?.root ? <NodeView key={nodeId(layout.root)} node={layout.root} ctx={ctx} /> : <EmptyCanvas ctx={ctx} />}
        </div>
        {/* Keyed by project, so a preview never carries over to another. */}
        {placement === "mini" && layout && <MiniGame key={state?.active} ctx={ctx} game={layout.game} canvas={main} />}
      </GameHost>
    </main>
  );
}

/**
 * Tells the core when the canvas stacks its panes (`STACK_WIDTH`), so the
 * agent's views make no splits there: on each change and each connect.
 */
function useStackedReport(main: RefObject<HTMLElement | null>, online: boolean, send: CanvasProps["send"]) {
  useEffect(() => {
    const el = main.current;
    if (!el || !online) return;
    let last: boolean | null = null;
    const report = () => {
      const style = getComputedStyle(el);
      const width = el.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight);
      const stacked = width <= STACK_WIDTH;
      if (stacked !== last && send({ type: "canvas_stacked", stacked })) last = stacked;
    };
    report();
    const observer = new ResizeObserver(report);
    observer.observe(el);
    return () => observer.disconnect();
  }, [online, send]);
}

/**
 * Shown while the agent has arranged views: "Restore my layout" (back to
 * before its first change) and "Close agent tabs". It floats, so it never
 * moves what the user is reading.
 */
function AgentBar({ ctx, restorable }: { ctx: Ctx; restorable: boolean }) {
  const keep = (e: { preventDefault: () => void }) => e.preventDefault();
  return (
    <div className="agent-bar" role="group" aria-label="Views the agent arranged" data-testid="agent-bar">
      <span className="agent-dot" aria-hidden="true" />
      <span className="agent-bar-text">The agent arranged views</span>
      {restorable && (
        <button type="button" className="link-button" disabled={!ctx.online} onMouseDown={keep} onClick={() => ctx.op({ type: "restore" })}>
          Restore my layout
        </button>
      )}
      {ctx.agentTabs.size > 0 && (
        <button type="button" className="link-button" disabled={!ctx.online} onMouseDown={keep} onClick={() => ctx.op({ type: "close_agent_tabs" })}>
          Close agent tabs
        </button>
      )}
    </div>
  );
}

/** Global layout shortcuts (see `shortcutFor`), acting on the focused pane. */
function useShortcuts(
  layout: AppState["layouts"][string] | null,
  op: (op: LayoutOp) => boolean,
  refocus: (sent: boolean) => void,
) {
  const live = useRef(layout);
  live.current = layout;
  useEffect(() => {
    const onKeyDown = (e: globalThis.KeyboardEvent) => {
      const target = e.target instanceof HTMLElement ? e.target : null;
      // Not behind a modal dialog (the pairing prompt, new project).
      if (target?.closest("[aria-modal='true']")) return;
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
      let sent: boolean;
      if (step && pane.tabs.length > 1) {
        const view = pane.tabs[(pane.active + step + pane.tabs.length) % pane.tabs.length]!;
        sent = op({ type: "open", view, target: { kind: "pane", pane: pane.id } });
      } else if (action === "close_tab") {
        const view = pane.tabs[pane.active];
        sent = op(view ? { type: "close", view } : { type: "close_pane", pane: pane.id });
      } else if (action === "split_right") {
        sent = op({ type: "split", pane: pane.id, direction: "right" });
      } else if (action === "next_pane" && panes.length > 1) {
        const next = panes[(panes.indexOf(pane) + 1) % panes.length]!;
        sent = op({ type: "focus", pane: next.id });
      } else {
        return;
      }
      refocus(sent);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [op, refocus]);
}

/**
 * The one game view, kept mounted in a detached element that moves into
 * whichever slot claims it (the pane showing the Game tab, or the Mini
 * card): moving it keeps its stream and input capture. Unmounted while no
 * slot shows it.
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

/** Where the game view shows: the pane whose Game tab is shown, or the Mini card. */
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

/**
 * Where keyboard focus last was in a pane (its id, and the element's id if
 * it has one). A split around a pane (the agent opening a view beside it)
 * mounts it anew, dropping that focus to the page; the new pane takes it back.
 */
let lastFocus: { pane: number; id: string } | null = null;

/** Keyed by id, so a node's local state (a splitter preview) never passes to another. */
function nodeId(node: Node) {
  return `${node.kind}-${node.id}`;
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
  // Ratios sent and not yet seen back. The core's ratio replaces the preview
  // once it is the last one sent, or one this page never sent (another window).
  const pending = useRef<number[]>([]);
  useEffect(() => {
    const at = pending.current.indexOf(split.ratio);
    pending.current = at < 0 ? [] : pending.current.slice(at + 1);
    if (!pending.current.length) setDragged(null);
  }, [split.ratio]);
  const ratio = dragged ?? split.ratio;
  const right = split.direction === "right";
  /**
   * Sends `r` (clamped) unless it is `from`; the ratio to show until the
   * core's arrives: the clamped one, or null (the core's) when unsent.
   */
  const resize = (r: number, from: number) => {
    const clamped = Math.round(Math.min(Math.max(r, RATIO_MIN), RATIO_MAX));
    if (clamped === from) return from === split.ratio ? null : from;
    if (!ctx.op({ type: "resize", split: split.id, ratio: clamped })) return null;
    pending.current.push(clamped);
    return clamped;
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
        <NodeView key={nodeId(split.first)} node={split.first} ctx={ctx} />
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
        <NodeView key={nodeId(split.second)} node={split.second} ctx={ctx} />
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
  // The shown tab stays in view in a bar that scrolls: the bar alone
  // scrolls, never the page (an agent's tab must not move what the user reads).
  useEffect(() => {
    const tab = active && document.getElementById(tabId(pane.id, active));
    const bar = tab && tab.closest<HTMLElement>(".pane-tabs");
    if (!tab || !bar) return;
    const t = tab.getBoundingClientRect();
    const b = bar.getBoundingClientRect();
    if (t.left < b.left) bar.scrollLeft -= b.left - t.left;
    else if (t.right > b.right) bar.scrollLeft += t.right - b.right;
  }, [pane.id, active && viewKey(active)]);
  // Native listeners, not React's: the game view is portalled in from
  // `GameHost`, so its React events never pass through this pane.
  const box = useRef<HTMLElement>(null);
  // Mounted anew (see `lastFocus`): focus that fell to the page comes back.
  useLayoutEffect(() => {
    const lost = document.activeElement === null || document.activeElement === document.body;
    if (!lost || lastFocus?.pane !== pane.id) return;
    const el = (lastFocus.id && document.getElementById(lastFocus.id)) || box.current?.querySelector<HTMLElement>("[role=tab][aria-selected=true]");
    el?.focus({ preventScroll: true });
  }, []);
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
      ctx.refocus(op({ type: "close", view: pane.tabs[i]! }));
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
            const byAgent = ctx.agentTabs.has(viewKey(view));
            return (
              <div
                key={viewKey(view)}
                className={`tab${byAgent ? " tab-agent" : ""}`}
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
                  title={byAgent ? `${title} (opened by the agent)` : title}
                  onClick={() => show(view)}
                  onKeyDown={(e) => onTabKey(e, i)}
                >
                  {byAgent && <span className="agent-dot" data-testid="agent-mark" aria-label="Opened by the agent" />}
                  {title}
                </button>
                {/* A running game keeps its tab (the core refuses): Stop game, or Mini, first. */}
                {!(view.kind === "game" && isLive(ctx.state?.session)) && (
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
                )}
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
        {active ? <ViewBody view={active} pane={pane.id} ctx={ctx} /> : <EmptyPane pane={pane} ctx={ctx} />}
      </div>
    </section>
  );
}

function ViewBody({ view, pane, ctx }: { view: View; pane: number; ctx: Ctx }) {
  const { state } = ctx;
  switch (view.kind) {
    case "game":
      return <GameSlot />;
    case "file": {
      const h = ctx.highlight;
      const mark = h?.view.kind === "file" && h.view.path === view.path && h.lines ? { lines: h.lines, seq: h.seq } : null;
      return (
        <FileView
          path={view.path}
          file={state?.active ? ctx.files.get(fileKey(state.active, view.path)) : undefined}
          fetchFile={ctx.fetchFile}
          online={ctx.online}
          mark={mark}
          onUnmark={(seq) => ctx.send({ type: "clear_highlight", seq })}
        />
      );
    }
    case "changes": {
      const task = state?.tasks.find((t) => t.id === view.task) ?? null;
      // The core's highlight counts down, so it never repeats a click's `seq`.
      const h = ctx.highlight;
      const pointed = h?.view.kind === "changes" && h.view.task === view.task && h.file ? { path: h.file, seq: -h.seq } : null;
      return (
        <DiffView
          state={state}
          online={ctx.online}
          send={ctx.send}
          task={task}
          diff={ctx.diffs.get(view.task)}
          fetchDiff={ctx.fetchDiff}
          notice={ctx.notice}
          target={pointed ?? (ctx.target?.task === view.task ? ctx.target : null)}
        />
      );
    }
    case "task": {
      const task = state?.tasks.find((t) => t.id === view.task) ?? null;
      // Beside this page's pane, not the one that happens to have the focus.
      const openChanges = () => {
        ctx.op({ type: "focus", pane });
        ctx.op({ type: "open", view: { kind: "changes", task: view.task }, target: { kind: "side" } });
      };
      return (
        <TaskPage
          state={state}
          task={task}
          items={ctx.transcripts.get(view.task)}
          fetchTranscript={ctx.fetchTranscript}
          online={ctx.online}
          send={ctx.send}
          notice={ctx.notice}
          onContinue={() => ctx.onContinue(view.task)}
          onOpenChanges={openChanges}
        />
      );
    }
  }
}

/** Start or stop and the placement control, in the tab bar while the Game tab shows (D21, D22). */
function GameControls({ ctx }: { ctx: Ctx }) {
  return (
    <div className="pane-game">
      <StartStopGame ctx={ctx} />
      <PlacementControl ctx={ctx} />
    </div>
  );
}

/**
 * The one start/stop control (D22): Stop game ends the process; Start game
 * brings it back paused on its first frame (D37). Play is the transport's.
 */
function StartStopGame({ ctx, compact = false }: { ctx: Ctx; compact?: boolean }) {
  const live = isLive(ctx.state?.session);
  const label = live ? "Stop game" : "Start game";
  const disabled = !ctx.online || (!live && !ctx.state?.active);
  const toggle = () => ctx.send({ type: live ? "stop_session" : "start_session" });
  if (compact) {
    return (
      <button type="button" className="icon-button" aria-label={label} title={label} disabled={disabled} onClick={toggle}>
        <Icon name={live ? "stop" : "power"} />
      </button>
    );
  }
  return (
    <button type="button" className="button button-compact" disabled={disabled} onClick={toggle}>
      {label}
    </button>
  );
}

const PLACEMENTS: { placement: Placement; label: string; title: string }[] = [
  { placement: "pane", label: "Pane", title: "Show the game in a pane" },
  { placement: "mini", label: "Mini", title: "Float the game over the canvas" },
  { placement: "window", label: "Own window", title: "Run the game in its own window (restarts a running game)" },
];

/** Where the game shows; to or from its own window restarts a live session. */
function PlacementControl({ ctx }: { ctx: Ctx }) {
  const current = ctx.placement;
  return (
    <div className="segmented" role="group" aria-label="Where the game shows">
      {PLACEMENTS.map(({ placement, label, title }) => (
        <button
          key={placement}
          type="button"
          aria-pressed={placement === current}
          title={title}
          disabled={!ctx.online}
          onClick={() => placement !== current && ctx.op({ type: "place", placement })}
        >
          {label}
        </button>
      ))}
    </div>
  );
}

/** The game in its own window: one line saying so, and the ways back (D21). */
function WindowNote({ ctx }: { ctx: Ctx }) {
  const status = ctx.state?.session?.status;
  const live = isLive(ctx.state?.session);
  // An ended session says why, as the game view would.
  const reason = status?.kind === "failed" || status?.kind === "exited" ? status.reason : null;
  return (
    <div className="game-note" data-testid="game-note">
      <p className={reason ? "reason" : undefined}>
        {reason ??
          (ctx.state?.no_gpu
            ? "Own window: sessions here run without a GPU, so there is no window to show."
            : live
              ? "The game is running in its own window."
              : "The game runs in its own window.")}
      </p>
      <StartStopGame ctx={ctx} />
      <button type="button" className="button button-compact" disabled={!ctx.online} onClick={() => ctx.op({ type: "place", placement: "pane" })}>
        Bring back to a pane
      </button>
    </div>
  );
}

/** The mini view's title strip height (CSS: `.mini-strip`). */
const MINI_STRIP = 36;
/** The compact transport bar's height under it (CSS: `.transport-compact`). */
const MINI_TRANSPORT = 33;

/** `el`'s padding box in CSS pixels, following resizes. */
function useClientSize(el: RefObject<HTMLElement | null>) {
  const [size, setSize] = useState<{ w: number; h: number } | null>(null);
  useLayoutEffect(() => {
    const target = el.current;
    if (!target) return;
    const read = () => {
      const { clientWidth: w, clientHeight: h } = target;
      setSize((s) => (s?.w === w && s.h === h ? s : { w, h }));
    };
    read();
    const observer = new ResizeObserver(read);
    observer.observe(target);
    return () => observer.disconnect();
  }, [el]);
  return size;
}

/**
 * The game floating over the canvas (D21 Mini): the one game view, moved
 * into this card. Dragged by its strip and let go, it snaps to the nearest
 * corner; its corner handle resizes it at the game's aspect ratio. Only the
 * drag is local: a let-go sends the corner and width to the core.
 */
function MiniGame({ ctx, game, canvas }: { ctx: Ctx; game: GamePlacement; canvas: RefObject<HTMLElement | null> }) {
  const { mini } = game;
  const size = useClientSize(canvas);
  const status = ctx.state?.session?.status;
  const surface = status?.kind === "running" ? status.surface : null;
  const aspect = surface ? surface[0] / surface[1] : DEFAULT_ASPECT;
  // The card's chrome: its strip, and the transport bar while one shows.
  const { transport } = useTransport();
  const chrome = MINI_STRIP + (status?.kind === "running" && transport?.clock ? MINI_TRANSPORT : 0);
  // Sent and not yet answered. The core answers every op with its state (a
  // new object, changed or not), so the next one replaces the preview.
  const [sent, setSent] = useState<Mini | null>(null);
  useEffect(() => setSent(null), [game]);
  const [drag, setDrag] = useState<{ x: number; y: number; dx: number; dy: number; resize: boolean } | null>(null);
  // Unmeasured, it is laid out unseen: the game view moves in at once, never remounted.
  const canvasSize = size ?? { w: 0, h: 0 };
  // Resizing starts from the width shown, which a small canvas may have cut.
  const fitted = miniRect(sent ?? mini, aspect, canvasSize, chrome);
  const shown = { ...(sent ?? mini), width: size ? clampWidth(fitted.w) : (sent ?? mini).width };
  const width = drag?.resize ? resizedWidth(shown.corner, shown.width, drag.dx, drag.dy, aspect) : shown.width;
  const rect = miniRect({ ...shown, width }, aspect, canvasSize, chrome);
  // Moved, the card stays inside the canvas.
  const within = (v: number, room: number) => Math.min(Math.max(v, 0), Math.max(0, room));
  const x = drag && !drag.resize ? within(rect.x + drag.dx, canvasSize.w - rect.w) : rect.x;
  const y = drag && !drag.resize ? within(rect.y + drag.dy, canvasSize.h - rect.h) : rect.y;

  const put = (next: Mini) => {
    const now = sent ?? mini;
    if (next.corner === now.corner && next.width === now.width) return;
    if (ctx.op({ type: "set_mini", mini: next })) setSent(next);
  };
  const begin = (e: PointerEvent<HTMLElement>, resize: boolean) => {
    // The strip's controls are clicked, not dragged; its grip and the rest drag.
    if (e.button !== 0 || (!resize && (e.target as HTMLElement).closest("button:not(.mini-grip)"))) return;
    e.preventDefault();
    e.currentTarget.setPointerCapture(e.pointerId);
    setDrag({ x: e.clientX, y: e.clientY, dx: 0, dy: 0, resize });
  };
  const move = (e: PointerEvent<HTMLElement>) => {
    if (drag && e.currentTarget.hasPointerCapture(e.pointerId)) setDrag({ ...drag, dx: e.clientX - drag.x, dy: e.clientY - drag.y });
  };
  const end = (e: PointerEvent<HTMLElement>) => {
    if (!drag || !e.currentTarget.hasPointerCapture(e.pointerId)) return;
    setDrag(null);
    if (drag.resize) put({ ...shown, width: clampWidth(rect.w) });
    // A move keeps the width asked for, not the one a small canvas cut it to.
    else put({ ...(sent ?? mini), corner: nearestCorner(x + rect.w / 2, y + rect.h / 2, canvasSize) });
  };
  const handlers = { onPointerMove: move, onPointerUp: end, onPointerCancel: () => setDrag(null) };
  const onKeyDown = (e: KeyboardEvent) => {
    const next = miniKey(e.key.startsWith("Arrow") ? (sent ?? mini) : shown, e.key);
    if (!next) return;
    e.preventDefault();
    put(next);
  };
  const corner = `mini-${isTop(shown.corner) ? "top" : "bottom"}-${isLeft(shown.corner) ? "left" : "right"}`;
  return (
    <section
      className={`mini ${corner}${drag ? " mini-dragging" : ""}`}
      aria-label="Game (mini)"
      data-testid="mini"
      style={{ left: x, top: y, width: rect.w, height: rect.h, visibility: size ? undefined : "hidden" }}
    >
      <div className="mini-strip" onPointerDown={(e) => begin(e, false)} {...handlers}>
        <button
          type="button"
          className="mini-grip"
          aria-label="Move the game view"
          title="Drag to move it to a corner. Keys: arrows move it, + and − resize it"
          onKeyDown={onKeyDown}
        >
          Game
        </button>
        <PlacementControl ctx={ctx} />
        <StartStopGame ctx={ctx} compact />
        <button
          type="button"
          className="icon-button"
          aria-label="Dock"
          title="Dock: back to its pane"
          disabled={!ctx.online}
          onClick={() => ctx.op({ type: "place", placement: "pane" })}
        >
          <Icon name="dock" />
        </button>
      </div>
      <div className="mini-body">
        <GameSlot />
      </div>
      <div className="mini-resize" aria-hidden="true" onPointerDown={(e) => begin(e, true)} {...handlers} />
    </section>
  );
}

/** Views to offer in a pane: the game if it is not open, the files, the current task's changes. */
function offers(ctx: Ctx): { view: View; label: string }[] {
  const { state, currentTask } = ctx;
  const open = new Set(ctx.titles.keys());
  const out: { view: View; label: string }[] = [];
  if (!open.has("game") && ctx.placement === "pane") out.push({ view: { kind: "game" }, label: "Game" });
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
  const layout = ctx.state?.active ? ctx.state.layouts[ctx.state.active] : undefined;
  const arranges = layout?.agent.arranges ?? true;
  const pick = (op: LayoutOp) => {
    setOpen(false);
    ctx.refocus(ctx.op(op));
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
          <button
            type="button"
            role="menuitemcheckbox"
            aria-checked={arranges}
            title="When on, the agent can open views beside yours and its walkthroughs open when a run ends"
            onClick={() => pick({ type: "agent_arranges", on: !arranges })}
          >
            <span className="menu-check" aria-hidden="true">
              {arranges ? "✓" : ""}
            </span>
            <span className="menu-title">Let the agent arrange views</span>
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
        {/* In Mini or its own window the game has its own Start game. */}
        {ctx.placement === "pane" && (
          <button
            type="button"
            className="button button-primary"
            disabled={!online || !state?.active}
            // In a pane, a starting session opens its Game tab here.
            onClick={() => send({ type: "start_session" })}
          >
            Start game
          </button>
        )}
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
