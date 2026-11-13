import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import type { AppState, Decision, Item, Task } from "./generated/protocol";
import { stepAvailable, tokens } from "./chat-model";
import { WalkSteps } from "./Chat";
import { DEPTH_WORDS, evidenceLines, firstOf, sizeLine, TAB_WORDS, TABS, VISIBLE, type Tab } from "./decision-model";
import { Counts, DiffView, FileCounts, ReviewActions, TaskBadge } from "./DiffView";
import { hasChange, plural, totals } from "./diff-model";
import { Icon } from "./Icon";
import { tabTarget } from "./menu";
import { PairFrames, PathSection } from "./PathEvidence";
import type { DiffMsg, Notice, Send } from "./socket";
import { taskPage, usageTable, type RunSteps, type TaskPage as Page } from "./task-page-model";
import { relativeTime, untitled } from "./tasks-model";
import { RenameInput, useNow } from "./TasksPane";

/**
 * A task's page (the `Task` tab): a read view of what the agent was given,
 * its steps, its change (Keep / Revert as in the Changes tab), its
 * walkthrough and its usage, assembled from the transcript. While its
 * change waits, the decision page (D35) leads: the first page, then depth
 * as tabs, with Keep / Revert in the header.
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
  diff,
  fetchDiff,
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
  diff: DiffMsg | undefined;
  fetchDiff: (task: number) => void;
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
  const decision = state?.decisions[task.id];
  const walk = page?.walkthrough ? (
    <WalkSteps
      steps={page.walkthrough.steps}
      current={page.walkthrough.current}
      available={page.walkthrough.steps.map((s) =>
        stepAvailable(s.view, state?.files ?? [], !!state?.tracks_changes && hasChange(task)),
      )}
      enabled={online}
      onStep={(step) => send({ type: "walkthrough_step", task: task.id, item: page.walkthrough!.item, step })}
    />
  ) : null;
  const details =
    !page || !usage ? (
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
        {/* While a change waits, the decision page's tabs hold these. */}
        {!decision && (
          <>
            <Section id={id("change")} title="Change">
              <ChangeSection task={task} tracks={state?.tracks_changes ?? false} now={now} {...{ online, send, notice, onOpenChanges }} />
            </Section>
            {task.evidence && (
              <Section id={id("paths")} title="Predicted path">
                <PathSection task={task} />
              </Section>
            )}
            {walk && (
              <Section id={id("walk")} title="Walkthrough">
                {walk}
              </Section>
            )}
          </>
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
    );
  return (
    <article className={decision ? "task-page task-page-decision" : "task-page"} data-testid="task-page" aria-labelledby={id("title")}>
      <header className="task-page-head">
        <div className="task-page-title">
          <Title task={task} id={id("title")} online={online} send={send} />
          <TaskBadge status={task.status} testid="task-page-status" />
          {decision && (
            <div className="task-page-decide">
              <ReviewActions task={task} send={send} online={online} notice={notice} />
            </div>
          )}
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
      {decision ? (
        <Depths
          // Its own tab state per task and change: the canvas reuses this page for another task.
          key={`${task.id} ${task.change?.tree}`}
          task={task}
          decision={decision}
          page={page}
          walk={walk}
          details={details}
          diff={<DiffView bare target={null} {...{ state, task, diff, fetchDiff, send, online, notice }} />}
        />
      ) : (
        details
      )}
    </article>
  );
}

/**
 * Per task and change (its tree), the tab its decision page last showed:
 * kept across mounts (a split remounts it); a new change leads with the
 * first page.
 */
const shownTabs = new Map<string, Tab>();

/**
 * The decision page's tabs (the ARIA tabs pattern, as the canvas's): the
 * first page, then each part that backs it.
 */
function Depths({
  task,
  decision,
  page,
  walk,
  details,
  diff,
}: {
  task: Task;
  decision: Decision;
  page: Page | null;
  walk: ReactNode;
  details: ReactNode;
  diff: ReactNode;
}) {
  const shown = `${task.id} ${task.change?.tree}`;
  const [tab, setTab] = useState<Tab>(() => shownTabs.get(shown) ?? "decision");
  const tabId = (t: Tab) => `task-${task.id}-tab-${t}`;
  const show = (t: Tab, focus: boolean) => {
    shownTabs.set(shown, t);
    setTab(t);
    if (focus) document.getElementById(tabId(t))?.focus();
  };
  // Arrow keys move between tabs and show them; Home and End jump.
  const onKey = (e: KeyboardEvent, i: number) => {
    const to = tabTarget(e.key, i, TABS.length);
    if (to === null) return;
    e.preventDefault();
    show(TABS[to]!, true);
  };
  return (
    <>
      <div className="depth-tabs" role="tablist" aria-label="Decision page">
        {TABS.map((t, i) => (
          <button
            key={t}
            type="button"
            role="tab"
            id={tabId(t)}
            className="depth-tab"
            aria-selected={t === tab}
            aria-controls={`task-${task.id}-depth`}
            tabIndex={t === tab ? 0 : -1}
            onClick={() => show(t, false)}
            onKeyDown={(e) => onKey(e, i)}
          >
            {i > 0 && (
              <span className="depth-n" aria-hidden="true">
                {i}
              </span>
            )}
            {TAB_WORDS[t]}
          </button>
        ))}
      </div>
      <div
        className={`depth-panel depth-panel-${tab}`}
        role="tabpanel"
        id={`task-${task.id}-depth`}
        aria-labelledby={tabId(tab)}
        data-testid={`depth-${tab}`}
      >
        {tab === "decision" && (
          <>
            <FirstPage task={task} decision={decision} highlights={!!walk} onTab={(t) => show(t, true)} />
            {details}
          </>
        )}
        {tab === "highlights" && (walk ?? <p className="hint">{page ? "The agent gave no walkthrough of this change." : "Loading…"}</p>)}
        {tab === "captures" &&
          (task.evidence ? <PathSection task={task} /> : <p className="hint">There are no captures of this change.</p>)}
        {tab === "changes" && diff}
      </div>
    </>
  );
}

/** A first-page item's way into the tab that backs it, numbered as the tab is. */
function TabLink({ tab, onTab }: { tab: Tab; onTab: (tab: Tab) => void }) {
  return (
    <button type="button" className="tab-link" aria-label={`Open ${TAB_WORDS[tab]}`} onClick={() => onTab(tab)}>
      <span className="depth-n" aria-hidden="true">
        {TABS.indexOf(tab)}
      </span>
      {TAB_WORDS[tab]}
    </button>
  );
}

/**
 * The first page (D35), in order: the size, how deep to look, the before
 * and after in place, the agent's summary, the evidence and the things to
 * note. Only what the app knows or the agent reported; nothing invented.
 */
function FirstPage({
  task,
  decision,
  highlights,
  onTab,
}: {
  task: Task;
  decision: Decision;
  highlights: boolean;
  onTab: (tab: Tab) => void;
}) {
  const { size, depth, reason } = decision;
  const report = task.report;
  const kind = task.evidence?.state.kind;
  const pair = kind === "shown" || kind === "taking";
  const summary = report?.summary ?? null;
  const id = (part: string) => `task-${task.id}-first-${part}`;
  // Alone, the pair or the summary takes the whole row.
  const wide = (on: boolean) => (on ? "" : " first-wide");
  return (
    <div className="first-page" data-testid="first-page">
      <section className="first-size" aria-labelledby={id("size")}>
        <h3 id={id("size")} className="eyebrow first-head">
          Size of the change
        </h3>
        <p className="first-size-line" data-testid="first-size">
          <Counts added={size.added} removed={size.removed} /> <span>{sizeLine(size)}</span>
        </p>
        <TabLink tab="changes" onTab={onTab} />
      </section>
      <section className={`first-verdict verdict verdict-${depth}`} aria-label="How deep to look" data-testid="first-verdict">
        <p className="first-verdict-word">{DEPTH_WORDS[depth]}</p>
        <p className="first-verdict-reason">{reason}</p>
      </section>
      {pair && (
        <section className={`first-pair${wide(!!summary)}`} aria-labelledby={id("pair")}>
          <h3 id={id("pair")} className="eyebrow first-head">
            Before and after
          </h3>
          <PairFrames task={task} onOpen={() => onTab("captures")} />
        </section>
      )}
      {summary && (
        <section className={`first-summary${wide(pair)}`} aria-labelledby={id("summary")}>
          <h3 id={id("summary")} className="eyebrow first-head">
            Summary
          </h3>
          <p className="first-summary-text" data-testid="first-summary">
            {summary}
          </p>
          {highlights && <TabLink tab="highlights" onTab={onTab} />}
        </section>
      )}
      <section className="first-list" aria-labelledby={id("evidence")}>
        <h3 id={id("evidence")} className="eyebrow first-head">
          Evidence
        </h3>
        <Lines
          testid="first-evidence"
          empty="No check was reported and no path was compared."
          onTab={onTab}
          items={evidenceLines(task).map((line) => ({
            icon: line.ok === null ? null : line.ok ? "check" : "cross",
            tone: line.ok === null ? "neutral" : line.ok ? "ok" : "danger",
            body: line.code ? (
              <>
                <code>{line.code}</code> {line.text}
              </>
            ) : (
              line.text
            ),
            tab: line.tab,
          }))}
        />
      </section>
      {/* Without a report, what the agent did not check is unknown: no list. */}
      {report && (
        <section className="first-list" aria-labelledby={id("notes")}>
          <h3 id={id("notes")} className="eyebrow first-head">
            Things to note
          </h3>
          <Lines
            testid="first-notes"
            // An empty list is the agent saying so; none at all, it did not say.
            empty={report.notes ? "Nothing to note." : "None reported."}
            onTab={onTab}
            items={(report.notes ?? []).map((note) => ({ icon: "alert", tone: "warn", body: note }))}
          />
        </section>
      )}
    </div>
  );
}

type Line = { icon: "check" | "cross" | "alert" | null; tone: string; body: ReactNode; tab?: Tab | undefined };

/** A short list: the first `VISIBLE` lines, the rest behind "Show N more". */
function Lines({ items, empty, testid, onTab }: { items: Line[]; empty: string; testid: string; onTab: (tab: Tab) => void }) {
  const [all, setAll] = useState(false);
  if (items.length === 0) return <p className="hint">{empty}</p>;
  const { shown, more } = all ? { shown: items, more: 0 } : firstOf(items, VISIBLE);
  return (
    <>
      <ul className="first-lines" data-testid={testid}>
        {shown.map((item, i) => (
          <li key={i} className={`first-line first-line-${item.tone}`}>
            <span className="first-line-icon">{item.icon ? <Icon name={item.icon} /> : <span className="step-dot" />}</span>
            <span className="first-line-body">{item.body}</span>
            {item.tab && <TabLink tab={item.tab} onTab={onTab} />}
          </li>
        ))}
      </ul>
      {items.length > VISIBLE && (
        <button type="button" className="link-button" aria-expanded={all} onClick={() => setAll((a) => !a)}>
          {all ? "Show fewer" : `Show ${more} more`}
        </button>
      )}
    </>
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
  const button = useRef<HTMLButtonElement>(null);
  if (editing) {
    const done = () => {
      setEditing(false);
      setTimeout(() => button.current?.focus());
    };
    return <RenameInput task={task} send={send} className="task-page-rename" onDone={done} />;
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
            <FileCounts file={f} />
          </li>
        ))}
        {task.change.more > 0 && <li className="hint">and {plural(task.change.more, "more file")}</li>}
      </ul>
      <div className="task-change-actions">
        <button type="button" className="button" onClick={onOpenChanges}>
          Open changes
        </button>
        {/* As the Changes tab offers them: on any change, disabled while it runs or reverts. */}
        <ReviewActions task={task} send={send} online={online} notice={notice} />
      </div>
    </div>
  );
}
