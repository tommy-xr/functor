/** A canonical key name as reported by the runtime (`functor_runtime_common::Key`).
 * The `(string & {})` arm keeps newly-added keys usable before the SDK is updated. */
export type KeyName =
  | "A" | "B" | "C" | "D" | "E" | "F" | "G" | "H" | "I" | "J" | "K" | "L" | "M"
  | "N" | "O" | "P" | "Q" | "R" | "S" | "T" | "U" | "V" | "W" | "X" | "Y" | "Z"
  | "Up" | "Down" | "Left" | "Right" | "Space" | "Enter" | "Escape" | "Unknown"
  | "Num0" | "Num1" | "Num2" | "Num3" | "Num4"
  | "Num5" | "Num6" | "Num7" | "Num8" | "Num9"
  | (string & {});

export type Vec3 = [number, number, number];
export type Quaternion = [x: number, y: number, z: number, w: number];

/** A rig-local tracked pose: +X right, +Y up, -Z forward. */
export interface TrackingPose {
  position: Vec3;
  orientation: Quaternion;
}

/** One target-neutral tracked XR controller. */
export interface XrControllerSnapshot {
  active: boolean;
  grip: TrackingPose | null;
  aim: TrackingPose | null;
  trigger: number;
  squeeze: number;
  thumbstick: [x: number, y: number];
  primary_pressed: boolean;
  secondary_pressed: boolean;
  thumbstick_pressed: boolean;
  menu_pressed: boolean;
}

/** XR tracking and controllers sampled for one fixed simulation step. */
export interface XrInputSnapshot {
  head: TrackingPose | null;
  left: XrControllerSnapshot;
  right: XrControllerSnapshot;
}

/** Wire spelling of a mouse button for `POST /input`. */
export type MouseButtonName = "left" | "right" | "middle";

/** A fixed mouse-button set, used for held, pressed, and released fields. */
export interface MouseButtons {
  left: boolean;
  right: boolean;
  middle: boolean;
}

/** Runtime-owned input sampled independently of the game model.
 *
 * Typed device domains extend this record: XR is available today; gamepad and
 * mobile-touch snapshots can be added without replacing keyboard/mouse or
 * introducing target-specific clients. */
export interface InputSnapshot {
  /** Keys currently held, by canonical name. */
  held_keys: KeyName[];
  /** Keys pressed since the previous fixed simulation step.
   *
   * Absent on runtimes predating deterministic sampled edges. */
  pressed_keys?: KeyName[];
  /** Keys released since the previous fixed simulation step.
   *
   * A quick tap can place the same key in both edge arrays. Absent on older
   * runtimes. */
  released_keys?: KeyName[];
  /** Last known cursor position and logical surface extent in the same
   * top-left-origin coordinate space, plus held and edge sets.
   *
   * Desktop reports window points and web reports CSS pixels, independent of
   * Retina/device-pixel ratio. Surface and button/edge fields are absent from
   * older runtimes' `/state`; treat a missing button field as an empty set. */
  mouse: {
    x: number;
    y: number;
    surface_width?: number;
    surface_height?: number;
    buttons?: MouseButtons;
    pressed?: MouseButtons;
    released?: MouseButtons;
  };
  /** Present while an XR target has valid head tracking. */
  xr?: XrInputSnapshot;
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

/** A structured JSON view of a Functor Lang value: plain data maps structurally
 * (records as objects, lists as arrays, maps as canonical entry arrays), and
 * everything else is a sigil-keyed object no record field can collide with —
 * `{"$map": [[key, value], ...]}`,
 * `{"$tuple": [...]}`,
 * `{"$ctor": "Some", "args": [...]}`, `{"$fn": "<fn(dt)>"}`,
 * `{"$host": "SceneNode"}`, `{"$number": "NaN"}`, and
 * `{"$truncated": "max depth"}` past the nesting bound. */
export type ModelJson =
  | number
  | string
  | boolean
  | null
  | ModelJson[]
  | { [key: string]: ModelJson };

/** Runtime state from `GET /state`. `input` is structured and game-agnostic;
 * `model` is the structured, lossy JSON view of the game model — the default
 * thing to read fields from. */
export interface RuntimeState {
  frame: number;
  tts: number;
  /** Clock steps queued by `advance` that have not run yet. `0` means every
   * requested step has been simulated. */
  pending_steps: number;
  /** How many times the model has been REPLACED by game logic since load —
   * the version label for a NETWORKED model, since pausing pins the clock and
   * not the transport, so a paused game keeps folding inbound messages while
   * `frame` stands still. Protocol v10+; a pre-v10 runtime omits it. */
  model_revision?: number;
  /** Inbound network events the shell has accepted but not yet delivered to
   * the game. Poll until `0` for quiescence before snapshotting a baseline.
   * Protocol v10+; a pre-v10 runtime omits it, so gate on `protocol_version`
   * rather than reading a missing field as quiescent. */
  pending_net?: number;
  /** Combined/legacy output extent. Use `views` when view identity matters. */
  viewport: RuntimeViewport;
  views: RuntimeView[];
  input: InputSnapshot;
  /** Structured, lossy JSON view of the model (`null` for producers without
   * a structured model, e.g. replay). Protocol v4+ — a pre-v4 runtime sends
   * the Debug TEXT under this key instead; gate on `GET /`'s
   * `protocol_version` before reading it as data. */
  model: ModelJson;
  /** The Rust-`Debug` pretty-print of the model: the human-facing view (full
   * depth, construction order where `model` is lossy). Opaque text — don't
   * parse it. v4 and later — a pre-v4 runtime omits it (and sends the text
   * under `model` instead). */
  model_debug: string;
}

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
  | { type: "mouse_button"; button: MouseButtonName; down: boolean }
  | { type: "ui_event"; slot: number; kind: UiEventKind }
  | { type: "webview_event"; slot: number; kind: UiEventKind }
  | ({ type: "xr" } & XrInputSample)
  | { type: "xr_clear" };

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

export type UiEventKind =
  | "Clicked"
  | { SliderChanged: number }
  | { TextChanged: string };

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
