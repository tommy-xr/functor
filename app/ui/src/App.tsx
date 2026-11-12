import { useEffect, useRef, useState, type ReactNode } from "react";
import type { AppState, ClientMsg, Session, View as TabView } from "./generated/protocol";
import { Canvas, type CanvasProps } from "./Canvas";
import { ChatDock } from "./Chat";
import { basename, pickHarness } from "./chat-model";
import { FolderActions, NoProject, type FolderProps } from "./Folders";
import { NewProjectModal } from "./NewProject";
import { Icon } from "./Icon";
import { focusedPane, panesOf, placementOf, tabTitles, viewKey } from "./layout-model";
import { HAS_TOKEN, useAppSocket, type Conn, type Send } from "./socket";
import { TasksPane } from "./TasksPane";
import { choiceKey, chosenTask, currentTask, needsYou } from "./tasks-model";


const VIEWS = ["explorer", "tasks", "devices"] as const;
type View = (typeof VIEWS)[number];

const VIEW_LABELS: Record<View, string> = { explorer: "Explorer", tasks: "Tasks", devices: "Devices" };

export function App() {
  const {
    state,
    conn,
    send,
    transcripts,
    fetchTranscript,
    accepted,
    notice,
    diffs,
    fetchDiff,
    files,
    fetchFile,
    dirs,
    listDirs,
    plan,
    checkProject,
  } = useAppSocket();
  const [view, setView] = useView();
  const online = conn === "open" && state !== null;
  // The chat's current task (`null`: a new one) is the core's, per project,
  // so the dock, the Tasks pane and every window agree. Until the core has
  // one (a restarted app forgets), this page's last, else the newest open
  // task, and the core is told so.
  const chosen = chosenTask(state);
  const seen = useRef(new Map<string, number | null>());
  if (state?.active && chosen !== undefined) seen.current.set(choiceKey(state), chosen);
  const selected = currentTask(state, seen.current);
  useEffect(() => {
    if (online && state.active && chosen === undefined) send({ type: "select_task", task: selected });
  }, [online, state?.active, chosen === undefined]);
  const setSelected = (task: number | null) => send({ type: "select_task", task });
  const [target, setTarget] = useState<CanvasProps["target"]>(null);
  // The chat's actions open or focus a view beside what the user is looking at.
  const openBeside = (view: TabView) => send({ type: "layout", op: { type: "open", view, target: { kind: "side" } } });
  const viewDiff = (path: string | null) => {
    if (selected === null) return;
    // The user's own target replaces a step's pointer into these changes.
    const h = state?.highlight;
    if (h?.view.kind === "changes" && h.view.task === selected) send({ type: "clear_highlight", seq: h.seq });
    openBeside({ kind: "changes", task: selected });
    setTarget((t) => ({ task: selected, path, seq: (t?.seq ?? 0) + 1 }));
  };
  const composer = () => document.querySelector<HTMLTextAreaElement>("textarea[aria-label='Message the agent']")?.focus();
  const newTask = () => {
    setSelected(null);
    composer();
  };
  // A task row: the chat's current task, and its page in the focused pane (or beside it).
  const openTask = (task: number, side: boolean) => {
    setSelected(task);
    send({ type: "layout", op: { type: "open", view: { kind: "task", task }, target: { kind: side ? "side" : "focused" } } });
  };
  const continueTask = (task: number) => {
    setSelected(task);
    composer();
  };
  const task = state?.tasks.find((t) => t.id === selected && t.project === state.active) ?? null;
  const folder: FolderProps = { state, online, send, dirs, listDirs };
  useWindowMenu(state, send);
  return (
    <>
    {/* Behind the new-project modal, nothing takes focus or clicks. */}
    <div className="app" inert={!!state?.new_project}>
      <TitleBar state={state} conn={conn} />
      <div className="workspace">
        <Rail view={view} onSelect={setView} waiting={needsYou(state?.tasks ?? [], state?.active ?? null)} />
        <div className="panes">
          {view === "explorer" && <ExplorerPane {...folder} />}
          {view === "tasks" && (
            <TasksPane
              state={state}
              online={online}
              send={send}
              transcripts={transcripts}
              fetchTranscript={fetchTranscript}
              notice={notice}
              selected={selected}
              onOpen={openTask}
              onNewTask={newTask}
            />
          )}
          {view === "devices" && (
            <PlaceholderPane title="Devices">
              Headsets you connect by cable or Wi-Fi will appear here.
            </PlaceholderPane>
          )}
          {state && !state.active ? (
            <NoProject {...folder} onNewProject={() => send({ type: "show_new_project", here: !!state.folder })} />
          ) : (
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
              transcripts={transcripts}
              fetchTranscript={fetchTranscript}
              onContinue={continueTask}
            />
          )}
          <ChatDock
            state={state}
            conn={conn}
            send={send}
            transcripts={transcripts}
            fetchTranscript={fetchTranscript}
            accepted={accepted}
            notice={notice}
            canvasWord={canvasWord(state)}
            // In Mini or its own window the game is already in view.
            onOpenGame={() => placementOf(state?.active ? state.layouts[state.active] : null) === "pane" && openBeside({ kind: "game" })}
            selected={selected}
            onSelect={setSelected}
            onViewDiff={viewDiff}
          />
        </div>
      </div>
    </div>
    {state?.new_project && (
      <NewProjectModal
        {...folder}
        plan={plan}
        checkProject={checkProject}
        harness={pickHarness(state)}
      />
    )}
    </>
  );
}

/** The window's own hooks into the page; absent in a browser. */
type WindowHooks = {
  functorMenu?: (msg: ClientMsg) => void;
  ipc?: { postMessage: (text: string) => void };
};

/**
 * The window's menu bar: Open Folder and Open Recent arrive as messages to
 * send like a button's, and the recent projects go out for Open Recent.
 */
function useWindowMenu(state: AppState | null, send: Send) {
  const hooks = window as WindowHooks;
  useEffect(() => {
    hooks.functorMenu = (msg) => {
      if (msg.type === "pick_folder" || msg.type === "open_folder" || msg.type === "show_new_project") send(msg);
    };
  }, [send]);
  const listed = (state?.recents ?? []).filter((r) => !r.missing).map(({ name, path }) => ({ name, path }));
  const recents = JSON.stringify({ recents: listed });
  useEffect(() => {
    if (state) hooks.ipc?.postMessage(recents);
  }, [recents, state === null]);
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
  // The project, else the open folder.
  const name = activeProject(state)?.name ?? (state?.folder ? basename(state.folder) : null);
  const word = sessionWord(state?.session);
  // The app window shows it as its title.
  useEffect(() => {
    document.title = name ? `Functor — ${name}` : "Functor";
  }, [name]);
  return (
    <header className="titlebar">
      <span className="brand">
        <img src="icon.svg" alt="" width={28} height={28} />
        Functor
      </span>
      <span className="title-project">{name ?? (state ? "No folder open" : "")}</span>
      {state?.git && (
        <span className="pill mono branch" title={`${state.git.branch} · ${state.git.commit}`}>
          {state.git.branch} · {state.git.commit}
        </span>
      )}
      <span className="spacer" />
      <span role="status" className="conn">
        {conn !== "open" && (
          <span className="pill pill-warn">
            {!HAS_TOKEN
              ? "No session token — open the URL functor-app printed"
              : conn === "connecting"
                ? "Connecting…"
                : "Reconnecting…"}
          </span>
        )}
      </span>
      {/* Dimmed while disconnected: the last state may no longer be true. */}
      <span className={`pill pill-session-${word}${conn === "open" ? "" : " pill-stale"}`}>Local session · {word}</span>
      <span className="pill">No headset</span>
    </header>
  );
}

/** `waiting`: the active project's tasks that need the user, badged on Tasks. */
function Rail({ view, onSelect, waiting }: { view: View; onSelect: (v: View) => void; waiting: number }) {
  return (
    <nav className="rail" aria-label="Views">
      {VIEWS.map((v) => {
        const badge = v === "tasks" && waiting > 0;
        const label = badge ? `${VIEW_LABELS[v]}, ${waiting} need${waiting === 1 ? "s" : ""} you` : VIEW_LABELS[v];
        return (
          <button key={v} type="button" aria-label={label} title={label} aria-pressed={v === view} onClick={() => onSelect(v)}>
            <Icon name={v} />
            {badge && (
              <span className="rail-badge" aria-hidden="true" data-testid="tasks-badge">
                {waiting > 99 ? "99+" : waiting}
              </span>
            )}
          </button>
        );
      })}
    </nav>
  );
}

function ExplorerPane(props: FolderProps) {
  const { state, online, send } = props;
  const projects = state?.projects ?? [];
  const where = state?.git ? "repo" : "folder";
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
                {/* The folder itself: its name, not `.`. */}
                {p.path === "." ? p.name : p.path}
              </option>
            ))}
          </select>
          <Icon name="chevron" />
        </div>
        {state && (
          <p className="hint">
            {!state.folder
              ? "No folder open"
              : projects.length === 0
                ? `No projects in this ${where}`
                : projects.length === 1
                  ? `The only project in this ${where}`
                  : `One of ${projects.length} projects in this ${where}`}
          </p>
        )}
        <FolderActions {...props} />
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
