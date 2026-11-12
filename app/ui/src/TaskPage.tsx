import { useEffect, useRef, useState, type ReactNode } from "react";
import type { AppState, Item, Task } from "./generated/protocol";
import { stepAvailable, tokens, viewPhrase } from "./chat-model";
import { Counts, ReviewActions, TaskBadge } from "./DiffView";
import { hasChange, plural, totals } from "./diff-model";
import { Icon } from "./Icon";
import type { Notice, Send } from "./socket";
import { taskPage, usageTable, type RunSteps } from "./task-page-model";
import { relativeTime } from "./tasks-model";
import { untitled, useNow } from "./TasksPane";

/**
 * A task's page (the `Task` tab): a read view of what the agent was given,
 * its steps, its change (Keep / Revert as in the Changes tab), its
 * walkthrough and its usage, assembled from the transcript.
 */
export function TaskPage({
  state,
  task,
  items,
  fetchTranscript,
  online,
  send,
  notice,
  onContinue,
  onOpenChanges,
}: {
  state: AppState | null;
  task: Task | null;
  items: Item[] | undefined;
  fetchTranscript: (task: number) => void;
  online: boolean;
  send: Send;
  notice: Notice | null;
  onContinue: () => void;
  onOpenChanges: () => void;
}) {
  const now = useNow();
  useEffect(() => {
    if (task && online) fetchTranscript(task.id);
  });
  if (!task) {
    return (
      <div className="placeholder">
        <p>This task is gone.</p>
      </div>
    );
  }
  const page = items ? taskPage(items, task.run?.id ?? null) : null;
  const usage = items ? usageTable(items) : null;
  const id = (part: string) => `task-${task.id}-${part}`;
  const created = relativeTime(task.created_at, now);
  const active = relativeTime(task.updated_at, now);
  return (
    <article className="task-page" data-testid="task-page" aria-labelledby={id("title")}>
      <header className="task-page-head">
        <div className="task-page-title">
          <Title task={task} id={id("title")} online={online} send={send} />
          <TaskBadge status={task.status} testid="task-page-status" />
        </div>
        <p className="task-meta">
          <span className="mono">{task.model ? `${task.harness} · ${task.model}` : task.harness}</span>
          {created && <span>Started {created}</span>}
          {active && <span>Last active {active}</span>}
        </p>
        <div className="review-buttons">
          <button type="button" className="button button-small" disabled={!state} onClick={onContinue}>
            Continue in chat
          </button>
        </div>
      </header>
      {!page || !usage ? (
        <p className="hint">Loading…</p>
      ) : (
        <>
          <Section id={id("given")} title="What the agent was given">
            {page.given === null ? <p className="hint">No message yet.</p> : <p className="task-given">{page.given}</p>}
            {page.followUps.length > 0 && (
              <>
                <h4 className="task-sub">Then you said</h4>
                <ol className="task-followups">
                  {page.followUps.map((text, i) => (
                    <li key={i}>{text}</li>
                  ))}
                </ol>
              </>
            )}
          </Section>
          <Section id={id("steps")} title="Steps">
            <Steps runs={page.runs} />
          </Section>
          <Section id={id("change")} title="Change">
            <ChangeSection task={task} tracks={state?.tracks_changes ?? false} now={now} {...{ online, send, notice, onOpenChanges }} />
          </Section>
          {page.walkthrough && (
            <Section id={id("walk")} title="Walkthrough">
              <ol className="walk-steps">
                {page.walkthrough.steps.map((s, i) => {
                  const walk = page.walkthrough!;
                  const available = stepAvailable(s.view, state?.files ?? [], !!state?.tracks_changes && hasChange(task));
                  const where = viewPhrase(s.view);
                  return (
                    <li key={i}>
                      <button
                        type="button"
                        className="walk-step"
                        aria-current={i === walk.current ? "step" : undefined}
                        disabled={!online || !available}
                        onClick={() => send({ type: "walkthrough_step", task: task.id, item: walk.item, step: i })}
                      >
                        <span className="walk-n" aria-hidden="true">
                          {i + 1}
                        </span>
                        <span className="walk-body">
                          <span className="walk-title">{s.title}</span>
                          {s.note && <span className="walk-note">{s.note}</span>}
                          <span className="walk-where">
                            {!available ? "No longer available" : where.code ? <><code>{where.code}</code>{where.after}</> : where.after?.replace(/^ /, "")}
                          </span>
                        </span>
                      </button>
                    </li>
                  );
                })}
              </ol>
            </Section>
          )}
          <Section id={id("usage")} title="Usage">
            {usage.rows.length === 0 ? (
              <p className="hint">No runs yet.</p>
            ) : (
              <table className="task-usage" data-testid="task-usage">
                <thead>
                  <tr>
                    <th scope="col">Run</th>
                    <th scope="col">Tokens in</th>
                    <th scope="col">Tokens out</th>
                  </tr>
                </thead>
                <tbody>
                  {usage.rows.map((r) => (
                    <tr key={r.run}>
                      <th scope="row">{r.run}</th>
                      <td>{r.input === null ? "—" : tokens(r.input)}</td>
                      <td>{r.output === null ? "—" : tokens(r.output)}</td>
                    </tr>
                  ))}
                </tbody>
                <tfoot>
                  <tr>
                    <th scope="row">{plural(usage.total.turns, "turn")}</th>
                    <td>{tokens(usage.total.input)}</td>
                    <td>{tokens(usage.total.output)}</td>
                  </tr>
                </tfoot>
              </table>
            )}
          </Section>
        </>
      )}
    </article>
  );
}

function Section({ id, title, children }: { id: string; title: string; children: ReactNode }) {
  return (
    <section className="task-section" aria-labelledby={id}>
      <h3 id={id} className="eyebrow">
        {title}
      </h3>
      {children}
    </section>
  );
}

/** The title, renamed in place: Enter or leaving the field saves, Escape does not. */
function Title({ task, id, online, send }: { task: Task; id: string; online: boolean; send: Send }) {
  const [editing, setEditing] = useState(false);
  const cancel = useRef(false);
  const button = useRef<HTMLButtonElement>(null);
  const commit = (value: string) => {
    setEditing(false);
    const next = value.split(/\s+/).filter(Boolean).join(" ");
    if (!cancel.current && next && next !== task.title) send({ type: "rename_task", task: task.id, title: next });
    cancel.current = false;
    setTimeout(() => button.current?.focus());
  };
  if (editing) {
    return (
      <input
        className="task-rename task-page-rename"
        aria-label="Task title"
        defaultValue={task.title}
        maxLength={120}
        autoFocus
        onFocus={(e) => e.currentTarget.select()}
        onKeyDown={(e) => {
          if (e.key === "Escape") cancel.current = true;
          if (e.key === "Enter" || e.key === "Escape") e.currentTarget.blur();
        }}
        onBlur={(e) => commit(e.currentTarget.value)}
      />
    );
  }
  return (
    <>
      <h2 id={id} className="task-page-name">
        {untitled(task)}
      </h2>
      <button
        ref={button}
        type="button"
        className="icon-button"
        aria-label="Rename task"
        title="Rename"
        disabled={!online}
        onClick={() => setEditing(true)}
      >
        <Icon name="compose" />
      </button>
    </>
  );
}

/** The last run's steps; earlier runs behind "Show earlier runs". */
function Steps({ runs }: { runs: RunSteps[] }) {
  const [all, setAll] = useState(false);
  if (runs.length === 0) return <p className="hint">No runs yet.</p>;
  const shown = all ? runs : runs.slice(-1);
  return (
    <div className="task-runs">
      {runs.length > 1 && (
        <button type="button" className="link-button" aria-expanded={all} onClick={() => setAll((a) => !a)}>
          {all ? "Show the last run only" : `Show earlier runs (${runs.length - 1})`}
        </button>
      )}
      {shown.map((run) => (
        <div key={run.n} className="task-run" data-testid="task-run">
          <p className="task-run-head">
            <span>Run {run.n}</span>
            {run.outcome ? (
              <span className={`result result-${run.outcome.tone}`}>{run.outcome.label}</span>
            ) : (
              <span className="result result-neutral">In progress</span>
            )}
          </p>
          {run.steps.length === 0 ? (
            <p className="hint">No tool calls.</p>
          ) : (
            <ol className="task-steps">
              {run.steps.map((s, i) => (
                <li key={i} className={`task-step task-step-${s.state}`} data-testid="task-step">
                  <span className={s.state === "pending" ? "step-dot step-dot-busy" : "step-dot"} aria-hidden="true" />
                  <span className="step-text">
                    {s.phrase.text}
                    {s.phrase.code && <code>{s.phrase.code}</code>}
                    {s.phrase.after}
                  </span>
                  {s.state === "failed" && <span className="task-step-flag">failed</span>}
                  {s.state === "unanswered" && <span className="task-step-flag">no result</span>}
                </li>
              ))}
            </ol>
          )}
        </div>
      ))}
    </div>
  );
}

/** The change: its files and counts, Open changes, and Keep / Revert while it waits for review. */
function ChangeSection({
  task,
  tracks,
  now,
  online,
  send,
  notice,
  onOpenChanges,
}: {
  task: Task;
  tracks: boolean;
  now: number;
  online: boolean;
  send: Send;
  notice: Notice | null;
  onOpenChanges: () => void;
}) {
  const when = relativeTime(task.updated_at, now);
  if (task.status.kind === "kept" || task.status.kind === "reverted") {
    const what = task.status.kind === "kept" ? "The change was kept" : "The change was reverted";
    return <p data-testid="task-change-closed">{when ? `${what} ${when}.` : `${what}.`}</p>;
  }
  if (!tracks) return <p className="hint">This folder is not a git repository, so changes are not tracked.</p>;
  if (!hasChange(task)) return <p className="hint">{task.run ? "No change yet." : "No change."}</p>;
  const sum = totals(task.change);
  return (
    <div className="task-change" data-testid="task-change">
      <p>
        {plural(sum.files, "file")} <Counts added={sum.added} removed={sum.removed} />
      </p>
      <ul className="task-change-files">
        {task.change.files.map((f) => (
          <li key={f.path}>
            <span className="mono">{f.path}</span>
            {f.binary ? <span className="counts mono">binary</span> : <Counts added={f.added} removed={f.removed} />}
          </li>
        ))}
        {task.change.more > 0 && <li className="hint">and {plural(task.change.more, "more file")}</li>}
      </ul>
      <div className="task-change-actions">
        <button type="button" className="button" onClick={onOpenChanges}>
          Open changes
        </button>
        {task.status.kind === "ready_for_review" && <ReviewActions task={task} send={send} online={online} notice={notice} />}
      </div>
    </div>
  );
}
