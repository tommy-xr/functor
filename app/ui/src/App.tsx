import { useState, type ReactNode } from "react";
import type { AppState, Session, View as TabView } from "./generated/protocol";
import { Canvas, type CanvasProps } from "./Canvas";
import { ChatDock } from "./Chat";
import { Icon } from "./Icon";
import { focusedPane, panesOf, tabTitles, viewKey } from "./layout-model";
import { useAppSocket, type Conn, type Send } from "./socket";


const VIEWS = ["explorer", "tasks", "devices"] as const;
type View = (typeof VIEWS)[number];

const VIEW_LABELS: Record<View, string> = { explorer: "Explorer", tasks: "Tasks", devices: "Devices" };

export function App() {
  const { state, conn, send, transcripts, fetchTranscript, accepted, notice, diffs, fetchDiff, files, fetchFile } =
    useAppSocket();
  const [view, setView] = useView();
  // The chat's current task (`null`: a new one).
  const [selected, setSelected] = useState<number | null>(null);
  const [target, setTarget] = useState<CanvasProps["target"]>(null);
  // The chat's actions open or focus a view beside what the user is looking at.
  const openBeside = (view: TabView) => send({ type: "layout", op: { type: "open", view, target: { kind: "side" } } });
  const viewDiff = (path: string | null) => {
    if (selected === null) return;
    openBeside({ kind: "changes", task: selected });
    setTarget((t) => ({ task: selected, path, seq: (t?.seq ?? 0) + 1 }));
  };
  const newTask = () => {
    setSelected(null);
    document.querySelector<HTMLTextAreaElement>("textarea[aria-label='Message the agent']")?.focus();
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
            diffs={diffs}
            fetchDiff={fetchDiff}
            notice={notice}
            target={target}
            files={files}
            fetchFile={fetchFile}
            currentTask={task}
            onNewTask={newTask}
          />
          <ChatDock
            state={state}
            conn={conn}
            send={send}
            transcripts={transcripts}
            fetchTranscript={fetchTranscript}
            accepted={accepted}
            notice={notice}
            canvasWord={canvasWord(state)}
            onOpenGame={() => openBeside({ kind: "game" })}
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

/** The focused tab, as the composer's context chip names it. */
function canvasWord(state: AppState | null): string {
  const layout = state?.active ? state.layouts[state.active] : undefined;
  const pane = focusedPane(layout);
  const view = pane?.tabs[pane.active];
  if (!view) return "Nothing open";
  const views = panesOf(layout).flatMap((p) => p.tabs);
  return `${tabTitles(views, state?.tasks ?? []).get(viewKey(view))} tab`;
}

function TitleBar({ state, conn }: { state: AppState | null; conn: Conn }) {
  const project = activeProject(state);
  const word = sessionWord(state?.session);
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
  const openFile = (path: string, side: boolean) =>
    send({ type: "layout", op: { type: "open", view: { kind: "file", path }, target: { kind: side ? "side" : "focused" } } });
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
        {/* A click opens the file in the focused pane; Alt-click or the side button, beside it. */}
        <ul className="files" aria-label="Files">
          {(state?.files ?? []).map((f) => (
            <li key={f} className={f.endsWith(".fun") ? undefined : "dim"}>
              <button
                type="button"
                className="file-button"
                disabled={!online}
                title={`Open ${f} (Alt-click: to the side)`}
                onClick={(e) => openFile(f, e.altKey)}
              >
                {f}
              </button>
              <button
                type="button"
                className="file-side"
                disabled={!online}
                aria-label={`Open ${f} to the side`}
                title="Open to the side"
                onClick={() => openFile(f, true)}
              >
                <Icon name="split-right" />
              </button>
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
