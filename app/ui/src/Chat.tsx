import {
  Fragment,
  memo,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import type { AppState, Change, Decision, Harness, Item, Task, Usage } from "./generated/protocol";
import {
  atBottom,
  clip,
  connectionLines,
  outcome,
  pickHarness,
  toolFile,
  phraseText,
  STATUS_WORDS,
  stepAvailable,
  stepPhrase,
  toolKind,
  usageBefore,
  usageText,
  viewPhrase,
  walkTo,
  type Snapshot,
} from "./chat-model";
import { AgentCard } from "./AgentCard";
import { dockTask } from "./agent-model";
import { DEPTH_WORDS, kindsLine } from "./decision-model";
import { Counts, ReviewActions, TaskBadge } from "./DiffView";
import { PathCard } from "./PathEvidence";
import { fileFor, hasChange, plural, totals } from "./diff-model";
import { renderMarkdown } from "./markdown";
import type { Accepted, Conn, Notice, Send } from "./socket";
import { Icon } from "./Icon";
import { useMenu } from "./menu";


const RECENT = 8;
/** The composer's textarea grows up to this, then scrolls. */
const MAX_INPUT_PX = 160;

type Props = {
  state: AppState | null;
  conn: Conn;
  send: Send;
  transcripts: ReadonlyMap<number, Item[]>;
  fetchTranscript: (task: number) => void;
  accepted: Accepted | null;
  notice: Notice | null;
  /** The canvas's focused tab, for the composer's context chip. */
  canvasWord: string;
  /** Opens or focuses the Game tab, beside what is shown. */
  onOpenGame: () => void;
  /** The current task (the core's); `null` is the empty state. */
  selected: number | null;
  onSelect: (task: number | null) => void;
  /** Opens or focuses the current task's Changes tab at `path` (repo-relative), or at its top. */
  onViewDiff: (path: string | null) => void;
  /** Opens a task (an agent card's click). */
  onOpenTask: (task: number) => void;
};

const NO_FILES: string[] = [];

/** What a walkthrough card needs: which steps can be shown, and a way to show one. */
type Walk = {
  files: readonly string[];
  /** The task still has a change to show. */
  changes: boolean;
  online: boolean;
  onStep: ((item: number, step: number) => void) | null;
};

/** A line the dock shows for a connection or session change, at the conversation's length when it happened. */
type Line = { id: number; project: string | null; task: number | null; at: number; text: string };

/** The chat dock: one task's conversation (the current task) and the composer. */
export function ChatDock({
  state,
  conn,
  send,
  transcripts,
  fetchTranscript,
  accepted,
  notice,
  canvasWord,
  onOpenGame,
  selected,
  onSelect: setSelected,
  onViewDiff,
  onOpenTask,
}: Props) {
  const online = conn === "open" && state !== null;
  const active = state?.active ?? null;
  const tasks = useMemo(
    () => (state?.tasks ?? []).filter((t) => t.project === active).sort((a, b) => b.id - a.id),
    [state?.tasks, active],
  );
  const current = tasks.find((t) => t.id === selected) ?? null;
  const items = current ? transcripts.get(current.id) : undefined;
  // Once per task per connection: `fetchTranscript` skips one already asked for.
  useEffect(() => {
    if (current && online) fetchTranscript(current.id);
  });

  // Sent and not yet answered: kept in the composer until its item arrives.
  const [draft, setDraft] = useState("");
  const [pending, setPending] = useState<{ task: number | null; text: string } | null>(null);
  const [shownNotice, setShownNotice] = useState<string | null>(null);
  useEffect(() => {
    if (!pending || !accepted || accepted.text !== pending.text) return;
    // A new task becomes the current one in the core as it starts.
    if (pending.task !== null && pending.task !== accepted.task) return;
    setPending(null);
    setDraft((d) => (d === pending.text ? "" : d));
    setShownNotice(null);
    // Only a new acceptance settles a send.
  }, [accepted]);
  // A refusal while a send is pending answers it; others (a stop racing the run's end) are not news.
  useEffect(() => {
    if (!notice || !pending) return;
    setPending(null);
    setShownNotice(notice.reason);
  }, [notice]);
  // A dropped connection forgets the send; the text stays.
  useEffect(() => {
    if (conn !== "open") setPending(null);
  }, [conn]);

  const lines = useConnectionLines(state, conn, current?.id ?? null, items?.length);
  // Stable while nothing it holds changes: every row is memoised on it.
  const files = state?.files ?? NO_FILES;
  const changes = !!current && !!state?.tracks_changes && hasChange(current);
  const taskId = current?.id ?? null;
  const walk = useMemo<Walk>(
    () => ({
      files,
      changes,
      online,
      onStep: taskId === null ? null : (item, step) => send({ type: "walkthrough_step", task: taskId, item, step }),
    }),
    [files, changes, online, taskId, send],
  );
  const agent = dockTask(state, current?.id ?? null);
  const harness: Harness = current?.harness ?? pickHarness(state);
  const running = current?.run != null;

  const submit = () => {
    const text = draft;
    if (!text.trim() || pending || !online) return;
    const sent = current
      ? send({ type: "send_message", task: current.id, message: text })
      : send({ type: "new_task", message: text, harness });
    if (sent) setPending({ task: current?.id ?? null, text });
  };
  const stop = () => current && send({ type: "stop_task", task: current.id });
  const select = (task: number | null) => {
    setSelected(task);
    setShownNotice(null);
  };

  return (
    <section className="chat" aria-labelledby="chat-heading">
      <ChatHead tasks={tasks} current={current} onSelect={select} />
      {agent && (
        <div className="chat-agent">
          <AgentCard task={agent} activity={state!.activity[agent.id]!} onOpen={onOpenTask} />
        </div>
      )}
      <Conversation
        key={current?.id ?? "new"}
        task={current}
        items={items}
        lines={lines.filter((l) => l.project === active && l.task === (current?.id ?? null))}
        onOpenGame={onOpenGame}
        onViewDiff={onViewDiff}
        walk={walk}
        repoRoot={state?.repo_root ?? ""}
        project={current?.project ?? ""}
        review={
          current &&
          current.run === null &&
          hasChange(current) && (
            <ReviewCard
              task={current}
              decision={state?.decisions[current.id]}
              send={send}
              online={online}
              notice={notice}
              onViewDiff={onViewDiff}
              onOpenTask={onOpenTask}
            />
          )
        }
      />
      <div className="composer-wrap">
        {shownNotice && (
          <p className="notice" role="status" data-testid="chat-notice">
            {shownNotice}
          </p>
        )}
        <Composer
          state={state}
          online={online}
          draft={draft}
          onDraft={(d) => {
            setDraft(d);
            setShownNotice(null);
          }}
          harness={harness}
          // A task keeps the agent it started with.
          pickable={current === null}
          onHarness={(h) => send({ type: "set_harness", harness: h })}
          running={running}
          busy={pending !== null}
          onSubmit={submit}
          onStop={stop}
          canvasWord={canvasWord}
          placeholder={current ? "Reply in this task" : "Ask for a change, or about the running game"}
        />
      </div>
    </section>
  );
}

/** Lines from connection and session changes, each anchored where the current conversation stood. */
function useConnectionLines(state: AppState | null, conn: Conn, task: number | null, at: number | undefined): Line[] {
  const [lines, setLines] = useState<Line[]>([]);
  const prev = useRef<Snapshot>({ conn, active: undefined, session: null });
  const ids = useRef(0);
  // While a transcript is (re)fetched its length is unknown: keep the last one.
  const project = state?.active ?? null;
  const anchor = useRef({ project, task, at: at ?? 0 });
  if (at !== undefined || anchor.current.task !== task) anchor.current = { project, task, at: at ?? 0 };
  anchor.current.project = project;
  const next: Snapshot = { conn, active: state ? state.active : undefined, session: state?.session?.status ?? null };
  useEffect(() => {
    const changed = connectionLines(prev.current, next);
    prev.current = next;
    if (!changed.length) return;
    setLines((all) => [...all.slice(-20), ...changed.map((text) => ({ id: ++ids.current, text, ...anchor.current }))]);
  }, [next.conn, next.active, next.session?.kind]);
  return lines;
}

function ChatHead({
  tasks,
  current,
  onSelect,
}: {
  tasks: Task[];
  current: Task | null;
  onSelect: (task: number | null) => void;
}) {
  const { open, setOpen, ref: menu, onKeyDown } = useMenu();
  return (
    <header className="chat-head">
      <div className="chat-title">
        <h2 id="chat-heading" title={current?.title}>
          {current ? current.title || "Untitled task" : "New task"}
        </h2>
        {current && <TaskBadge status={current.status} />}
      </div>
      <div className="chat-menu" ref={menu} onKeyDown={onKeyDown}>
        <button
          type="button"
          className="icon-button"
          aria-label="Recent tasks"
          title="Recent tasks"
          aria-haspopup="menu"
          aria-expanded={open}
          disabled={tasks.length === 0}
          onClick={() => setOpen((o) => !o)}
        >
          <Icon name="history" />
        </button>
        {open && (
          <div className="menu" role="menu" aria-label="Recent tasks">
            {tasks.slice(0, RECENT).map((t) => (
              <button
                key={t.id}
                type="button"
                role="menuitem"
                aria-current={t.id === current?.id || undefined}
                onClick={() => {
                  setOpen(false);
                  onSelect(t.id);
                }}
              >
                <span className="menu-title">{t.title || "Untitled task"}</span>
                <span className="menu-status">{STATUS_WORDS[t.status.kind]}</span>
              </button>
            ))}
          </div>
        )}
      </div>
      <button
        type="button"
        className="icon-button"
        aria-label="New task"
        title="New task"
        disabled={current === null}
        onClick={() => onSelect(null)}
      >
        <Icon name="compose" />
      </button>
    </header>
  );
}

/**
 * The end of a conversation whose change waits: the decision page in
 * small (D35) — its size, how deep to look and why, the predicted path —
 * with Keep / Revert, View diff and the way to the page.
 */
function ReviewCard({
  task,
  decision,
  send,
  online,
  notice,
  onViewDiff,
  onOpenTask,
}: {
  task: Task & { change: Change };
  decision: Decision | undefined;
  send: Send;
  online: boolean;
  notice: Notice | null;
  onViewDiff: (path: string | null) => void;
  onOpenTask: (task: number) => void;
}) {
  const sum = totals(task.change);
  return (
    <div className="chat-card review-card" data-testid="review-card">
      <p className="review-card-title">
        Review change · {plural(sum.files, "file")} <Counts added={sum.added} removed={sum.removed} />
        {decision && decision.size.kinds.length > 0 && ` · ${kindsLine(decision.size)}`}
      </p>
      {decision && (
        <p className={`review-verdict verdict verdict-${decision.depth}`} data-testid="review-verdict">
          <strong>{DEPTH_WORDS[decision.depth]}</strong> {decision.reason}
        </p>
      )}
      <PathCard task={task} />
      <ReviewActions task={task} send={send} online={online} notice={notice} onViewDiff={() => onViewDiff(null)} />
      <button
        type="button"
        className="link-button review-card-open"
        // Clicking keeps the keyboard focus where it was (the composer).
        onMouseDown={(e) => e.preventDefault()}
        onClick={() => onOpenTask(task.id)}
      >
        Open the decision page
      </button>
    </div>
  );
}

/** The transcript: follows the bottom while the reader is there; "Jump to latest" once they scroll up. */
function Conversation({
  task,
  items,
  lines,
  onOpenGame,
  onViewDiff,
  walk,
  repoRoot,
  project,
  review,
}: {
  task: Task | null;
  items: Item[] | undefined;
  lines: Line[];
  onOpenGame: () => void;
  onViewDiff: (path: string | null) => void;
  walk: Walk;
  repoRoot: string;
  /** The task's project, which relative tool paths are in. */
  project: string;
  /** Shown after the conversation: the change waiting for review. */
  review: ReactNode;
}) {
  const log = useRef<HTMLDivElement>(null);
  const [follow, setFollow] = useState(true);
  useLayoutEffect(() => {
    const el = log.current;
    if (el && follow) el.scrollTop = el.scrollHeight;
  }, [items, lines, follow, !!review]);
  const runFrom = task?.run?.id ?? Infinity;
  // `Infinity` is every line past the transcript's end.
  const linesAt = (at: number) =>
    lines
      .filter((l) => (at === Infinity ? l.at > (items?.length ?? 0) : l.at === at))
      .map((l) => (
        <p key={l.id} className="conn-line">
          {l.text}
        </p>
      ));
  let body: ReactNode;
  if (task && !items) {
    body = <p className="chat-empty">Loading…</p>;
  } else {
    body = (items ?? []).map((item, i) => (
      <Fragment key={i}>
        <Row
          item={item}
          index={i}
          walk={walk}
          usage={usageBefore(items!, i)}
          pending={i > runFrom}
          onOpenGame={onOpenGame}
          onViewDiff={onViewDiff}
          change={task?.change ?? null}
          repoRoot={repoRoot}
          project={project}
        />
        {linesAt(i + 1)}
      </Fragment>
    ));
  }
  const n = items?.length ?? 0;
  return (
    <div className="chat-scroll">
      <div
        ref={log}
        className="chat-log"
        role="log"
        aria-live="polite"
        aria-relevant="additions"
        aria-label="Conversation"
        tabIndex={0}
        onScroll={(e) => setFollow(atBottom(e.currentTarget))}
      >
        {!task && <p className="chat-empty">Ask the agent for a change, or about the game. It sees the live game.</p>}
        {linesAt(0)}
        {body}
        {/* Lines from before the transcript arrived, or past its end. */}
        {lines.some((l) => l.at > n) && linesAt(Infinity)}
        {items && review}
      </div>
      {!follow && (
        <button
          type="button"
          className="jump"
          onClick={() => {
            const el = log.current;
            if (el) el.scrollTop = el.scrollHeight;
            setFollow(true);
          }}
        >
          Jump to latest
        </button>
      )}
    </div>
  );
}

/** One transcript item. Memoised: an `Item` update replaces only its own object, so a streamed reply re-renders alone. */
const Row = memo(function Row({
  item,
  index,
  walk,
  usage,
  pending,
  onOpenGame,
  onViewDiff,
  change,
  repoRoot,
  project,
}: {
  item: Item;
  index: number;
  walk: Walk;
  usage: Usage | null;
  pending: boolean;
  onOpenGame: () => void;
  onViewDiff: (path: string | null) => void;
  change: Change | null;
  repoRoot: string;
  /** The task's project, which relative tool paths are in. */
  project: string;
}) {
  switch (item.kind) {
    case "user":
      return <div className="msg msg-user">{item.text}</div>;
    case "agent":
      return <AgentText text={item.text} partial={item.partial} />;
    case "tool":
      return (
        <ToolCall
          item={item}
          pending={pending && item.result === null}
          onOpenGame={onOpenGame}
          onViewDiff={onViewDiff}
          change={change}
          repoRoot={repoRoot}
          project={project}
        />
      );
    case "usage":
      return null;
    case "walkthrough":
      return <WalkthroughCard item={item} index={index} walk={walk} />;
    case "finished":
    case "stopped":
    case "kept":
    case "reverted": {
      const { tone, label } = outcome(item)!;
      return (
        <div className="outcome">
          <span className={`result result-${tone}`} data-testid="result">
            {label}
          </span>
          {usage && <span className="usage">{usageText(usage)}</span>}
        </div>
      );
    }
  }
});

function AgentText({ text, partial }: { text: string; partial: boolean }) {
  const body = useMemo(() => renderMarkdown(text), [text]);
  // Busy while streaming, so a screen reader reads the reply once it is whole.
  return (
    <div className="msg msg-agent md" aria-busy={partial}>
      {body}
    </div>
  );
}

type ToolItem = Extract<Item, { kind: "tool" }>;

function ToolCall({
  item,
  pending,
  onOpenGame,
  onViewDiff,
  change,
  repoRoot,
  project,
}: {
  item: ToolItem;
  pending: boolean;
  onOpenGame: () => void;
  onViewDiff: (path: string | null) => void;
  change: Change | null;
  repoRoot: string;
  /** The task's project, which relative tool paths are in. */
  project: string;
}) {
  const phrase = stepPhrase(item.name, item.input);
  const kind = toolKind(item.name);
  // The change's counts for the file this call edited, while that file is in it.
  const path = toolFile(item.input);
  const file = kind === "change" && path ? fileFor(change, repoRoot, project, path) : null;
  const [open, setOpen] = useState(false);
  const line = (
    <button
      type="button"
      className="step-line"
      aria-expanded={open}
      aria-label={`${phraseText(phrase)}${pending ? ", in progress" : ""}: details`}
      onClick={() => setOpen((o) => !o)}
    >
      <span className={pending ? "step-dot step-dot-busy" : "step-dot"} aria-hidden="true" />
      <span className="step-text">
        {phrase.text}
        {phrase.code && <code>{phrase.code}</code>}
        {phrase.after}
        {pending && <span className="step-busy">…</span>}
        {file && !file.binary && <Counts added={file.added} removed={file.removed} />}
      </span>
      <Icon name={open ? "chevron-up" : "chevron"} />
    </button>
  );
  const details = open && <StepDetails item={item} pending={pending} />;
  if (kind === "step") {
    return (
      <div className="step" data-testid="step" aria-busy={pending}>
        {line}
        {details}
      </div>
    );
  }
  const isChange = kind === "change";
  return (
    <div className="chat-card" data-testid={isChange ? "change-card" : "capture-card"} aria-busy={pending}>
      {line}
      {!isChange && <p className="chat-card-note">The capture went to the agent; the canvas shows the game live.</p>}
      <div className="chat-card-actions">
        <button
          type="button"
          className="button button-small"
          // Clicking keeps the keyboard focus where it was (the composer).
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => (isChange ? onViewDiff(file?.path ?? null) : onOpenGame())}
        >
          {isChange ? "View diff" : "Open in canvas"}
        </button>
      </div>
      {details}
    </div>
  );
}

type WalkthroughItem = Extract<Item, { kind: "walkthrough" }>;

/**
 * The agent's walkthrough (`present`): its steps, the current one marked,
 * Prev / Next. A click shows the step's view in the canvas; the composer
 * keeps the keyboard focus. A step whose file or change is gone is shown as
 * unavailable.
 */
function WalkthroughCard({ item, index, walk }: { item: WalkthroughItem; index: number; walk: Walk }) {
  const { steps, current } = item;
  const available = steps.map((s) => stepAvailable(s.view, walk.files, walk.changes));
  const can = walk.online && walk.onStep !== null;
  const go = (step: number | null) => step !== null && walk.onStep?.(index, step);
  const prev = walkTo(available, current, -1);
  const next = walkTo(available, current, 1);
  const keep = (e: { preventDefault: () => void }) => e.preventDefault();
  return (
    <div className="chat-card walkthrough" data-testid="walkthrough" role="group" aria-label={`Walkthrough: ${steps[0]?.title ?? ""}`}>
      <p className="walkthrough-title">
        <span className="eyebrow">Walkthrough</span>
        <span>{steps[0]?.title}</span>
      </p>
      <WalkSteps steps={steps} current={current} available={available} enabled={can} onStep={go} />
      <div className="chat-card-actions walk-nav">
        <button type="button" className="button button-small" disabled={!can || prev === null} onMouseDown={keep} onClick={() => go(prev)}>
          Prev
        </button>
        <span className="walk-count">{current === null ? `${steps.length} steps` : `Step ${current + 1} of ${steps.length}`}</span>
        <button type="button" className="button button-small" disabled={!can || next === null} onMouseDown={keep} onClick={() => go(next)}>
          {current === null ? "Start" : "Next"}
        </button>
      </div>
    </div>
  );
}

/**
 * A walkthrough's steps, each a button showing its view (the chat's card and
 * the task page). A click keeps the keyboard focus where it was.
 */
export function WalkSteps({
  steps,
  current,
  available,
  enabled,
  onStep,
}: {
  steps: WalkthroughItem["steps"];
  current: number | null;
  available: readonly boolean[];
  enabled: boolean;
  onStep: (step: number) => void;
}) {
  return (
    <ol className="walk-steps">
      {steps.map((s, i) => {
        const where = viewPhrase(s.view);
        return (
          <li key={i}>
            <button
              type="button"
              className="walk-step"
              aria-current={i === current ? "step" : undefined}
              disabled={!enabled || !available[i]}
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => onStep(i)}
            >
              <span className="walk-n" aria-hidden="true">
                {i + 1}
              </span>
              <span className="walk-body">
                <span className="walk-title">{s.title}</span>
                {s.note && <span className="walk-note">{s.note}</span>}
                <span className="walk-where">
                  {available[i] ? (
                    <>
                      {where.code && <code>{where.code}</code>}
                      {where.code ? where.after : where.after?.replace(/^ /, "")}
                    </>
                  ) : (
                    "No longer available"
                  )}
                </span>
              </span>
            </button>
          </li>
        );
      })}
    </ol>
  );
}

function StepDetails({ item, pending }: { item: ToolItem; pending: boolean }) {
  const input = JSON.stringify(item.input, null, 2);
  return (
    <div className="step-details">
      <Clipped label="Input" text={input} />
      {item.result !== null ? (
        <Clipped label="Result" text={item.result || "(empty)"} />
      ) : (
        <p className="step-none">{pending ? "Waiting for the result…" : "No result: the run ended first."}</p>
      )}
    </div>
  );
}

/** Monospace text cut to a few lines until "Show more". */
function Clipped({ label, text }: { label: string; text: string }) {
  const [all, setAll] = useState(false);
  const { shown, more } = clip(text);
  return (
    <div className="clipped">
      <span className="eyebrow">{label}</span>
      <pre>{all ? text : shown}</pre>
      {more && (
        <button type="button" className="link-button" onClick={() => setAll((a) => !a)}>
          {all ? "Show less" : "Show more"}
        </button>
      )}
    </div>
  );
}

function Composer({
  state,
  online,
  draft,
  onDraft,
  harness,
  pickable,
  onHarness,
  running,
  busy,
  onSubmit,
  onStop,
  canvasWord,
  placeholder,
}: {
  state: AppState | null;
  online: boolean;
  draft: string;
  onDraft: (d: string) => void;
  harness: Harness;
  pickable: boolean;
  onHarness: (h: Harness) => void;
  running: boolean;
  busy: boolean;
  onSubmit: () => void;
  onStop: () => void;
  canvasWord: string;
  placeholder: string;
}) {
  const input = useRef<HTMLTextAreaElement>(null);
  // Grows with its text up to MAX_INPUT_PX.
  useLayoutEffect(() => {
    const el = input.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, MAX_INPUT_PX)}px`;
  }, [draft]);
  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    // Enter sends; Shift+Enter is a newline; Enter that ends an IME composition is the IME's.
    if (e.key !== "Enter" || e.shiftKey || e.nativeEvent.isComposing || e.keyCode === 229) return;
    e.preventDefault();
    onSubmit();
  };
  const session = state?.session?.status.kind;
  // A task's agent shows even if it is no longer installed.
  const available = state?.harnesses ?? [];
  const harnesses = available.includes(harness) ? available : [...available, harness];
  const project = state?.projects.find((p) => p.path === state.active);
  return (
    <div className="composer">
      <ul className="chips" aria-label="What the agent will see">
        {project && <li className="chip mono">{project.path === "." ? project.name : project.path}</li>}
        <li className="chip">{session === "running" ? "Game running" : "No game running"}</li>
        <li className="chip">{canvasWord}</li>
      </ul>
      <textarea
        ref={input}
        aria-label="Message the agent"
        rows={1}
        placeholder={placeholder}
        value={draft}
        disabled={!state}
        onChange={(e) => onDraft(e.target.value)}
        onKeyDown={onKeyDown}
      />
      <div className="composer-row">
        <button
          type="button"
          className="round"
          aria-label="Attach a capture or file (coming later)"
          title="Attaching captures and files comes later"
          aria-disabled="true"
        >
          <Icon name="plus" />
        </button>
        <span className="switcher model-picker">
          <select
            aria-label="Agent"
            title={pickable ? "The agent new tasks start with" : "A task keeps the agent it started with"}
            value={harness}
            disabled={!online || !pickable || !state?.harnesses.length}
            onChange={(e) => onHarness(e.target.value as Harness)}
          >
            {harnesses.map((h) => (
              <option key={h} value={h}>
                {h}
              </option>
            ))}
          </select>
          <Icon name="chevron" />
        </span>
        <span className="spacer" />
        <button
          type="button"
          className="round round-plain"
          aria-label="Hold to talk (coming later)"
          title="Voice input comes later"
          aria-disabled="true"
        >
          <Icon name="mic" />
        </button>
        {running ? (
          <button type="button" className="round round-stop" aria-label="Stop" disabled={!online} onClick={onStop}>
            <Icon name="stop" />
          </button>
        ) : (
          <button
            type="button"
            className="round round-send"
            aria-label="Send"
            disabled={!online || busy || !draft.trim()}
            onClick={onSubmit}
          >
            <Icon name="send" />
          </button>
        )}
      </div>
    </div>
  );
}
