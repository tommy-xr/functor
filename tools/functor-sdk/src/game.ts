import type { HttpClient } from "./client.js";
import type {
  InputCommand,
  KeyName,
  ProjectAssets,
  ProjectSources,
  RuntimeState,
  Scene,
  WaitForOptions,
  XrInputSnapshot,
} from "./types.js";

/** Default per-step delta time (seconds), ~one frame at 60 Hz. */
export const DEFAULT_STEP_DT = 1 / 60;

/** Canonicalize a key name to the runtime's form (e.g. "up" -> "Up", "w" -> "W").
 * Every canonical Key name is a single word with only its first letter capitalized. */
function canonicalKeyName(key: string): string {
  return key.length === 0
    ? key
    : key[0].toUpperCase() + key.slice(1).toLowerCase();
}

/** A high-level client for a single running game (one debug port).
 *
 * Two ways to use it: **observe** (`state`, `scene`, `capture`) a game that's
 * running on the wall clock, or **drive** it deterministically by pinning the
 * clock (`pause`), injecting input, and advancing a frame at a time (`step`). */
export class FunctorClient {
  constructor(protected readonly http: HttpClient) {}

  // --- Observe -------------------------------------------------------------

  /** Current runtime state: frame, game time, viewport, and model (Debug text). */
  state(): Promise<RuntimeState> {
    return this.http.getJson<RuntimeState>("/state");
  }

  /** Current frame description: camera + scene + lights. */
  scene(): Promise<Scene> {
    return this.http.getJson<Scene>("/scene");
  }

  /** Current paused-inspector trace JSON. Its nested value schema is open. */
  trace(): Promise<unknown> {
    return this.http.getJson<unknown>("/trace");
  }

  /** Keys the runtime currently considers held (structured, game-agnostic). */
  async heldKeys(): Promise<KeyName[]> {
    return (await this.state()).input.held_keys;
  }

  /** Whether a given key is currently held. Case-insensitive: accepts either the
   * canonical name ("Up") or the lowercase form taken by keyDown/keyUp ("up"). */
  async isKeyDown(key: KeyName): Promise<boolean> {
    const want = canonicalKeyName(key);
    return (await this.heldKeys()).some((k) => k === want);
  }

  /** Latest rig-local XR head/controller snapshot, or undefined on non-XR
   * targets and while XR tracking is unavailable. */
  async xrInput(): Promise<XrInputSnapshot | undefined> {
    return (await this.state()).input.xr;
  }

  /** Capture the next rendered frame as PNG bytes. */
  capture(): Promise<Buffer> {
    return this.http.postBinary("/capture");
  }

  // --- Program lifecycle --------------------------------------------------

  /** Hot-reload one raw Functor Lang entry source, preserving the model. */
  reloadSource(source: string): Promise<string> {
    return this.http.postRawText("/reload-source", source);
  }

  /** Hot-reload a complete sibling-module project, preserving the model. */
  reloadProject(files: ProjectSources): Promise<string> {
    return this.http.postText("/reload-project", files);
  }

  /** Load a complete sibling-module project as a new game, initializing its
   * model from `init`. Use reloadProject for subsequent live edits. */
  loadProject(files: ProjectSources): Promise<string> {
    return this.http.postText("/load-project", files);
  }

  /** Upload one project-relative texture/model/audio asset. Existing decoded
   * render data for the locator is evicted when the bytes changed. */
  reloadAsset(path: string, bytes: Uint8Array): Promise<string> {
    const pathBytes = new TextEncoder().encode(path);
    if (pathBytes.byteLength > 0xffff_ffff) {
      throw new Error("asset path is too long");
    }
    const body = new Uint8Array(4 + pathBytes.byteLength + bytes.byteLength);
    new DataView(body.buffer).setUint32(0, pathBytes.byteLength, false);
    body.set(pathBytes, 4);
    body.set(bytes, 4 + pathBytes.byteLength);
    return this.http.postRawBinary("/reload-asset", body);
  }

  /** Upload the current asset set, then remove uploads absent from its
   * manifest. Files transfer individually so large projects stay bounded by
   * their largest asset rather than total project size. */
  async reloadAssets(files: ProjectAssets): Promise<string> {
    for (const [path, bytes] of files) {
      await this.reloadAsset(path, bytes);
    }
    return this.http.postText(
      "/sync-assets",
      files.map(([path]) => path),
    );
  }

  /** Restore the recorded model and physics state at a rendered frame. */
  rewind(frame: number): Promise<string> {
    return this.http.postText("/rewind", { frame });
  }

  // --- Drive: input --------------------------------------------------------

  /** Inject a raw input command. */
  async input(cmd: InputCommand): Promise<void> {
    await this.http.postText("/input", cmd);
  }

  /** Press or release a key (e.g. "w", "up", "space"). */
  key(key: string, down: boolean): Promise<void> {
    return this.input({ type: "key", key, down });
  }

  keyDown(key: string): Promise<void> {
    return this.key(key, true);
  }

  keyUp(key: string): Promise<void> {
    return this.key(key, false);
  }

  /** Move the mouse cursor to an absolute position. */
  mouseMove(x: number, y: number): Promise<void> {
    return this.input({ type: "mouse_move", x, y });
  }

  /** Scroll the mouse wheel. */
  mouseWheel(delta: number): Promise<void> {
    return this.input({ type: "mouse_wheel", delta });
  }

  // --- Drive: clock --------------------------------------------------------

  /** Pin the game clock so it stops advancing (a "pause"). With no argument,
   * pins at the current game time; pass `tts` to pin at a specific time. */
  async pause(tts?: number): Promise<void> {
    const at = tts ?? (await this.state()).tts;
    await this.http.postText("/time", { type: "set", tts: at });
  }

  /** Advance the clock by one step (default ~one 60 Hz frame), then hold. */
  async step(dts: number = DEFAULT_STEP_DT): Promise<void> {
    await this.http.postText("/time", { type: "advance", dts });
  }

  /** Advance `n` steps, one frame at a time. */
  async stepFrames(n: number, dts: number = DEFAULT_STEP_DT): Promise<void> {
    for (let i = 0; i < n; i++) {
      await this.step(dts);
    }
  }

  /** Resume following the wall clock. */
  async resume(): Promise<void> {
    await this.http.postText("/time", { type: "resume" });
  }

  /** Poll `state()` until `predicate` holds, or throw on timeout. Useful for
   * async conditions like network convergence. */
  waitForState(
    predicate: (state: RuntimeState) => boolean,
    opts?: WaitForOptions,
  ): Promise<RuntimeState> {
    return waitFor(() => this.state(), predicate, opts);
  }
}

/** Poll `poll()` until `predicate(value)` is true, then return that value;
 * throw if it doesn't happen within the timeout. A throwing `poll()` is treated
 * as "not ready yet" and retried (e.g. a transient `/state` hiccup mid-
 * convergence), with the last error surfaced if the deadline passes. */
export async function waitFor<T>(
  poll: () => Promise<T>,
  predicate: (value: T) => boolean,
  opts: WaitForOptions = {},
): Promise<T> {
  const timeoutMs = opts.timeoutMs ?? 10_000;
  const intervalMs = opts.intervalMs ?? 100;
  const deadline = Date.now() + timeoutMs;
  let lastError: unknown;
  for (;;) {
    try {
      const value = await poll();
      lastError = undefined;
      if (predicate(value)) return value;
    } catch (err) {
      lastError = err;
    }
    const remaining = deadline - Date.now();
    if (remaining <= 0) {
      const what = opts.description ? ` waiting for ${opts.description}` : "";
      const cause = lastError === undefined ? "" : ` (last error: ${lastError})`;
      throw new Error(`waitFor timed out after ${timeoutMs}ms${what}${cause}`);
    }
    await new Promise((r) => setTimeout(r, Math.min(intervalMs, remaining)));
  }
}

/** Advance several clients by one lockstep frame, concurrently.
 *
 * The building block for **multiplayer simulation**: pin every client's clock,
 * then `stepAll` them by the same dt each tick so their simulations stay in
 * sync (the out-of-process analogue of the in-process `functor-netsim`
 * harness).
 *
 * Rejects (via `Promise.all`) if any client's step fails — but the others may
 * already have advanced, so a rejection means the simulation is desynced with
 * no automatic rollback; treat it as terminal for the run. */
export function stepAll(
  clients: readonly Pick<FunctorClient, "step">[],
  dts: number = DEFAULT_STEP_DT,
): Promise<void[]> {
  return Promise.all(clients.map((c) => c.step(dts)));
}
