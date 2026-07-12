// The editor ↔ player postMessage protocol, factored out of sandbox.js so the
// landing hero can drive the same live-reload loop. Dependency-free (no
// CodeMirror): the consumer owns the editor and hands source strings in.
//
// The seam is the one the VSCode live-preview panel uses: edits are debounced
// and pushed as `functor-lang-set-source`; the runtime hot-swaps the program
// with the model preserved and replies `functor-lang-set-source-result`. The
// player announces itself with `functor-lang-preview-ready` after it boots.

// Same cadence as the VSCode extension: fast enough to feel live, slow enough
// not to push a reload per keystroke.
const PUSH_DEBOUNCE_MS = 300;

export class PlayerBridge {
  // iframe: the player element. Callbacks map protocol events to UI:
  //   onReloading()          — a push was sent (busy)
  //   onLive()               — the player is ready with nothing pending
  //   onResult(ok, message)  — a hot-swap reply came back
  constructor(iframe, { onReloading, onLive, onResult, debounceMs = PUSH_DEBOUNCE_MS }) {
    this.iframe = iframe;
    this.onReloading = onReloading;
    this.onLive = onLive;
    this.onResult = onResult;
    this.debounceMs = debounceMs;

    this.previewReady = false;
    this.dirty = false;
    this.pushTimer = null;
    this.lastSource = "";
    // Correlates results with pushes: each posted push gets a fresh id, the
    // runtime echoes it in the result, and a result for anything but the
    // LATEST push is stale — ignore it, its reply is coming. Results with no
    // id (an older runtime) are accepted as before.
    this.pushId = 0;

    // Replies and readiness from the player iframe. Only trust the iframe we
    // created (same-origin, but be explicit about the source anyway).
    window.addEventListener("message", (event) => this.#onMessage(event));

    // Handshake: the player posts a one-shot `functor-lang-preview-ready` when it
    // boots, but under load that can fire before this bridge exists (its message
    // listener isn't attached yet), leaving the status stuck "busy". So also
    // greet the player: post `functor-lang-preview-hello` into the iframe now and
    // on every load — an already-live player replies with the ready message.
    this.#hello();
    this.iframe.addEventListener("load", () => this.#hello());
  }

  // Greet the player so an already-live one re-announces readiness. Harmless if
  // the player isn't up yet: it ignores the hello and its one-shot ready (now
  // reaching our attached listener) covers that direction.
  #hello() {
    this.iframe.contentWindow?.postMessage({ type: "functor-lang-preview-hello" }, "*");
  }

  // Debounced live edit: swap in `source` once the buffer settles.
  push(source) {
    this.lastSource = source;
    clearTimeout(this.pushTimer);
    this.pushTimer = setTimeout(() => this.#post(), this.debounceMs);
  }

  // Reset for a fresh iframe load (fresh model: init runs). Cancels any pending
  // push and drops readiness until the next `functor-lang-preview-ready`.
  reset() {
    clearTimeout(this.pushTimer);
    this.previewReady = false;
    this.dirty = false;
  }

  #post() {
    if (!this.previewReady || !this.iframe.contentWindow) {
      this.dirty = true;
      return;
    }
    this.dirty = false;
    this.onReloading();
    this.pushId += 1;
    this.iframe.contentWindow.postMessage(
      { type: "functor-lang-set-source", source: this.lastSource, id: this.pushId },
      "*"
    );
  }

  #onMessage(event) {
    if (event.source !== this.iframe.contentWindow) return;
    const data = event.data;
    if (!data) return;
    if (data.type === "functor-lang-preview-ready") {
      // Idempotent: the one-shot ready and a hello-ack ready can both arrive for
      // one live session — honor only the first (reset() re-arms this on reload).
      if (this.previewReady) return;
      this.previewReady = true;
      // Flush edits made while the runtime was still starting.
      if (this.dirty) this.#post();
      else this.onLive();
    } else if (data.type === "functor-lang-set-source-result") {
      // A reply from the outgoing document (its WindowProxy survives the src
      // swap) must not overwrite the "loading…" status of the incoming one.
      if (!this.previewReady) return;
      // A result carrying an id that isn't the latest push's is stale — a
      // newer push is already in flight; its reply supersedes this one.
      if (data.id !== undefined && data.id !== this.pushId) return;
      this.onResult(data.ok, data.message);
    }
  }
}
