// The chat dock's pure parts: tool-call phrases, transcript updates, outcome
// pills, connection lines and scroll-follow. No React, so `node --test` runs them.

import type { Item, JsonValue, SessionStatus, Task, Usage } from "./generated/protocol";

/** A step's one line: a verb, and a file, command or pattern shown as code. */
export type Phrase = { text: string; code?: string };

/** `mcp__<server>__get_state` → `get_state`; other names unchanged. */
export function toolName(name: string): string {
  return name.replace(/^mcp__.+?__/, "");
}

/** The game tools (`functor mcp`'s `#[tool]`s in the CLI) by bare name. */
const GAME_TOOLS = new Map(
  Object.entries({
    get_state: "Read the game state",
    get_scene: "Read the scene",
    get_frames: "Read the frame timings",
    get_spikes: "Read the frame spikes",
    get_trace: "Read the frame trace",
    capture_frame: "Captured a frame",
    send_input: "Sent input to the game",
    pause: "Paused the game",
    resume: "Resumed the game",
    step_all: "Stepped every session",
    rewind: "Rewound the game",
    reload_source: "Reloaded the game",
    reload_project: "Reloaded the project",
    launch_game: "Launched the game",
    launch_session_group: "Launched a session group",
    connect_game: "Connected to the game",
    list_sessions: "Listed the game sessions",
    stop_game: "Stopped the game",
    wire_log: "Read the network log",
    run_game_code_unsafe: "Ran a game script",
    init_game: "Created a game",
    save_project: "Saved the project",
    app_context: "Checked what you are looking at",
    api_reference: "Read the API reference",
    language_guide: "Read the language guide",
  }),
);

/** The short human line for a tool call, from its name and input. */
export function stepPhrase(name: string, input: JsonValue): Phrase {
  const bare = toolName(name);
  const arg = (key: string) => {
    const v = isObject(input) ? input[key] : undefined;
    return typeof v === "string" && v.trim() ? v.trim() : undefined;
  };
  const file = arg("file_path") ?? arg("notebook_path") ?? arg("path");
  const named = (text: string, fallback: string) =>
    file ? { text, code: basename(file) } : { text: `${text} ${fallback}` };
  switch (bare) {
    case "Read":
      return named("Read", "a file");
    case "Edit":
    case "MultiEdit":
    case "NotebookEdit":
      return named("Edited", "a file");
    case "Write":
      return named("Wrote", "a file");
    case "Bash": {
      const command = arg("command");
      return command ? { text: "Ran", code: firstLine(command, 60) } : { text: "Ran a command" };
    }
    case "Grep": {
      const pattern = arg("pattern");
      return pattern ? { text: "Searched for", code: firstLine(pattern, 60) } : { text: "Searched the code" };
    }
    case "Glob": {
      const pattern = arg("pattern");
      return pattern ? { text: "Listed", code: firstLine(pattern, 60) } : { text: "Listed files" };
    }
    // Claude Code loading a deferred tool's schema before its first call.
    case "ToolSearch":
      return { text: "Looked up tools" };
    case "step": {
      const frames = isObject(input) && typeof input.frames === "number" ? input.frames : 1;
      return { text: `Stepped ${frames} frame${frames === 1 ? "" : "s"}` };
    }
  }
  return { text: GAME_TOOLS.get(bare) ?? bare };
}

export function phraseText({ text, code }: Phrase): string {
  return code ? `${text} ${code}` : text;
}

/** How a tool call shows: a change card, a capture card, or a plain step. */
export function toolKind(name: string): "change" | "capture" | "step" {
  const bare = toolName(name);
  if (bare === "Edit" || bare === "MultiEdit" || bare === "Write" || bare === "NotebookEdit") return "change";
  if (bare === "capture_frame") return "capture";
  return "step";
}

function isObject(v: JsonValue): v is { [key: string]: JsonValue } {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function basename(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path;
}

/** The first non-blank line, cut to `max` characters with `…`. */
export function firstLine(text: string, max = 120): string {
  const line = text.split("\n").find((l) => l.trim()) ?? "";
  const trimmed = line.trim();
  return trimmed.length > max ? `${trimmed.slice(0, max - 1)}…` : trimmed;
}

/**
 * A task's transcript after `ServerMsg::Item`: replaced at `index`, or
 * appended when `index` is the next one. `undefined` (not fetched) stays
 * so; a gap answers `"refetch"`.
 */
export function applyItem(items: Item[] | undefined, index: number, item: Item): Item[] | undefined | "refetch" {
  if (items === undefined) return undefined;
  if (index < items.length) {
    const next = items.slice();
    next[index] = item;
    return next;
  }
  if (index === items.length) return [...items, item];
  return "refetch";
}

export type Outcome = { tone: "ok" | "danger" | "neutral"; label: string };

/** The result pill for a `finished` or `stopped` item; `null` for others. */
export function outcome(item: Item): Outcome | null {
  if (item.kind === "stopped") return { tone: "neutral", label: "Stopped" };
  if (item.kind !== "finished") return null;
  if (item.ok) return { tone: "ok", label: "Done" };
  const why = item.summary ? firstLine(item.summary) : "";
  return { tone: "danger", label: why ? `Failed: ${why}` : "Failed" };
}

/** The run's usage, which the core adds just before its outcome. */
export function usageBefore(items: Item[], index: number): Usage | null {
  const prev = items[index - 1];
  return prev?.kind === "usage" ? { input: prev.input, output: prev.output } : null;
}

/** `2400` → `2.4k`, `1250000` → `1.3M`. */
export function tokens(n: number): string {
  if (n < 1000) return String(n);
  const [d, unit] = n < 1e6 ? [1e3, "k"] : [1e6, "M"];
  return `${(n / d).toFixed(1).replace(/\.0$/, "")}${unit}`;
}

export function usageText(usage: Usage): string {
  return `${tokens(usage.input)} tokens in · ${tokens(usage.output)} out`;
}

/** A tool result or input cut for the collapsed view: at most `lines` lines and `chars` characters. */
export function clip(text: string, lines = 12, chars = 800): { shown: string; more: boolean } {
  let shown = text.split("\n").slice(0, lines).join("\n");
  if (shown.length > chars) shown = shown.slice(0, chars);
  return shown.length < text.length ? { shown: `${shown}…`, more: true } : { shown: text, more: false };
}

/** The task the dock opens on: the newest in `project` that is in progress or waiting on you. */
export function initialTask(tasks: Task[], project: string | null): number | null {
  const open = tasks.filter(
    (t) => t.project === project && (t.status.kind === "in_progress" || t.status.kind === "waiting_on_you"),
  );
  return open.length ? Math.max(...open.map((t) => t.id)) : null;
}

/** What the connection lines are derived from: the socket and the active project's session. */
export type Snapshot = {
  conn: "connecting" | "open" | "reconnecting";
  /** `undefined` before the first state. */
  active: string | null | undefined;
  session: SessionStatus | null;
};

/** The quiet lines the dock shows for a change from `prev` to `next`. */
export function connectionLines(prev: Snapshot, next: Snapshot): string[] {
  const lines: string[] = [];
  if (prev.conn === "open" && next.conn === "reconnecting") lines.push("Connection lost, reconnecting…");
  if (prev.conn === "reconnecting" && next.conn === "open") lines.push("Reconnected");
  // A switched project restarts its session; that is not news about this one.
  if (prev.active === undefined || prev.active !== next.active) return lines;
  const was = prev.session?.kind ?? "stopped";
  const now = next.session ?? { kind: "stopped" as const };
  if (was === now.kind) return lines;
  if (now.kind === "running") lines.push("Game session started");
  else if (now.kind === "stopped" && (was === "running" || was === "starting")) lines.push("Game session stopped");
  else if (now.kind === "failed") lines.push(`Game session failed: ${firstLine(now.reason)}`);
  else if (now.kind === "exited") lines.push(`Game session exited: ${firstLine(now.reason)}`);
  return lines;
}

/** Whether a scroller is at (within `slack` px of) its bottom. */
export function atBottom(el: { scrollTop: number; scrollHeight: number; clientHeight: number }, slack = 24): boolean {
  return el.scrollHeight - el.scrollTop - el.clientHeight <= slack;
}
