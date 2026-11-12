import { useEffect, useRef, useState, type KeyboardEvent, type MouseEvent } from "react";
import type { AppState, Item, Task } from "./generated/protocol";
import { TaskBadge } from "./DiffView";
import { Icon } from "./Icon";
import type { Notice, Send } from "./socket";
import { deleteBlock, groupTasks, relativeTime, secondaryLine, untitled, type Group } from "./tasks-model";

/** The wall clock, ticking every `ms` (30 s: relative times move on). */
export function useNow(ms = 30_000): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), ms);
    return () => clearInterval(timer);
  }, [ms]);
  return now;
}

/** A task's title renamed in place: Enter or leaving the field saves, Escape does not. */
export function RenameInput({ task, send, className = "", onDone }: { task: Task; send: Send; className?: string; onDone: () => void }) {
  const cancel = useRef(false);
  return (
    <input
      className={`task-rename ${className}`}
      aria-label="Task title"
      defaultValue={task.title}
      maxLength={120}
      autoFocus
      onFocus={(e) => e.currentTarget.select()}
      onKeyDown={(e) => {
        if (e.key === "Escape") cancel.current = true;
        if (e.key === "Enter" || e.key === "Escape") e.currentTarget.blur();
      }}
      onBlur={(e) => {
        const next = e.currentTarget.value.split(/\s+/).filter(Boolean).join(" ");
        if (!cancel.current && next && next !== task.title) send({ type: "rename_task", task: task.id, title: next });
        onDone();
      }}
    />
  );
}

/**
 * The rail's Tasks pane (D12): the active project's tasks by group, a
 * search over titles, and per row rename and delete. A row is the chat's
 * current task when selected (core state); opening one also opens its
 * Task tab, Alt- or Cmd-click to the side.
 */
export function TasksPane({
  state,
  online,
  send,
  transcripts,
  fetchTranscript,
  notice,
  selected,
  onOpen,
  onNewTask,
}: {
  state: AppState | null;
  online: boolean;
  send: Send;
  transcripts: ReadonlyMap<number, Item[]>;
  fetchTranscript: (task: number) => void;
  notice: Notice | null;
  /** The chat's current task. */
  selected: number | null;
  onOpen: (task: number, side: boolean) => void;
  onNewTask: () => void;
}) {
  const now = useNow();
  const [query, setQuery] = useState("");
  const [doneOpen, setDoneOpen] = useState(false);
  const active = state?.active ?? null;
  const all = (state?.tasks ?? []).filter((t) => t.project === active);
  const groups = groupTasks(all, active, query);
  // A search shows its matches in Done too.
  const shown = (g: Group) => g !== "done" || doneOpen || query.trim() !== "";
  const rows = groups.flatMap((g) => (shown(g.group) ? g.tasks : []));
  // Shown rows' second lines come from their transcripts; each is fetched once per connection.
  useEffect(() => {
    if (online) for (const t of rows) fetchTranscript(t.id);
  });
  // One row is in the tab order (roving): the last focused, else the selected, else the first.
  const [roving, setRoving] = useState<number | null>(null);
  const tabStop = rows.some((t) => t.id === roving) ? roving : rows.some((t) => t.id === selected) ? selected : (rows[0]?.id ?? null);
  const list = useRef<HTMLDivElement>(null);
  const focusRow = (id: number) => {
    setRoving(id);
    list.current?.querySelector<HTMLElement>(`[data-task-row="${id}"]`)?.focus();
  };
  const onListKey = (e: KeyboardEvent) => {
    const at = rows.findIndex((t) => t.id === tabStop);
    const target = e.target as HTMLElement;
    if (!target.matches("[data-task-row]")) return;
    const to =
      e.key === "ArrowDown" ? at + 1
      : e.key === "ArrowUp" ? at - 1
      : e.key === "Home" ? 0
      : e.key === "End" ? rows.length - 1
      : null;
    if (to === null || to < 0 || to >= rows.length) return;
    e.preventDefault();
    focusRow(rows[to]!.id);
  };

  let empty: string | null = null;
  if (state && all.length === 0) empty = "No tasks yet. Ask the agent for a change and it is tracked here until you keep or revert it.";
  else if (groups.length === 0) empty = `No task titles match “${query.trim()}”.`;

  return (
    <aside className="pane tasks-pane" aria-label="Tasks">
      <div className="tasks-head">
        <h2 className="pane-title">Tasks</h2>
        <button type="button" className="button button-small" disabled={!state} onClick={onNewTask}>
          <Icon name="plus" />
          New task
        </button>
      </div>
      <input
        type="search"
        className="text-input tasks-search"
        aria-label="Search tasks"
        placeholder="Search tasks"
        value={query}
        disabled={!state || all.length === 0}
        onChange={(e) => setQuery(e.target.value)}
      />
      {empty && (
        <p className="hint" data-testid="tasks-empty">
          {empty}
        </p>
      )}
      <div className="tasks-list" ref={list} onKeyDown={onListKey}>
        {groups.map((g) => {
          const id = `tasks-group-${g.group}`;
          return (
            <section key={g.group} className="tasks-group" data-group={g.group}>
              {g.group === "done" ? (
                <button
                  type="button"
                  id={id}
                  className="tasks-group-toggle"
                  aria-expanded={shown("done")}
                  disabled={query.trim() !== ""}
                  onClick={() => setDoneOpen((o) => !o)}
                >
                  <span className="eyebrow">
                    {g.label} · {g.tasks.length}
                  </span>
                  <Icon name={shown("done") ? "chevron-up" : "chevron"} />
                </button>
              ) : (
                <h3 id={id} className="eyebrow tasks-group-label">
                  {g.label} · {g.tasks.length}
                </h3>
              )}
              {shown(g.group) && (
                <ul className="tasks-rows" aria-labelledby={id}>
                  {g.tasks.map((t) => (
                    <TaskRow
                      key={t.id}
                      task={t}
                      items={transcripts.get(t.id)}
                      now={now}
                      selected={t.id === selected}
                      tabStop={t.id === tabStop}
                      online={online}
                      tracks={state?.tracks_changes ?? false}
                      send={send}
                      notice={notice}
                      onFocus={() => setRoving(t.id)}
                      onOpen={onOpen}
                    />
                  ))}
                </ul>
              )}
            </section>
          );
        })}
      </div>
    </aside>
  );
}

function TaskRow({
  task,
  items,
  now,
  selected,
  tabStop,
  online,
  tracks,
  send,
  notice,
  onFocus,
  onOpen,
}: {
  task: Task;
  items: Item[] | undefined;
  now: number;
  selected: boolean;
  tabStop: boolean;
  online: boolean;
  tracks: boolean;
  send: Send;
  notice: Notice | null;
  onFocus: () => void;
  onOpen: (task: number, side: boolean) => void;
}) {
  const [renaming, setRenaming] = useState(false);
  const [confirming, setConfirming] = useState(false);
  // A refusal that arrives after this row's delete was sent shows under it.
  const [since, setSince] = useState(Infinity);
  const main = useRef<HTMLButtonElement>(null);
  const title = untitled(task);
  const block = deleteBlock(task, tracks);
  const second = secondaryLine(task, items);
  const when = relativeTime(task.updated_at, now);
  const refusal = notice && notice.seq > since ? notice.reason : null;
  const back = () => setTimeout(() => main.current?.focus());
  const open = (e: MouseEvent) => onOpen(task.id, e.altKey || e.metaKey);
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Enter" && (e.altKey || e.metaKey)) {
      e.preventDefault();
      onOpen(task.id, true);
    } else if (e.key === "F2") {
      e.preventDefault();
      setRenaming(true);
    } else if (e.key === "Delete" || (e.key === "Backspace" && e.metaKey)) {
      e.preventDefault();
      setConfirming(true);
    }
  };
  return (
    <li className={`task-row${selected ? " task-row-selected" : ""}`} data-testid="task-row">
      <div className="task-row-line">
        {renaming ? (
          <RenameInput
            task={task}
            send={send}
            onDone={() => {
              setRenaming(false);
              back();
            }}
          />
        ) : (
          <button
            ref={main}
            type="button"
            className="task-main"
            data-task-row={task.id}
            tabIndex={tabStop ? 0 : -1}
            aria-current={selected || undefined}
            title={`${title} (Alt-click: open to the side; F2: rename; Delete: delete)`}
            onFocus={onFocus}
            onClick={open}
            onKeyDown={onKey}
          >
            <span className="task-row-top">
              <span className="task-title">{title}</span>
              {when && (
                <time className="task-when" dateTime={new Date(task.updated_at!).toISOString()}>
                  {when}
                </time>
              )}
            </span>
            <span className="task-row-sub">
              <TaskBadge status={task.status} testid="task-row-status" />
              <span className="task-second">{second}</span>
            </span>
          </button>
        )}
        <span className="task-actions">
          <button
            type="button"
            className="icon-button"
            aria-label={`Rename ${title}`}
            title="Rename (F2)"
            tabIndex={tabStop ? 0 : -1}
            disabled={!online || renaming}
            onClick={() => setRenaming(true)}
          >
            <Icon name="compose" />
          </button>
          <button
            type="button"
            className="icon-button"
            aria-label={`Delete ${title}`}
            title="Delete (Delete)"
            tabIndex={tabStop ? 0 : -1}
            disabled={!online}
            onClick={() => setConfirming(true)}
          >
            <Icon name="trash" />
          </button>
        </span>
      </div>
      {confirming && (
        <div className="task-confirm" role="group" aria-label={`Delete ${title}`}>
          {block ? (
            <p>Cannot delete this task yet. {block}</p>
          ) : (
            <p>Delete this task? Its conversation is removed from the app; your files stay as they are.</p>
          )}
          <div className="review-buttons">
            {!block && (
              <button
                type="button"
                className="button button-small button-danger"
                disabled={!online}
                onClick={() => {
                  setSince(notice?.seq ?? 0);
                  setConfirming(false);
                  send({ type: "delete_task", task: task.id });
                }}
              >
                Delete
              </button>
            )}
            <button
              type="button"
              className="button button-small"
              autoFocus
              onClick={() => {
                setConfirming(false);
                back();
              }}
            >
              {block ? "OK" : "Cancel"}
            </button>
          </div>
        </div>
      )}
      {refusal && (
        <p className="reason" role="status">
          {refusal}
        </p>
      )}
    </li>
  );
}
