// The editor ↔ player postMessage protocol — the seam between an editor page
// (sandbox, IDE, hero, demo-editor) and the wasm runtime inside its player
// iframe. Both bridges (player-bridge.ts, project-bridge.ts) speak it, the
// player implements the other end (site/player.html), and the VSCode
// live-preview panel speaks the single-source half of it.
//
// These messages cross a window boundary, so nothing checks them at runtime
// beyond the `type` discriminant: a payload typo used to fail silently. The
// types here are the single written-down description of that contract — keep
// them in step with site/player.html when the protocol changes.

/** One file of an in-memory project (the IDE's flat module space). */
export interface ProjectFile {
  path: string;
  source: string;
}

// --- editor → player ---------------------------------------------------------

/**
 * Greet a player that may already be live. Its one-shot `preview-ready` can
 * fire before a bridge attaches its listener; an already-live player answers a
 * hello with a fresh `preview-ready`.
 */
export interface PreviewHello {
  type: "functor-lang-preview-hello";
}

/**
 * Hot-swap the single entry buffer, preserving the model.
 *
 * `id` is optional because it is not universal: both bridges here always send
 * one, but the VSCode live-preview panel posts id-less pushes
 * (tools/functor-lang-vscode/client/extension.js), and the player and runtime
 * both accept them — an id-less push simply gets an id-less result (the
 * pre-id protocol).
 */
export interface SetSource {
  type: "functor-lang-set-source";
  source: string;
  id?: number;
}

/**
 * Boot from / hot-swap a whole in-memory project (the IDE's push).
 *
 * `files` must be non-empty and every `path` non-blank — the player rejects
 * anything else as a "malformed project push" (player.html). That is a runtime
 * precondition rather than a type: the only producer is ide.js, which is still
 * JavaScript, so a non-empty-tuple type here would be an unchecked assertion
 * at the boundary rather than a real guarantee.
 */
export interface SetProject {
  type: "functor-lang-set-project";
  files: ProjectFile[];
  /** Optional for the same reason as {@link SetSource.id}. */
  id?: number;
}

export type EditorMessage = PreviewHello | SetSource | SetProject;

/**
 * The callback contract both bridges expose: how protocol events map to UI.
 * It lives here rather than in either bridge so the two stay independent
 * siblings — `import type` + `verbatimModuleSyntax` erase it entirely, so
 * neither bundle pulls in the other.
 */
export interface BridgeOptions {
  /** a push was sent (busy) */
  onReloading: () => void;
  /** the player is ready with nothing pending (or, for a project, booted) */
  onLive: () => void;
  /** a hot-swap reply came back */
  onResult: (ok: boolean, message: string) => void;
  debounceMs?: number;
  errorGraceMs?: number;
}

// --- player → editor ---------------------------------------------------------

/** The producer is live. Also sent in answer to a `preview-hello`. */
export interface PreviewReady {
  type: "functor-lang-preview-ready";
}

/**
 * The player's listener is armed and it is waiting to be handed a project —
 * only after this may a `set-project` push land (`player.html?project=inline`).
 */
export interface ProjectWaiting {
  type: "functor-lang-project-waiting";
}

/**
 * The reply to a `set-source` / `set-project` hot-swap. `id` echoes the push
 * it answers; a result for anything but the latest push is stale. It is
 * optional because an older runtime omits it.
 */
export interface SetSourceResult {
  type: "functor-lang-set-source-result";
  ok: boolean;
  message: string;
  id?: number;
}

export type ConsoleLevel = "log" | "warn" | "error";

/**
 * A forwarded runtime console line (Functor Lang `Debug.log` and friends).
 * `frame` is the game frame it was emitted on, or null before the runtime has
 * recorded one (the Output panel then shows the time alone).
 */
export interface ConsoleLine {
  type: "functor-lang-console";
  level: ConsoleLevel;
  message: string;
  frame: number | null;
}

/**
 * The paused inspector's trace — live values and entry-point executions for
 * the paused frame. The payload is the interpreter's own shape, consumed only
 * by lang-intel; it stays `unknown` here until that module migrates.
 */
export interface InspectorTrace {
  type: "functor-inspector-trace";
  trace: unknown;
}

export type PlayerMessage =
  | PreviewReady
  | ProjectWaiting
  | SetSourceResult
  | ConsoleLine
  | InspectorTrace;

const PLAYER_MESSAGE_TYPES = new Set<PlayerMessage["type"]>([
  "functor-lang-preview-ready",
  "functor-lang-project-waiting",
  "functor-lang-set-source-result",
  "functor-lang-console",
  "functor-inspector-trace",
]);

/**
 * Narrow a `MessageEvent.data` of unknown provenance to a player message.
 * Anything that is not an object carrying one of the KNOWN discriminants is
 * rejected — null, numbers, and another library's window traffic alike.
 *
 * Checking the discriminant (rather than merely "has some string `type`") is
 * what makes the returned type honest. It is also behaviour-preserving: every
 * consumer already switched on these exact types and ignored the rest.
 *
 * The PAYLOAD fields are asserted, not validated: the player is same-origin
 * first-party code (site/player.html) and the `event.source` check upstream
 * pins the sender to our own iframe, so a well-formed discriminant implies a
 * well-formed body. Validating fields here would additionally CHANGE
 * behaviour — the old code passed a malformed body straight through to the
 * callbacks — so it is deliberately out of scope for this migration.
 */
export const asPlayerMessage = (data: unknown): PlayerMessage | null =>
  typeof data === "object" &&
  data !== null &&
  PLAYER_MESSAGE_TYPES.has((data as { type?: PlayerMessage["type"] }).type as PlayerMessage["type"])
    ? (data as PlayerMessage)
    : null;
