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
no_gpu: boolean, 
/**
 * Every project's tasks, by id; their transcripts travel separately.
 */
tasks: Array<Task>, 
/**
 * The agents this machine can run, in the order the picker lists them.
 */
harnesses: Array<Harness>, 
/**
 * Per project path, the agent last picked for its new tasks.
 */
harness_choice: { [key in string]: Harness }, };

/**
 * UI → core.
 */
export type ClientMsg = { "type": "get_state" } | { "type": "set_active_project", path: string, } | { "type": "start_session" } | { "type": "stop_session" } | { "type": "set_session_mode", mode: SessionMode, } | { "type": "input", input: GameInput, } | { "type": "new_task", message: string, harness: Harness, } | { "type": "send_message", task: number, message: string, } | { "type": "stop_task", task: number, } | { "type": "get_transcript", task: number, } | { "type": "set_harness", harness: Harness, };

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

export type Harness = "claude" | "codex";

/**
 * One transcript entry, in the order the UI shows them.
 */
export type Item = { "kind": "user", text: string, } | { "kind": "agent", text: string, partial: boolean, } | { "kind": "tool", name: string, input: JsonValue, result: string | null, } | { "kind": "usage", input: number, output: number, } | { "kind": "finished", ok: boolean, summary: string | null, } | { "kind": "stopped" };

export type JsonValue = number | string | boolean | Array<JsonValue> | { [key in string]: JsonValue } | null;

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

export type Run = { 
/**
 * The index of the user message that started it; events for another
 * run of the task are dropped.
 */
id: number, 
/**
 * The latest (cumulative) usage, added as an item when the run ends.
 */
usage: Usage | null, };

/**
 * Core → UI.
 */
export type ServerMsg = { "type": "state", state: AppState, } | { "type": "transcript", task: number, items: Array<Item>, } | { "type": "item", task: number, index: number, item: Item, } | { "type": "rejected", reason: string, };

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

export type Task = { 
/**
 * Unique among the loaded tasks.
 */
id: number, 
/**
 * The project's path relative to the repo root.
 */
project: string, 
/**
 * Short, from the first message.
 */
title: string, 
/**
 * From [`derive_status`], after every change.
 */
status: TaskStatus, harness: Harness, 
/**
 * As the agent reported it.
 */
model: string | null, 
/**
 * The agent's session, from its `started`; a follow-up resumes it.
 */
session_id: string | null, 
/**
 * The run in flight.
 */
run: Run | null, };

export type TaskStatus = { "kind": "in_progress" } | { "kind": "waiting_on_you" } | { "kind": "ready_for_review" } | { "kind": "failed", summary: string | null, };

export type Usage = { 
/**
 * Includes cached prompt tokens.
 */
input: number, output: number, };
