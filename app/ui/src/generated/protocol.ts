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
 * The active project's game session; `None` until one first starts.
 */
session: Session | null, };

/**
 * UI → core.
 */
export type ClientMsg = { "type": "get_state" } | { "type": "set_active_project", path: string, } | { "type": "start_session" } | { "type": "stop_session" };

/**
 * What `functor.json` declares to run.
 */
export type Entries = { "kind": "single", file: string, } | { "kind": "named", roles: Array<Role>, } | { "kind": "invalid", error: string, };

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

export type SessionStatus = { "kind": "stopped" } | { "kind": "starting" } | { "kind": "running", debug_url: string, frame: number | null, } | { "kind": "exited", reason: string, } | { "kind": "failed", reason: string, };
