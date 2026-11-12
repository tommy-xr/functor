// Generated from the Rust app-protocol types by ts-rs. Do not edit.
// Regenerate: UPDATE_TS=1 cargo test -p functor-app-core --features ts ts_bindings

export type AppState = { 
/**
 * Absolute path of the repo root.
 */
repo_root: string, 
/**
 * Sorted by `path`.
 */
projects: Array<Project>, 
/**
 * The active project's `path`; `None` only when there are no projects.
 */
active: string | null, 
/**
 * The active project's loadable `.fun`/`.funi` files and `functor.json`,
 * sorted ([`crate::projects::project_files`]).
 */
files: Array<string>, 
/**
 * The repo's checkout; `None` outside git.
 */
git: GitInfo | null, 
/**
 * The active project's game session; `None` until one first starts.
 */
session: Session | null, 
/**
 * Where sessions run; a change restarts a live one.
 */
session_mode: SessionMode, 
/**
 * Sessions run `--headless` (`functor-app --no-gpu`), so there are no frames.
 */
no_gpu: boolean, };

/**
 * UI → core.
 */
export type ClientMsg = { "type": "get_state" } | { "type": "set_active_project", path: string, } | { "type": "start_session" } | { "type": "stop_session" } | { "type": "set_session_mode", mode: SessionMode, } | { "type": "input", input: GameInput, };

/**
 * What `functor.json` declares to run.
 */
export type Entries = { "kind": "single", file: string, } | { "kind": "named", roles: Array<Role>, } | { "kind": "invalid", error: string, };

/**
 * Keyboard and mouse input in the debug server's `POST /input` shape.
 */
export type GameInput = { "type": "key", key: string, down: boolean, } | { "type": "mouse_move", x: number, y: number, } | { "type": "mouse_button", button: MouseButton, down: boolean, } | { "type": "mouse_wheel", delta: number, };

export type GitInfo = { 
/**
 * `HEAD` when detached.
 */
branch: string, 
/**
 * Abbreviated, as `git rev-parse --short` prints it.
 */
commit: string, };

export type MouseButton = "left" | "right" | "middle";

export type Project = { 
/**
 * The directory's name (`hello`).
 */
name: string, 
/**
 * Relative to the repo root, `/`-separated; `.` is the root itself.
 */
path: string, entries: Entries, };

/**
 * One `"entries"` role: `"client.fun"`, or `{ "file", "module" | "prefix" }`
 * for roles sharing a file.
 */
export type Role = { name: string, file: string, module?: string, prefix?: string, };

/**
 * Core → UI.
 */
export type ServerMsg = { "type": "state", state: AppState, };

export type Session = { 
/**
 * Unique per start; events from an older session's child are dropped.
 */
id: number, 
/**
 * The child's process id once spawned.
 */
pid: number | null, status: SessionStatus, };

/**
 * Where the session's game shows; the user picks it above the game view.
 */
export type SessionMode = "hidden" | "window";

export type SessionStatus = { "kind": "stopped" } | { "kind": "starting" } | { "kind": "running", debug_url: string, frame: number | null, 
/**
 * The game's pointer surface (`/state.input.mouse.surface_width`,
 * `surface_height`): the space `/input` mouse coordinates are in.
 */
surface: [number, number] | null, } | { "kind": "exited", reason: string, } | { "kind": "failed", reason: string, };
