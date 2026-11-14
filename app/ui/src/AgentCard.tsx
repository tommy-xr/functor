// An agent drawn as a small character (D34): its avatar, a thought bubble
// with what it is doing, and its stance. Claude is round and clay, Codex
// angular and violet; the face shows the stance.

import type { Activity, AppState, Harness, Stance, Subagent, Task } from "./generated/protocol";
import { bubble, cardLabel, cardTasks, ENGINE_WORDS, room, STANCE_WORDS, subBubble } from "./agent-model";
import { Icon } from "./Icon";
import { untitled } from "./tasks-model";

/** Eyes per stance, in the avatar's 36×36 box. */
function Face({ stance }: { stance: Stance }) {
  switch (stance) {
    case "thinking":
      // Looking up and away.
      return (
        <g className="avatar-eyes" fill="currentColor">
          <circle cx="15.5" cy="11.5" r="1.6" />
          <circle cx="21.5" cy="11.5" r="1.6" />
        </g>
      );
    case "working":
      return <path className="avatar-eyes" d="M13 14h4M19 14h4" />;
    case "waiting":
      // Eyes on you.
      return (
        <g className="avatar-eyes" fill="currentColor">
          <circle cx="14.5" cy="14" r="2.2" />
          <circle cx="21.5" cy="14" r="2.2" />
        </g>
      );
    case "done":
      return <path className="avatar-eyes" d="M12.5 14.5q2 -2 4 0M19.5 14.5q2 -2 4 0" />;
    case "failed":
      return <path className="avatar-eyes" d="M13 12.5l3 3M16 12.5l-3 3M20 12.5l3 3M23 12.5l-3 3" />;
  }
}

export function Avatar({ harness, stance }: { harness: Harness; stance: Stance }) {
  return (
    <svg className={`avatar avatar-${harness} avatar-${stance}`} viewBox="0 0 36 36" aria-hidden="true">
      {harness === "claude" ? (
        <>
          <path className="avatar-body" d="M6 35a12 12 0 0 1 24 0z" />
          <circle className="avatar-body" cx="18" cy="14" r="9" />
        </>
      ) : (
        <>
          <path className="avatar-body" d="M6 35l3.5-10h17L30 35z" />
          <path className="avatar-line" d="M18 4.5V2" />
          <rect className="avatar-body" x="9" y="5" width="18" height="17" rx="3" />
        </>
      )}
      {/* A raised hand: the user's turn. */}
      {stance === "waiting" && <path className="avatar-line avatar-hand" d="M28.5 29L32 19" />}
      <Face stance={stance} />
    </svg>
  );
}

/** One task's card; `onOpen` opens the task. */
export function AgentCard({ task, activity, onOpen }: { task: Task; activity: Activity; onOpen: (task: number) => void }) {
  const b = bubble(task, activity);
  const r = room(activity.context);
  const engine = ENGINE_WORDS[task.harness];
  return (
    <button
      type="button"
      className={`agent-card agent-${activity.stance}`}
      data-testid="agent-card"
      data-stance={activity.stance}
      aria-label={cardLabel(task, activity)}
      title={activity.error ? `${engine}: ${activity.error}` : engine}
      onClick={() => onOpen(task.id)}
    >
      <Avatar harness={task.harness} stance={activity.stance} />
      <span className="agent-text">
        <span className="agent-top">
          <span className="agent-title">{untitled(task)}</span>
          <span className="badge agent-stance" data-testid="agent-stance">
            {STANCE_WORDS[activity.stance]}
          </span>
        </span>
        <span className={`agent-bubble${b.error ? " agent-bubble-error" : ""}`} data-testid="agent-bubble">
          {b.kind ? <Icon name={`kind-${b.kind}`} /> : b.error && <Icon name="alert" />}
          <span className="agent-line">{b.text}</span>
          {b.count > 1 && (
            <span className="agent-count" data-testid="agent-count">
              {b.count}
            </span>
          )}
        </span>
        {activity.subagents.map((sub, i) => (
          <SubagentFigure key={i} harness={task.harness} sub={sub} />
        ))}
        {r && (
          <span className={`agent-room agent-room-${r.level}`} data-testid="agent-room" data-level={r.level} title={r.title}>
            <span className="agent-room-track">
              <span className="agent-room-fill" style={{ width: `${r.percent}%` }} />
            </span>
            <span className="agent-room-pct">{r.percent}%</span>
          </span>
        )}
      </span>
    </button>
  );
}

/** A subagent: a smaller figure on a tether from the agent that started it, with its own bubble. */
function SubagentFigure({ harness, sub }: { harness: Harness; sub: Subagent }) {
  const b = subBubble(sub);
  return (
    <span className={`agent-sub agent-${b.stance}`} data-testid="agent-sub" title={`Subagent: ${sub.label}`}>
      <Avatar harness={harness} stance={b.stance} />
      <span className="agent-bubble agent-sub-bubble">
        <Icon name={`kind-${b.kind}`} />
        <span className="agent-line">{b.text}</span>
      </span>
    </span>
  );
}

/** The Tasks pane's strip: a card per task still in play. */
export function AgentStrip({ state, onOpen }: { state: AppState | null; onOpen: (task: number) => void }) {
  const tasks = cardTasks(state);
  if (!state || tasks.length === 0) return null;
  return (
    <ul className="agent-strip" aria-label="Agents">
      {tasks.map((t) => (
        <li key={t.id}>
          <AgentCard task={t} activity={state.activity[t.id]!} onOpen={onOpen} />
        </li>
      ))}
    </ul>
  );
}
