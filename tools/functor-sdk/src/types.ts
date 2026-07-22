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

export interface RuntimeViewport {
  width: number;
  height: number;
}

/** One render view. Desktop reports `main`; stereo XR reports `left` and
 * `right`. Names are intentionally open-ended for future runtime targets. */
export interface RuntimeView {
  name: string;
  viewport: RuntimeViewport;
}

/** Runtime state from `GET /state`. `input` is structured and game-agnostic;
 * `model` is the game model rendered with Rust's pretty-`Debug` (not structured
 * JSON), so reading fields from it is best-effort string matching. */
export interface RuntimeState {
  frame: number;
  tts: number;
  /** Combined/legacy output extent. Use `views` when view identity matters. */
  viewport: RuntimeViewport;
  views: RuntimeView[];
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
  | { type: "mouse_wheel"; delta: number }
  | { type: "ui_event"; slot: number; kind: UiEventKind }
  | { type: "webview_event"; slot: number; kind: UiEventKind };

export type UiEventKind =
  | "Clicked"
  | { SliderChanged: number }
  | { TextChanged: string };

/** Whole-project wire body for `POST /reload-project`, entry file first. */
export type ProjectSources = Array<[path: string, source: string]>;

/** Options for polling helpers like `waitFor` / `waitForState`. */
export interface WaitForOptions {
  /** Total time to wait before giving up, ms (default 10_000). */
  timeoutMs?: number;
  /** Poll interval, ms (default 100). */
  intervalMs?: number;
  /** Phrase used in the timeout error message ("…waiting for <description>"). */
  description?: string;
}

/** Options for launching a `functor` process (`functor run native`). */
export interface LaunchOptions {
  /** Game directory (the runner's cwd, for resolving assets).
   * e.g. an absolute path to `examples/hello`. */
  gameDir: string;
  /** Debug-runtime HTTP port (default 8077). */
  port?: number;
  /** Path to the `functor` CLI binary (default `<repoRoot>/target/debug/functor`). */
  runnerBin?: string;
  /** Path to the `.fun` game source: launches the runner with `--functor-lang` (the Functor Lang
   * interpreter — docs/functor-lang.md Track C2/C3). `gameDir` stays the runner's cwd. */
  functorLangPath: string;
  /** Cargo workspace root (default: walk up from `gameDir`). */
  repoRoot?: string;
  /** Max time to wait for the runtime to be ready, ms (default 60_000). */
  launchTimeoutMs?: number;
  /** Echo runtime stdout/stderr to this process's stderr (default false). */
  echoLogs?: boolean;
  /** Run the runtime with no GL window (`--headless`): no display needed, but
   * `capture()` is unavailable. Ideal for CI / headless machines. */
  headless?: boolean;
  /** Show the GL window. By default (and unless `headless`), the runner is
   * launched with `--hidden`: the window is never shown and never steals focus
   * or the cursor, but keeps its GL context so `capture()` works. Pass `true`
   * to watch the game while a script drives it. */
  visible?: boolean;
}
