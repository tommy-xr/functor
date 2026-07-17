// The multi-file editor ↔ player bridge — the whole-project sibling of
// player-bridge.js (which pushes a single source buffer). The IDE owns every
// .fun in memory, so it pushes the full file set over `functor-lang-set-project`
// to a `player.html?project=inline` iframe: the player boots from memory (no
// fetch) on the first push, then hot-swaps (model preserved) on each edit.
//
// The boot handshake: the player announces `functor-lang-project-waiting` when
// its listener is armed; only then may we push. It replies
// `functor-lang-preview-ready` once the producer is live, and
// `functor-lang-set-source-result` (with our echoed id) for every hot-swap.

const PUSH_DEBOUNCE_MS = 300;

// A rejected edit keeps the last good program running, so an error isn't urgent.
// Hold it back this long before surfacing it — a fix within the window (the
// common case while typing) clears it before it ever shows.
const ERROR_GRACE_MS = 4000;

export class ProjectBridge {
  // iframe: the player element. Callbacks map protocol events to UI:
  //   onReloading()          — a push was sent (busy)
  //   onLive()               — the player booted / is ready
  //   onResult(ok, message)  — a hot-swap reply came back
  constructor(
    iframe,
    {
      onReloading,
      onLive,
      onResult,
      debounceMs = PUSH_DEBOUNCE_MS,
      errorGraceMs = ERROR_GRACE_MS,
    }
  ) {
    this.iframe = iframe;
    this.onReloading = onReloading;
    this.onLive = onLive;
    this.onResult = onResult;
    this.debounceMs = debounceMs;
    this.errorGraceMs = errorGraceMs;

    this.waiting = false; // player announced project-waiting (safe to push)
    this.files = null; // latest full file set
    this.pushTimer = null;
    this.errorTimer = null;
    // Correlates results with pushes: each push gets a fresh id, the runtime
    // echoes it, and a result for anything but the LATEST push is stale.
    this.pushId = 0;

    window.addEventListener("message", (event) => this.#onMessage(event));
  }

  // Debounced whole-project push: swap in the file set once edits settle.
  setProject(files) {
    this.files = files;
    clearTimeout(this.pushTimer);
    this.pushTimer = setTimeout(() => this.#send(), this.debounceMs);
  }

  // Reset for a fresh iframe (a new project=inline load): drop the handshake
  // state until the next `functor-lang-project-waiting`.
  reset() {
    clearTimeout(this.pushTimer);
    clearTimeout(this.errorTimer);
    this.waiting = false;
  }

  // Surface a hot-swap result — but hold errors back. A rejected edit keeps the
  // last good program running, so the preview IS still live; show that now and
  // only surface the error if the program stays broken past the grace window.
  // Any success (the usual next keystroke that re-parses) clears it instantly.
  #deliverResult(ok, message) {
    clearTimeout(this.errorTimer);
    if (ok) {
      this.onResult(true, message);
    } else {
      this.onLive();
      this.errorTimer = setTimeout(() => this.onResult(false, message), this.errorGraceMs);
    }
  }

  #send() {
    clearTimeout(this.pushTimer); // an early flush cancels the pending timer
    if (!this.iframe.contentWindow || !this.files) return;
    // The player drops anything sent before it announces `project-waiting`;
    // hold the push and flush it on that signal (below).
    if (!this.waiting) return;
    this.onReloading();
    this.pushId += 1;
    this.iframe.contentWindow.postMessage(
      { type: "functor-lang-set-project", files: this.files, id: this.pushId },
      "*"
    );
  }

  #onMessage(event) {
    if (event.source !== this.iframe.contentWindow) return;
    const data = event.data;
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
