import { useEffect, useState, type KeyboardEvent, type ReactNode } from "react";
import type { AppState, Session, SessionMode, Task } from "./generated/protocol";
import { ChatDock, type CanvasTab } from "./Chat";
import { DiffView, type DiffTarget } from "./DiffView";
import { GameView } from "./GameView";
import { Icon } from "./Icon";
import { useAppSocket, type Conn, type DiffMsg, type Notice, type Send } from "./socket";


const VIEWS = ["explorer", "tasks", "devices"] as const;
type View = (typeof VIEWS)[number];

const VIEW_LABELS: Record<View, string> = { explorer: "Explorer", tasks: "Tasks", devices: "Devices" };

export function App() {
  const { state, conn, send, transcripts, fetchTranscript, accepted, notice, diffs, fetchDiff } = useAppSocket();
  const [view, setView] = useView();
  // Lifted so the chat's cards can switch it.
  const [tab, setTab] = useState<CanvasTab>("game");
  // The chat's current task (`null`: a new one), which the Diff tab reviews.
  const [selected, setSelected] = useState<number | null>(null);
  const [target, setTarget] = useState<DiffTarget | null>(null);
  const viewDiff = (path: string | null) => {
    setTab("diff");
    setTarget((t) => ({ path, seq: (t?.seq ?? 0) + 1 }));
  };
  const online = conn === "open" && state !== null;
  const task = state?.tasks.find((t) => t.id === selected && t.project === state.active) ?? null;
  return (
    <div className="app">
      <TitleBar state={state} conn={conn} />
      <div className="workspace">
        <Rail view={view} onSelect={setView} />
        <div className="panes">
          {view === "explorer" && <ExplorerPane state={state} online={online} send={send} />}
          {view === "tasks" && (
            <PlaceholderPane title="Tasks">
              Each conversation with the agent will be listed here until you keep or drop its change.
            </PlaceholderPane>
          )}
          {view === "devices" && (
            <PlaceholderPane title="Devices">
              Headsets you connect by cable or Wi-Fi will appear here.
            </PlaceholderPane>
          )}
          <Canvas
            state={state}
            online={online}
            send={send}
            tab={tab}
            onTab={setTab}
            review={{ task, diff: task ? diffs.get(task.id) : undefined, fetchDiff, notice, target }}
          />
          <ChatDock
            state={state}
            conn={conn}
            send={send}
            transcripts={transcripts}
            fetchTranscript={fetchTranscript}
            accepted={accepted}
            notice={notice}
            tab={tab}
            onTab={setTab}
            selected={selected}
            onSelect={setSelected}
            onViewDiff={viewDiff}
          />
        </div>
      </div>
    </div>
  );
}

/** The open rail view, kept in the URL hash so a reload reopens it. */
function useView(): [View, (v: View) => void] {
  const initial = VIEWS.find((v) => `#${v}` === location.hash) ?? "explorer";
  const [view, setView] = useState<View>(initial);
  const select = (v: View) => {
    history.replaceState(null, "", `#${v}`);
    setView(v);
  };
  return [view, select];
}

function activeProject(state: AppState | null) {
  return state?.projects.find((p) => p.path === state.active) ?? null;
}

/** One word for the session, as the title bar and session card show it. */
function sessionWord(session: Session | null | undefined): string {
  return session?.status.kind ?? "stopped";
}

function isLive(session: Session | null | undefined): boolean {
  const kind = session?.status.kind;
  return kind === "starting" || kind === "running";
}

function TitleBar({ state, conn }: { state: AppState | null; conn: Conn }) {
  const project = activeProject(state);
  const word = sessionWord(state?.session);
  // The app window shows it as its title.
  useEffect(() => {
    document.title = project ? `Functor — ${project.name}` : "Functor";
  }, [project?.name]);
  return (
    <header className="titlebar">
      <span className="brand">
        <img src="icon.svg" alt="" width={28} height={28} />
        Functor
      </span>
      <span className="title-project">{project?.name ?? (state ? "No project" : "")}</span>
      {state?.git && (
        <span className="pill mono branch" title={`${state.git.branch} · ${state.git.commit}`}>
          {state.git.branch} · {state.git.commit}
        </span>
      )}
      <span className="spacer" />
      <span role="status" className="conn">
        {conn !== "open" && (
          <span className="pill pill-warn">{conn === "connecting" ? "Connecting…" : "Reconnecting…"}</span>
        )}
      </span>
      {/* Dimmed while disconnected: the last state may no longer be true. */}
      <span className={`pill pill-session-${word}${conn === "open" ? "" : " pill-stale"}`}>Local session · {word}</span>
      <span className="pill">No headset</span>
    </header>
  );
}

function Rail({ view, onSelect }: { view: View; onSelect: (v: View) => void }) {
  return (
    <nav className="rail" aria-label="Views">
      {VIEWS.map((v) => (
        <button
          key={v}
          type="button"
          aria-label={VIEW_LABELS[v]}
          title={VIEW_LABELS[v]}
          aria-pressed={v === view}
          onClick={() => onSelect(v)}
        >
          <Icon name={v} />
        </button>
      ))}
    </nav>
  );
}

function ExplorerPane({ state, online, send }: { state: AppState | null; online: boolean; send: Send }) {
  const projects = state?.projects ?? [];
  const session = state?.session;
  const status = session?.status;
  const frame = status?.kind === "running" ? status.frame : null;
  const reason = status?.kind === "failed" || status?.kind === "exited" ? status.reason : null;
  return (
    <aside className="pane" aria-label="Explorer">
      <section className="stack">
        <label className="eyebrow" htmlFor="project-switcher">
          Project
        </label>
        <div className="switcher">
          <select
            id="project-switcher"
            value={state?.active ?? ""}
            disabled={!online || projects.length === 0}
            onChange={(e) => send({ type: "set_active_project", path: e.target.value })}
          >
            {projects.map((p) => (
              <option key={p.path} value={p.path}>
                {p.path}
              </option>
            ))}
          </select>
          <Icon name="chevron" />
        </div>
        {state && (
          <p className="hint">
            {projects.length === 0
              ? "No projects in this repo"
              : projects.length === 1
                ? "The only project in this repo"
                : `One of ${projects.length} projects in this repo`}
          </p>
        )}
        <ul className="files" aria-label="Files">
          {(state?.files ?? []).map((f) => (
            <li key={f} className={f.endsWith(".fun") ? undefined : "dim"}>
              {f}
            </li>
          ))}
        </ul>
      </section>
      <section className="card" aria-labelledby="session-heading">
        <h2 id="session-heading" className="eyebrow">
          Session
        </h2>
        <dl className="rows">
          <div>
            <dt>Status</dt>
            <dd data-testid="session-status">{sessionWord(session)}</dd>
          </div>
          <div>
            <dt>Frame</dt>
            <dd className="mono" data-testid="session-frame">
              {frame ?? "—"}
            </dd>
          </div>
        </dl>
        {reason && <p className="reason">{reason}</p>}
      </section>
    </aside>
  );
}

function PlaceholderPane({ title, children }: { title: string; children: ReactNode }) {
  return (
    <aside className="pane" aria-label={title}>
      <h2 className="pane-title">{title}</h2>
      <p className="hint">{children}</p>
    </aside>
  );
}

const TABS = [
  { id: "game", label: "Game" },
  { id: "code", label: "Code", text: "The project's code will open here." },
  { id: "diff", label: "Diff" },
] as const;

/** What the Diff tab shows: the current task's change. */
type Review = {
  task: Task | null;
  diff: DiffMsg | undefined;
  fetchDiff: (task: number) => void;
  notice: Notice | null;
  target: DiffTarget | null;
};

function Canvas({
  state,
  online,
  send,
  tab: id,
  onTab,
  review,
}: {
  state: AppState | null;
  online: boolean;
  send: Send;
  tab: CanvasTab;
  onTab: (tab: CanvasTab) => void;
  review: Review;
}) {
  const tab = TABS.findIndex((t) => t.id === id);
  // Arrow keys move between tabs, as in the ARIA tabs pattern.
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    // Only from a tab: the mode picker in the same bar keeps its arrow keys.
    const step = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
    if (!step || (e.target as HTMLElement).getAttribute("role") !== "tab") return;
    const next = TABS[(tab + step + TABS.length) % TABS.length]!;
    onTab(next.id);
    document.getElementById(`tab-${next.id}`)?.focus();
  };
  const current = TABS[tab]!;
  return (
    <main className="canvas">
      <div className="tabs" role="tablist" aria-label="Canvas" onKeyDown={onKeyDown}>
        {TABS.map((t, i) => (
          <button
            key={t.id}
            id={`tab-${t.id}`}
            type="button"
            role="tab"
            aria-selected={i === tab}
            aria-controls="canvas-panel"
            tabIndex={i === tab ? 0 : -1}
            onClick={() => onTab(t.id)}
          >
            {t.label}
          </button>
        ))}
        {current.id === "game" && (
          <>
            <span className="spacer" />
            {/* The one place to stop the game; starting it is the game view's empty state. */}
            {isLive(state?.session) && (
              <button type="button" className="button" disabled={!online} onClick={() => send({ type: "stop_session" })}>
                Stop game
              </button>
            )}
            <ModePicker state={state} online={online} send={send} />
          </>
        )}
      </div>
      <div className="canvas-body" id="canvas-panel" role="tabpanel" aria-labelledby={`tab-${current.id}`}>
        {current.id === "game" ? (
          <GameView state={state} online={online} send={send} />
        ) : current.id === "diff" ? (
          <DiffView state={state} online={online} send={send} {...review} />
        ) : (
          <div className="placeholder">
            <p>{current.text}</p>
          </div>
        )}
      </div>
    </main>
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
