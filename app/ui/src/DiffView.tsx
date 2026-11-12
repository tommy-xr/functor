import { useEffect, useRef, useState } from "react";
import type { AppState, FileDiff, Task, TaskStatus } from "./generated/protocol";
import {
  collapse,
  hasChange,
  omittedText,
  plural,
  renderLines,
  totals,
} from "./diff-model";
import { STATUS_WORDS } from "./chat-model";
import type { DiffMsg, Notice, Send } from "./socket";

/** A file "View diff" asked to show; `seq` tells repeats apart. */
export type DiffTarget = { path: string | null; seq: number };

/** The current task's change: its files, each unified diff, and Keep / Revert. */
export function DiffView({
  state,
  task,
  diff,
  fetchDiff,
  send,
  online,
  notice,
  target,
}: {
  state: AppState | null;
  task: Task | null;
  diff: DiffMsg | undefined;
  fetchDiff: (task: number) => void;
  send: Send;
  online: boolean;
  notice: Notice | null;
  target: DiffTarget | null;
}) {
  const tracked = state?.tracks_changes ?? false;
  const change = hasChange(task) ? task.change : null;
  // A summary names the snapshot it measured: a diff of the same tree is current.
  const key = task && change ? `${task.id} ${change.tree}` : null;
  // Asked once each time the tab opens (the user may have edited by hand),
  // then again only for a snapshot the diff does not show yet.
  const asked = useRef<string | null | undefined>(undefined);
  useEffect(() => {
    if (!task || !change || !online || !tracked || asked.current === key) return;
    const opened = asked.current === undefined;
    asked.current = key;
    if (!opened && diff?.task === task.id && diff.tree === change.tree) return;
    fetchDiff(task.id);
  }, [key, online]);

  const current = diff && task && diff.task === task.id ? diff : undefined;
  // Scrolled to the file "View diff" named, once its diff is here.
  useEffect(() => {
    if (!target?.path || !current) return;
    const i = current.files.findIndex((f) => f.file.path === target.path);
    document.getElementById(`diff-file-${i}`)?.scrollIntoView({ block: "start" });
  }, [target?.seq, current]);

  let empty: string | null = null;
  if (!tracked) empty = "This folder is not a git repository, so changes are not tracked.";
  else if (!task) empty = "Pick a task to review its change.";
  else if (task.status.kind === "kept") empty = "The change was kept. Nothing left to review.";
  else if (task.status.kind === "reverted") empty = "The change was reverted. Nothing left to review.";
  else if (!change) empty = task.run ? "No changes yet. The agent is still working." : "No changes to review.";
  if (empty || !task || !change) {
    return (
      <div className="placeholder" data-testid="diff-empty">
        <p>{empty}</p>
      </div>
    );
  }
  const sum = totals(change);
  return (
    <div className="diff" data-testid="diff-view">
      <header className="diff-head">
        <h2 className="eyebrow">Change</h2>
        <span className="diff-total">
          {plural(sum.files, "file")} <Counts added={sum.added} removed={sum.removed} />
        </span>
        <TaskBadge status={task.status} testid="diff-status" />
        <span className="spacer" />
        <ReviewActions task={task} send={send} online={online} notice={notice} />
      </header>
      <nav className="diff-index" aria-label="Changed files">
        {change.files.map((f, i) => (
          <button
            key={f.path}
            type="button"
            className="diff-index-item"
            onClick={() => document.getElementById(`diff-file-${i}`)?.scrollIntoView({ block: "start" })}
          >
            <span className="mono">{f.path}</span>
            <FileCounts file={f} />
          </button>
        ))}
        {change.more > 0 && <span className="hint">and {plural(change.more, "more file")}</span>}
      </nav>
      {current?.error && <p className="reason">The diff could not be read: {current.error}</p>}
      {!current && <p className="hint">Loading the diff…</p>}
      {current?.files.map((f, i) => (
        <FileBlock key={f.file.path} id={`diff-file-${i}`} diff={f} current={f.file.path === target?.path} />
      ))}
    </div>
  );
}

function FileBlock({ id, diff, current }: { id: string; diff: FileDiff; current: boolean }) {
  const [expanded, setExpanded] = useState(false);
  const { file, lines, omitted } = diff;
  const { shown, hidden } = collapse(lines, expanded);
  return (
    <section className="diff-file" id={id} aria-label={file.path} data-current={current || undefined}>
      <header className="diff-file-head">
        <span className="mono diff-path">{file.from ? `${file.from} → ${file.path}` : file.path}</span>
        <FileCounts file={file} />
      </header>
      {omitted ? (
        <p className="diff-omitted">{omittedText(omitted, file)}</p>
      ) : lines.length === 0 ? (
        <p className="diff-omitted">No line changes (renamed or its mode changed).</p>
      ) : (
        <div className="diff-lines">
          <div className="diff-lines-inner">{renderLines(shown)}</div>
        </div>
      )}
      {hidden > 0 && (
        <button type="button" className="link-button diff-more" onClick={() => setExpanded(true)}>
          Show {plural(hidden, "more line")}
        </button>
      )}
    </section>
  );
}

export function TaskBadge({ status, testid = "task-status" }: { status: TaskStatus; testid?: string }) {
  return (
    <span className={`badge badge-${status.kind}`} data-testid={testid}>
      {STATUS_WORDS[status.kind]}
    </span>
  );
}

export function Counts({ added, removed }: { added: number; removed: number }) {
  return (
    <span className="counts mono">
      <span className="count-add">+{added}</span> <span className="count-del">−{removed}</span>
    </span>
  );
}

function FileCounts({ file }: { file: FileDiff["file"] }) {
  return file.binary ? <span className="counts mono">binary</span> : <Counts added={file.added} removed={file.removed} />;
}

/**
 * Keep change / Revert for `task`, with Revert confirmed in place. A refusal
 * that arrives after a click shows under the buttons.
 */
export function ReviewActions({
  task,
  send,
  online,
  notice,
  onViewDiff,
}: {
  task: Task;
  send: Send;
  online: boolean;
  notice: Notice | null;
  onViewDiff?: () => void;
}) {
  const [confirming, setConfirming] = useState(false);
  const [since, setSince] = useState(Infinity);
  const files = hasChange(task) ? totals(task.change).files : 0;
  const busy = !online || task.reverting || task.run !== null;
  const act = (type: "keep_change" | "revert_change") => {
    setSince(notice?.seq ?? 0);
    setConfirming(false);
    send({ type, task: task.id });
  };
  const refusal = notice && notice.seq > since ? notice.reason : null;
  if (confirming) {
    return (
      <div className="review review-confirm" role="group" aria-label="Confirm revert">
        <p>
          Revert puts the {plural(files, "file")} this task changed back as {files === 1 ? "it was" : "they were"}{" "}
          before it started, undoing any other edits made to {files === 1 ? "it" : "them"} since.
        </p>
        <div className="review-buttons">
          <button type="button" className="button button-danger" disabled={busy} onClick={() => act("revert_change")}>
            Revert {plural(files, "file")}
          </button>
          <button type="button" className="button" onClick={() => setConfirming(false)}>
            Cancel
          </button>
        </div>
      </div>
    );
  }
  return (
    <div className="review">
      <div className="review-buttons">
        <button type="button" className="button button-primary" disabled={busy} onClick={() => act("keep_change")}>
          Keep change
        </button>
        <button
          type="button"
          className="button"
          disabled={busy}
          onClick={() => {
            // Measured afresh, so the confirmation counts what is there now.
            send({ type: "get_diff", task: task.id });
            setConfirming(true);
          }}
        >
          {task.reverting ? "Reverting…" : "Revert"}
        </button>
        {onViewDiff && (
          <button type="button" className="button" onClick={onViewDiff}>
            View diff
          </button>
        )}
      </div>
      {refusal && (
        <p className="reason" role="status">
          {refusal}
        </p>
      )}
    </div>
  );
}
