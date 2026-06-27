import type { HttpClient } from "./client.js";
import type { InputCommand, RuntimeState, Scene } from "./types.js";

/** Default per-step delta time (seconds), ~one frame at 60 Hz. */
export const DEFAULT_STEP_DT = 1 / 60;

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

  /** Capture the next rendered frame as PNG bytes. */
  capture(): Promise<Buffer> {
    return this.http.postBinary("/capture");
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
