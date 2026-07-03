/** A canonical key name as reported by the runtime (`functor_runtime_common::Key`).
 * The `(string & {})` arm keeps newly-added keys usable before the SDK is updated. */
export type KeyName =
  | "A" | "B" | "C" | "D" | "E" | "F" | "G" | "H" | "I" | "J" | "K" | "L" | "M"
  | "N" | "O" | "P" | "Q" | "R" | "S" | "T" | "U" | "V" | "W" | "X" | "Y" | "Z"
  | "Up" | "Down" | "Left" | "Right" | "Space" | "Enter" | "Escape" | "Unknown"
  | (string & {});

/** Runtime-owned input snapshot (independent of the game model). */
export interface InputSnapshot {
  /** Keys currently held, by canonical name. */
  held_keys: KeyName[];
  /** Last known cursor position in window pixels. */
  mouse: { x: number; y: number };
}

/** Runtime state from `GET /state`. `input` is structured and game-agnostic;
 * `model` is the game model rendered with Rust's pretty-`Debug` (not structured
 * JSON), so reading fields from it is best-effort string matching. */
export interface RuntimeState {
  frame: number;
  tts: number;
  viewport: { width: number; height: number };
  input: InputSnapshot;
  model: string;
}

export type Vec3 = [number, number, number];

/** Camera block from `GET /scene`. */
export interface Camera {
  eye: Vec3;
  target: Vec3;
  up: Vec3;
  fov_radians: number;
  near: number;
  far: number;
}

/** The frame description from `GET /scene` (camera + scene + lights). The scene
 * and lights are passed through as-is for now. */
export interface Scene {
  camera: Camera;
  scene: unknown;
  lights: unknown;
}

/** An input event for `POST /input`, tagged by `type`. */
export type InputCommand =
  | { type: "key"; key: string; down: boolean }
  | { type: "mouse_move"; x: number; y: number }
  | { type: "mouse_wheel"; delta: number };

/** Options for polling helpers like `waitFor` / `waitForState`. */
export interface WaitForOptions {
  /** Total time to wait before giving up, ms (default 10_000). */
  timeoutMs?: number;
  /** Poll interval, ms (default 100). */
  intervalMs?: number;
  /** Phrase used in the timeout error message ("…waiting for <description>"). */
  description?: string;
}

/** Options for launching a `functor-runner` process. */
export interface LaunchOptions {
  /** Game directory containing `build-native/` (the runner's cwd, for assets).
   * e.g. an absolute path to `examples/hello`. */
  gameDir: string;
  /** Debug-runtime HTTP port (default 8077). */
  port?: number;
  /** Path to the `functor-runner` binary (default `<repoRoot>/target/debug/functor-runner`). */
  runnerBin?: string;
  /** Path to the game dylib (default `<gameDir>/build-native/target/debug/<libgame_native>`). */
  dylibPath?: string;
  /** Path to an `.mle` game source instead of a dylib: launches the runner
   * with `--mle` (the MLE interpreter — docs/mle.md Track C2/C3). Mutually
   * exclusive with `dylibPath`; `gameDir` stays the runner's cwd. */
  mlePath?: string;
  /** Cargo workspace root (default: walk up from `gameDir`). */
  repoRoot?: string;
  /** Max time to wait for the runtime to be ready, ms (default 60_000). */
  launchTimeoutMs?: number;
  /** Echo runtime stdout/stderr to this process's stderr (default false). */
  echoLogs?: boolean;
  /** Run the runtime with no GL window (`--headless`): no display needed, but
   * `capture()` is unavailable. Ideal for CI / headless machines. */
  headless?: boolean;
}
