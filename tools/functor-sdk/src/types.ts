// The debug-protocol wire types are generated from the Rust structs
// (`generated/protocol.ts`). This file re-exports them, narrows a few, and
// holds the SDK-only option types.

import type {
  Camera,
  GamepadSnapshot,
  InputCommand as WireInputCommand,
  InputSnapshot as WireInputSnapshot,
  JsonValue,
  Key,
  RuntimeState as WireRuntimeState,
  TouchPhase,
  TrackingPose,
  XrControllerSnapshot,
} from "./generated/protocol.js";

export type * from "./generated/protocol.js";

/** A canonical key name as reported by the runtime (`Key`). The
 * `(string & {})` arm keeps newly-added keys usable before the SDK is updated. */
export type KeyName = Key | (string & {});

/** The wire `InputSnapshot` with its key arrays widened to {@link KeyName}. */
export type InputSnapshot = Omit<
  WireInputSnapshot,
  "held_keys" | "pressed_keys" | "released_keys"
> & {
  held_keys: KeyName[];
  pressed_keys: KeyName[];
  released_keys: KeyName[];
};

/** Runtime state from `GET /state`, carrying the widened {@link InputSnapshot}. */
export type RuntimeState = Omit<WireRuntimeState, "input"> & {
  input: InputSnapshot;
};

export type Vec3 = [number, number, number];
export type Quaternion = [x: number, y: number, z: number, w: number];

/** Wire spelling of a touch transition phase for `POST /input`. */
export type TouchPhaseName = TouchPhase;

/** Wire spelling of a mouse button for `POST /input`. */
export type MouseButtonName = "left" | "right" | "middle";

/** A structured JSON view of a Functor Lang value: plain data maps structurally
 * (records as objects, lists as arrays, maps as canonical entry arrays), and
 * everything else is a sigil-keyed object no record field can collide with —
 * `{"$map": [[key, value], ...]}`,
 * `{"$tuple": [...]}`,
 * `{"$ctor": "Some", "args": [...]}`, `{"$fn": "<fn(dt)>"}`,
 * `{"$host": "SceneNode"}`, `{"$number": "NaN"}`, and
 * `{"$truncated": "max depth"}` past the nesting bound. */
export type ModelJson = JsonValue;

/** The frame description from `GET /scene` (camera + scene + lights). The scene
 * and lights are passed through as-is for now. */
export interface Scene {
  camera: Camera;
  scene: unknown;
  lights: unknown;
}

/** An input event for `POST /input`, tagged by `type`. Narrows the wire type:
 * `button` is a known name, and `xr`/`gamepad` take partial samples (the
 * runtime defaults omitted fields). */
export type InputCommand =
  | Exclude<WireInputCommand, { type: "mouse_button" | "xr" | "gamepad" }>
  | { type: "mouse_button"; button: MouseButtonName; down: boolean }
  | ({ type: "xr" } & XrInputSample)
  | ({ type: "gamepad" } & GamepadInputSample);

/** An injected gamepad sample for `POST /input` (desktop only) — the
 * {@link XrInputSample} contract for the pad domain: level state until
 * replaced, a whole-sample replacement whose omitted fields take their
 * defaults (centered sticks, released triggers/buttons). */
export type GamepadInputSample = Partial<GamepadSnapshot>;

/** An injected XR sample for `POST /input` (desktop only).
 *
 * Level state, not an edge event: it stays in force until replaced, and every
 * following fixed step feeds it to `sampledInput`. A WHOLE-sample replacement —
 * an omitted field takes its default (hand inactive, no pose, `0.0`, an identity
 * orientation) — so send both hands each step rather than relying on a merge. */
export interface XrInputSample {
  head?: Partial<TrackingPose> | null;
  left?: XrControllerSample;
  right?: XrControllerSample;
}

/** One injected controller. See {@link XrInputSample} for the defaulting rule. */
export type XrControllerSample = Partial<
  Omit<XrControllerSnapshot, "grip" | "aim">
> & {
  grip?: Partial<TrackingPose> | null;
  aim?: Partial<TrackingPose> | null;
};

/** Whole-project wire body for `POST /reload-project`, entry file first. */
export type ProjectSources = Array<[path: string, source: string]>;

/** Project-relative texture/model/audio bytes for the runtime asset cache. */
export type ProjectAssets = Array<[path: string, bytes: Uint8Array]>;

/** Options for polling helpers like `waitFor` / `waitForState`. */
export interface WaitForOptions {
  /** Total time to wait before giving up, ms (default 10_000). */
  timeoutMs?: number;
  /** Poll interval, ms (default 100). */
  intervalMs?: number;
  /** Phrase used in the timeout error message ("…waiting for <description>"). */
  description?: string;
}

/** Options for deterministic {@link FunctorClient.stepUntil} polling. */
export interface StepUntilOptions {
  /** Maximum fixed steps before failing (default 600, maximum 10,000). */
  maxFrames?: number;
  /** Delta time for each fixed step (default 1/60 second). */
  dts?: number;
  /** Phrase used in the failure message ("…waiting for <description>"). */
  description?: string;
}

/** Options for launching a `functor` process (`functor run native`). */
export interface LaunchOptions {
  /** Game directory (the runner's cwd, for resolving assets).
   * e.g. an absolute path to `examples/hello`. */
  gameDir: string;
  /** Debug-runtime HTTP port. Default 0 = OS-assigned: the runtime binds a
   * free port and reports it on its `[debug-server] listening` line, which
   * launch parses — read the actual port from `runner.port`. Pass a fixed
   * port only when something external must find the server at a known
   * address. */
  port?: number;
  /** Path to the `functor` CLI binary (default `<repoRoot>/target/debug/functor`). */
  runnerBin?: string;
  /** Path to the `.fun` game source: launches the runner with `--functor-lang` (the Functor Lang
   * interpreter — docs/functor-lang.md Track C2/C3). `gameDir` stays the runner's cwd. */
  functorLangPath: string;
  /** Which ROLE of a multi-entry project (functor.json `entries`) to launch,
   * forwarded as the CLI's `--entry <name>`. Required when two roles share one
   * file (the `{ "file": …, "module": … }` form, as in `examples/orbs`), since
   * `functorLangPath` alone cannot say which of them you meant; optional for
   * roles-as-files, where the path picks the role. */
  entry?: string;
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
