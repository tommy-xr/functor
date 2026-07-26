// The SITE's editor ↔ player postMessage protocol — the seam between an editor
// page (sandbox, IDE, hero, demo-editor) and the wasm runtime inside its
// player iframe. Both bridges (player-bridge.ts, project-bridge.ts) speak it;
// site/player.html implements the other end.
//
// These messages cross a window boundary, so nothing checks them at runtime
// beyond the `type` discriminant: a payload typo used to fail silently. Keep
// these types in step with site/player.html when the protocol changes.
//
// SCOPE — this describes the site's seam, not every speaker of every related
// message. Two neighbours overlap without being covered here:
//   • The VSCode live-preview panel sends `set-source`/`set-project` too, but
//     also a `functor-lang-preview-navigate` of its own, and its preview page
//     (runtime/functor-runtime-web/index-functor-lang.html) has no `hello`
//     handler — so the hello/ready handshake below is player.html-specific.
//   • The Rust web runtime posts `set-source-result` directly
//     (functor-runtime-web/src/lib.rs), in the same shape as player.html.

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
 * precondition rather than a type: the only producer is ide.ts, which builds
 * this list from user-editable localStorage, so a non-empty-tuple type here
 * would be an unchecked assertion at the boundary rather than a real
 * guarantee.
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
 * siblings — neither has to import the other just to name its own options.
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
 * omitted whenever the push carried no id — by older runtimes, but also by
 * the CURRENT one for an id-less push (the VSCode panel's, or a rejected
 * malformed project).
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
 * by lang-intel, which models it (`InspectorTraceDoc`) and reads it
 * defensively. It stays `unknown` HERE on purpose: nothing validates a
 * cross-window payload, so naming a shape at this seam would assert a
 * guarantee the boundary cannot make.
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

const PLAYER_MESSAGE_TYPES: ReadonlySet<string> = new Set<PlayerMessage["type"]>([
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
export const asPlayerMessage = (data: unknown): PlayerMessage | null => {
  if (typeof data !== "object" || data === null) return null;
  const { type } = data as { type?: unknown };
  return typeof type === "string" && PLAYER_MESSAGE_TYPES.has(type)
    ? (data as PlayerMessage)
    : null;
};
