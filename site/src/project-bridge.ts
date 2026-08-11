// The multi-file editor ↔ player bridge — the whole-project sibling of
// player-bridge.ts (which pushes a single source buffer). The IDE and the
// sandbox both own every .fun in memory, so they push the full file set over
// `functor-lang-set-project` to a `player.html?project=inline` iframe: the
// player boots from memory (no fetch) on the first push, then hot-swaps (model
// preserved) on each edit.
//
// The boot handshake: the player announces `functor-lang-project-waiting` when
// its listener is armed; only then may we push. It replies
// `functor-lang-preview-ready` once the producer is live, and
// `functor-lang-set-source-result` (with our echoed id) for every hot-swap.

import { asPlayerMessage } from "./protocol.js";
import type { BridgeOptions, ProjectFile, SetProject } from "./protocol.js";

const PUSH_DEBOUNCE_MS = 300;

// A rejected edit keeps the last good program running, so an error isn't urgent.
// Hold it back this long before surfacing it — a fix within the window (the
// common case while typing) clears it before it ever shows.
const ERROR_GRACE_MS = 4000;

export class ProjectBridge {
  readonly iframe: HTMLIFrameElement;
  private readonly onReloading: () => void;
  private readonly onLive: () => void;
  private readonly onResult: (ok: boolean, message: string) => void;
  private readonly debounceMs: number;
  private readonly errorGraceMs: number;

  private waiting = false; // player announced project-waiting (safe to push)
  // Has a push already BOOTED the current document? The boot push is a load,
  // not a reload — the host's "loading…" stands until the player reports in.
  private booted = false;
  private files: ProjectFile[] | null = null; // latest full file set
  private pushTimer: ReturnType<typeof setTimeout> | undefined;
  private errorTimer: ReturnType<typeof setTimeout> | undefined;
  // Correlates results with pushes: each push gets a fresh id, the runtime
  // echoes it, and a result for anything but the LATEST push is stale.
  private pushId = 0;

  constructor(
    iframe: HTMLIFrameElement,
    {
      onReloading,
      onLive,
      onResult,
      debounceMs = PUSH_DEBOUNCE_MS,
      errorGraceMs = ERROR_GRACE_MS,
      signal,
    }: BridgeOptions
  ) {
    this.iframe = iframe;
    this.onReloading = onReloading;
    this.onLive = onLive;
    this.onResult = onResult;
    this.debounceMs = debounceMs;
    this.errorGraceMs = errorGraceMs;

    // `signal` detaches the listener with a disposable pane (the sandbox's
    // mirror/server panes), exactly as PlayerBridge's does.
    window.addEventListener(
      "message",
      (event) => this.#onMessage(event),
      signal !== undefined ? { signal } : undefined
    );
  }

  // Debounced whole-project push: swap in the file set once edits settle.
  setProject(files: ProjectFile[]): void {
    this.files = files;
    clearTimeout(this.pushTimer);
    this.pushTimer = setTimeout(() => this.#send(), this.debounceMs);
  }

  // Reset for a fresh iframe (a new project=inline load): drop the handshake
  // state until the next `functor-lang-project-waiting`.
  reset(): void {
    clearTimeout(this.pushTimer);
    clearTimeout(this.errorTimer);
    this.waiting = false;
    this.booted = false;
  }

  // Surface a hot-swap result — but hold errors back. A rejected edit keeps the
  // last good program running, so the preview IS still live; show that now and
  // only surface the error if the program stays broken past the grace window.
  // Any success (the usual next keystroke that re-parses) clears it instantly.
  #deliverResult(ok: boolean, message: string): void {
    clearTimeout(this.errorTimer);
    if (ok) {
      this.onResult(true, message);
    } else {
      this.onLive();
      this.errorTimer = setTimeout(() => this.onResult(false, message), this.errorGraceMs);
    }
  }

  #send(): void {
    clearTimeout(this.pushTimer); // an early flush cancels the pending timer
    // An EMPTY set is rejected by the player as a malformed push, whose error
    // arrives as "live now, error in 4s" — so never send one; a bridge with
    // nothing to push simply waits for a real project.
    if (!this.iframe.contentWindow || !this.files?.length) return;
    // The player drops anything sent before it announces `project-waiting`;
    // hold the push and flush it on that signal (below).
    if (!this.waiting) return;
    // The BOOT push is this document's load — the host already says "loading…",
    // and calling it a reload would report a program that has never run as one
    // being swapped. Every later push is a real hot-swap.
    if (this.booted) this.onReloading();
    else this.booted = true;
    this.pushId += 1;
    const message: SetProject = {
      type: "functor-lang-set-project",
      files: this.files,
      id: this.pushId,
    };
    // Re-read `contentWindow` after `onReloading()`, matching PlayerBridge:
    // a callback that swapped the iframe must not have its push sent to the
    // outgoing window, and a detached one must throw rather than be dropped.
    this.iframe.contentWindow!.postMessage(message, "*");
  }

  #onMessage(event: MessageEvent): void {
    if (event.source !== this.iframe.contentWindow) return;
    const data = asPlayerMessage(event.data);
    if (!data) return;
    if (data.type === "functor-lang-project-waiting") {
      // The player is armed: flush the initial (or any held) project to boot it.
      this.waiting = true;
      if (this.files) this.#send();
    } else if (data.type === "functor-lang-preview-ready") {
      // Ignore a ready/result from the OUTGOING document — an iframe keeps its
      // WindowProxy (so `event.source` still matches) across a restart's src
      // swap, and a late reply from the old player would flash over the new
      // one's "loading…". `reset()` drops `waiting`; only the fresh
      // project-waiting handshake re-arms us. (Mirrors PlayerBridge's
      // previewReady guard.)
      if (!this.waiting) return;
      // The boot push carries an id but the boot path sends no result — the
      // ready signal is the boot's "ok". Later edits get result messages.
      this.onLive();
    } else if (data.type === "functor-lang-set-source-result") {
      if (!this.waiting) return;
      // A result whose id isn't the latest push's is stale — a newer push is
      // already in flight; its reply supersedes this one.
      if (data.id !== undefined && data.id !== this.pushId) return;
      this.#deliverResult(data.ok, data.message);
    }
  }
}
