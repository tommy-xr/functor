import { useState, type KeyboardEvent, type ReactNode } from "react";
import type { AppState, ClientMsg, Session } from "./generated/protocol";
import { useAppSocket, type Conn } from "./socket";

type Send = (msg: ClientMsg) => void;

const VIEWS = ["explorer", "tasks", "devices"] as const;
type View = (typeof VIEWS)[number];

const VIEW_LABELS: Record<View, string> = { explorer: "Explorer", tasks: "Tasks", devices: "Devices" };

export function App() {
  const { state, conn, send } = useAppSocket();
  const [view, setView] = useView();
  const online = conn === "open" && state !== null;
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
          <Canvas />
          <ChatDock />
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
  return (
    <header className="titlebar">
      <span className="brand">
        <img src="icon.svg" alt="" width={28} height={28} />
        Functor
      </span>
      <span className="title-project">{project?.name ?? (state ? "No project" : "")}</span>
      {state?.git && (
        <span className="pill mono" title="Branch · commit">
          {state.git.branch} · {state.git.commit}
        </span>
      )}
      <span className="spacer" />
      <span role="status" className="conn">
        {conn !== "open" && (
          <span className="pill pill-warn">{conn === "connecting" ? "Connecting…" : "Reconnecting…"}</span>
        )}
      </span>
      <span className={`pill pill-session-${word}`}>Local session · {word}</span>
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
  const live = isLive(session);
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
        <p className="hint">
          {projects.length === 1 ? "The only project" : `One of ${projects.length} projects`} in this repo
        </p>
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
        <div className="actions">
          <button
            type="button"
            className="button button-primary"
            disabled={!online || live || !state?.active}
            onClick={() => send({ type: "start_session" })}
          >
            Start
          </button>
          <button
            type="button"
            className="button"
            disabled={!online || !live}
            onClick={() => send({ type: "stop_session" })}
          >
            Stop
          </button>
        </div>
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
  { id: "game", label: "Game", text: "The running game will appear here, with its timeline below it." },
  { id: "code", label: "Code", text: "The project's code will open here." },
  { id: "diff", label: "Diff", text: "Changes the agent makes will appear here for review." },
] as const;

function Canvas() {
  const [tab, setTab] = useState(0);
  // Arrow keys move between tabs, as in the ARIA tabs pattern.
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const step = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
    if (!step) return;
    const next = (tab + step + TABS.length) % TABS.length;
    setTab(next);
    document.getElementById(`tab-${TABS[next]!.id}`)?.focus();
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
            onClick={() => setTab(i)}
          >
            {t.label}
          </button>
        ))}
      </div>
      <div className="canvas-body" id="canvas-panel" role="tabpanel" aria-labelledby={`tab-${current.id}`}>
        <div className="placeholder">
          <p>{current.text}</p>
        </div>
      </div>
    </main>
  );
}

/** Drawn but inert until there is an agent to talk to. */
function ChatDock() {
  return (
    <section className="chat" aria-labelledby="chat-heading">
      <header className="chat-head">
        <h2 id="chat-heading">Agent</h2>
      </header>
      <div className="chat-body">
        <p className="hint">Conversations with an agent that sees the live game will appear here.</p>
      </div>
      <div className="composer-wrap">
        <div className="composer" aria-disabled="true">
          <textarea aria-label="Message the agent" rows={2} placeholder="The agent is not available yet" disabled />
          <div className="composer-row">
            <button type="button" className="round" aria-label="Attach a capture or file" disabled>
              <Icon name="plus" />
            </button>
            <button type="button" className="model" aria-label="Choose the model" disabled>
              claude
              <Icon name="chevron" />
            </button>
            <span className="spacer" />
            <button type="button" className="round round-plain" aria-label="Hold to talk" disabled>
              <Icon name="mic" />
            </button>
            <button type="button" className="round round-send" aria-label="Send" disabled>
              <Icon name="send" />
            </button>
          </div>
        </div>
      </div>
    </section>
  );
}

const ICONS: Record<string, ReactNode> = {
  explorer: <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />,
  tasks: <path d="M9 6h11M9 12h11M9 18h11M4 6l1 1 2-2M4 12l1 1 2-2M4 18l1 1 2-2" />,
  devices: <path d="M4 9a3 3 0 0 1 3-3h10a3 3 0 0 1 3 3v6a2 2 0 0 1-2 2h-3l-2-3h-2l-2 3H6a2 2 0 0 1-2-2z" />,
  chevron: <path d="M6 9l6 6 6-6" />,
  plus: <path d="M12 5v14M5 12h14" />,
  mic: (
    <>
      <rect x="9" y="3" width="6" height="11" rx="3" />
      <path d="M5 11a7 7 0 0 0 14 0M12 18v3" />
    </>
  ),
  send: <path d="M12 19V5M6 11l6-6 6 6" />,
};

function Icon({ name }: { name: keyof typeof ICONS }) {
  return (
    <svg
      className="icon"
      aria-hidden="true"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      {ICONS[name]}
    </svg>
  );
}
