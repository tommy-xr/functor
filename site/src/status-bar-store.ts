// The status bar's LOGIC and types, feeding the React view in
// `components/StatusBar.tsx`: the problems list, the rAF-coalesced output log
// (capped, newest wins), and the paused inspector's executions picker.
//
// The store implements the `StatusBar` handle its producers already spoke to —
// the pages' own `appendOutput` calls and `wireLiveTrace`'s `setExecutions` —
// so those call sites never had to change. (The types lived in the imperative
// `status-bar.ts` until the IDE converted too; that module is gone.)

import type { ConsoleLevel } from "./protocol.js";

const MAX_OUTPUT_LINES = 500;

/**
 * One Problems row. `severity` is the producer's own (every call site passes a
 * CodeMirror lint `Diagnostic.severity`), and the bar only ever distinguishes
 * "error" from everything else — so it stays a plain string rather than
 * coupling this editor-agnostic surface to CodeMirror's union.
 */
export interface Problem {
  severity?: string;
  message: string;
  /** Pre-formatted location, e.g. `game.fun 12:5`. */
  loc: string;
  /** Jumping is the host's job (it closes over its editor). */
  jump?: () => void;
}

/** One row of the paused inspector's entry-point picker. */
export interface Execution {
  label: string;
  selected: boolean;
  onPick?: () => void;
}

/** The handle the producing pages hold. */
export interface StatusBar {
  setProblems: (items: Problem[]) => void;
  appendOutput: (level: ConsoleLevel, text: string, frame?: number | null) => void;
  setExecutions: (items: Execution[]) => void;
  /**
   * A slot on the status strip for whichever surface is currently open — the
   * pane grid's NETWORK legend and its demoted pane readouts today.
   *
   * It is an ELEMENT rather than store state on purpose: its owner (mp-panes)
   * is imperative DOM outside the React islands and repaints per frame, and
   * routing a 60Hz frame counter through the store would re-render the whole
   * bar — output list included — once per frame. React mounts this node and
   * never looks inside it.
   */
  viewStrip: HTMLElement;
}

/** One rendered output row. `time` is stamped at append, not at flush. */
export interface OutputLine {
  level: ConsoleLevel;
  text: string;
  frame: number | null;
  time: string;
  /** Render key: appends are monotonic, so a counter is a stable identity. */
  id: number;
}

export interface StatusBarSnapshot {
  problems: Problem[];
  output: OutputLine[];
  executions: Execution[];
}

export interface StatusBarStore extends StatusBar {
  subscribe: (listener: () => void) => () => void;
  getSnapshot: () => StatusBarSnapshot;
}

// Each line carries a `[Frame N | HH:MM:SS]` preamble — the game frame it was
// emitted on (when the runtime had one) and the wall clock.
const clock = (date: Date): string => {
  const two = (n: number): string => String(n).padStart(2, "0");
  return `${two(date.getHours())}:${two(date.getMinutes())}:${two(date.getSeconds())}`;
};

export const outputPreamble = (frame: number | null, time: string): string =>
  frame == null ? `[${time}]` : `[Frame ${frame} | ${time}]`;

export const createStatusBarStore = (): StatusBarStore => {
  let snapshot: StatusBarSnapshot = { problems: [], output: [], executions: [] };
  const listeners = new Set<() => void>();
  const viewStrip = document.createElement("div");
  viewStrip.className = "statusbar-view";

  const emit = (next: StatusBarSnapshot): void => {
    snapshot = next;
    for (const listener of listeners) listener();
  };

  // Appends are rAF-coalesced: a per-frame `Debug.log` in tick/draw arrives
  // ~60/sec, and one store update (hence one React render) per frame keeps the
  // panel usable under a logging loop.
  let pending: OutputLine[] = [];
  let flushScheduled = false;
  let nextId = 0;

  const flushOutput = (): void => {
    flushScheduled = false;
    if (pending.length === 0) return;
    // A burst larger than the cap only ever shows its tail — skip the rest.
    const lines = pending.slice(-MAX_OUTPUT_LINES);
    pending = [];
    emit({ ...snapshot, output: [...snapshot.output, ...lines].slice(-MAX_OUTPUT_LINES) });
  };

  return {
    viewStrip,
    subscribe: (listener) => {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    getSnapshot: () => snapshot,

    setProblems: (items) => emit({ ...snapshot, problems: items }),

    // `frame` is the game frame the line belongs to (null when the runtime had
    // none to offer — boot lines, host-side reload errors).
    appendOutput: (level, text, frame = null) => {
      pending.push({ level, text, frame, time: clock(new Date()), id: nextId++ });
      if (!flushScheduled) {
        flushScheduled = true;
        requestAnimationFrame(flushOutput);
      }
    },

    // The paused frame's entry-point executions (the inspector's picker), in
    // frame order. Empty while the game plays.
    setExecutions: (items) => emit({ ...snapshot, executions: items }),
  };
};
