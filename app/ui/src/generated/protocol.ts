// Generated from the Rust app-protocol types by ts-rs. Do not edit.
// Regenerate: UPDATE_TS=1 cargo test -p functor-app-core --features ts ts_bindings

export type AppState = { 
/**
 * Absolute path of the open folder's repo root (its nearest ancestor
 * with a `.git`, else the folder); empty when no folder is open.
 */
repo_root: string, 
/**
 * The folder the app opened, canonical; empty when none is.
 */
folder: string, 
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
 * The repo root is a git work tree, so tasks' changes are tracked
 * (snapshot, diff, keep, revert). Off outside git.
 */
tracks_changes: boolean, 
/**
 * The active project's game session; `None` until one first starts.
 * It runs as its project's game placement says (`Layout::game`).
 */
session: Session | null, 
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
harness_choice: { [key in string]: Harness }, 
/**
 * Per project path, its canvas layout; every project has one.
 */
layouts: { [key in string]: Layout }, 
/**
 * Recent projects, most recent first; kept for the app, not the folder.
 */
recents: Array<Recent>, 
/**
 * The window can show a native folder picker (`ClientMsg::PickFolder`).
 */
native_picker: boolean, 
/**
 * Why the last folder asked for did not open; cleared by the next try.
 */
open_error: string | null, };

/**
 * The summary that travels with the task.
 */
export type Change = { 
/**
 * In git's order (by path), at most [`MAX_FILES`].
 */
files: Array<FileChange>, 
/**
 * Changed files past [`MAX_FILES`], not listed.
 */
more: number, 
/**
 * The snapshot measured against the baseline: the same tree, the same
 * contents, so a diff fetched for it is still current.
 */
tree: string, };

/**
 * UI → core.
 */
export type ClientMsg = { "type": "get_state" } | { "type": "set_active_project", path: string, } | { "type": "start_session" } | { "type": "stop_session" } | { "type": "input", input: GameInput, } | { "type": "new_task", message: string, harness: Harness, } | { "type": "send_message", task: number, message: string, } | { "type": "stop_task", task: number, } | { "type": "get_transcript", task: number, } | { "type": "set_harness", harness: Harness, } | { "type": "get_diff", task: number, } | { "type": "keep_change", task: number, } | { "type": "revert_change", task: number, tree: string, } | { "type": "layout", op: LayoutOp, } | { "type": "get_file", path: string, } | { "type": "open_folder", path: string, } | { "type": "pick_folder" } | { "type": "list_dirs", path: string, } | { "type": "remove_recent", path: string, };

export type Corner = "top_left" | "top_right" | "bottom_left" | "bottom_right";

export type DiffLine = { kind: LineKind, 
/**
 * Without its `+`/`-`/` ` marker; a hunk header whole; a note without `\ `.
 */
text: string, };

/**
 * `Right`: the second child sits right of the first; `Down`: below it.
 */
export type Direction = "right" | "down";

/**
 * Where the Game tab was when it left its pane, so docking puts it back.
 */
export type Dock = { pane: number, index: number, };

/**
 * What `functor.json` declares to run.
 */
export type Entries = { "kind": "single", file: string, } | { "kind": "named", roles: Array<Role>, } | { "kind": "invalid", error: string, };

export type FileChange = { 
/**
 * Relative to the repo root; a rename's new path.
 */
path: string, 
/**
 * A rename's old path.
 */
from: string | null, added: number, removed: number, 
/**
 * Git counts no lines in it.
 */
binary: boolean, 
/**
 * More than [`FILE_LINES`] lines changed: summarised, not rendered.
 */
large: boolean, };

/**
 * One file of a full diff (`ServerMsg::Diff`).
 */
export type FileDiff = { file: FileChange, 
/**
 * From the first hunk header on; empty when `omitted`.
 */
lines: Array<DiffLine>, omitted: Omitted | null, };

/**
 * Keyboard and mouse input in the debug server's `POST /input` shape.
 */
export type GameInput = { "type": "key", key: string, down: boolean, } | { "type": "mouse_move", x: number, y: number, } | { "type": "mouse_button", button: MouseButton, down: boolean, } | { "type": "mouse_wheel", delta: number, };

export type GamePlacement = { placement: Placement, mini: Mini, 
/**
 * Set when the Game tab leaves a pane; taken when it comes back.
 */
dock: Dock | null, };

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
export type Item = { "kind": "user", text: string, } | { "kind": "agent", text: string, partial: boolean, } | { "kind": "tool", name: string, input: JsonValue, result: string | null, } | { "kind": "usage", input: number, output: number, } | { "kind": "finished", ok: boolean, summary: string | null, } | { "kind": "stopped" } | { "kind": "kept" } | { "kind": "reverted" };

export type JsonValue = number | string | boolean | Array<JsonValue> | { [key in string]: JsonValue } | null;

export type Layout = { 
/**
 * `None`: an empty canvas.
 */
root: Node | null, 
/**
 * The pane views open in by default; a pane of `root` exactly when
 * there is one.
 */
focus: number | null, 
/**
 * Where the game view shows. Absent from layouts saved before it
 * existed, which load as a pane.
 */
game: GamePlacement, };

/**
 * A change to the layout. Ops naming a pane or split that is gone (another
 * window changed the layout first) change nothing.
 */
export type LayoutOp = { "type": "open", view: View, target: Target, } | { "type": "move", view: View, pane: number, index: number, } | { "type": "close", view: View, } | { "type": "close_pane", pane: number, } | { "type": "split", pane: number, direction: Direction, } | { "type": "focus", pane: number, } | { "type": "resize", split: number, ratio: number, } | { "type": "reset" } | { "type": "place", placement: Placement, } | { "type": "set_mini", mini: Mini, };

export type LineKind = "hunk" | "context" | "added" | "removed" | "note";

/**
 * The mini view: the canvas corner it sits in and its width in CSS
 * pixels ([`MINI_MIN`]..=[`MINI_MAX`]); its height follows the game's
 * aspect ratio, and the page shrinks it to fit a smaller canvas.
 */
export type Mini = { corner: Corner, width: number, };

export type MouseButton = "left" | "right" | "middle";

export type Node = { "kind": "pane" } & Pane | { "kind": "split", id: number, direction: Direction, 
/**
 * The first child's share, in thousandths ([`RATIO_MIN`]..=[`RATIO_MAX`]).
 */
ratio: number, first: Node, second: Node, };

/**
 * Why a file's lines are not in the diff.
 */
export type Omitted = "binary" | "large" | "budget" | "unreadable";

export type Pane = { 
/**
 * Unique among the layout's panes and splits.
 */
id: number, 
/**
 * In tab-bar order. Empty only for a pane a split just made.
 */
tabs: Array<View>, 
/**
 * Index of the shown tab in `tabs` (0 when empty).
 */
active: number, };

/**
 * Where the project's one game view shows (D21).
 */
export type Placement = "pane" | "mini" | "window";

export type Project = { 
/**
 * The directory's name (`hello`).
 */
name: string, 
/**
 * Relative to the repo root, `/`-separated; `.` is the root itself.
 */
path: string, entries: Entries, };

export type Recent = { 
/**
 * The project directory's name.
 */
name: string, 
/**
 * Absolute and canonical: one entry per directory however it was opened.
 */
path: string, 
/**
 * Its directory could not be opened when last checked.
 */
missing: boolean, };

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
export type ServerMsg = { "type": "state", state: AppState, } | { "type": "transcript", task: number, items: Array<Item>, } | { "type": "item", task: number, index: number, item: Item, } | { "type": "diff", task: number, tree: string | null, files: Array<FileDiff>, more: number, error: string | null, } | { "type": "file", project: string, path: string, text: string | null, error: string | null, } | { "type": "dirs", path: string, dirs: Array<string>, error: string | null, } | { "type": "rejected", reason: string, };

export type Session = { 
/**
 * Unique per start; events from an older session's child are dropped.
 */
id: number, 
/**
 * The child's process id once spawned.
 */
pid: number | null, status: SessionStatus, };

export type SessionStatus = { "kind": "stopped" } | { "kind": "starting" } | { "kind": "running", debug_url: string, frame: number | null, 
/**
 * The game's pointer surface (`/state.input.mouse.surface_width`,
 * `surface_height`): the space `/input` mouse coordinates are in.
 */
surface: [number, number] | null, } | { "kind": "exited", reason: string, } | { "kind": "failed", reason: string, };

/**
 * Where [`LayoutOp::Open`] puts a view that is not open yet.
 */
export type Target = { "kind": "focused" } | { "kind": "side" } | { "kind": "pane", pane: number, };

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
run: Run | null, 
/**
 * The snapshot (a git tree in the app's store) the task's change is
 * measured from: taken as its first run, or the first after a keep or
 * revert, started. `None` outside git.
 */
baseline: string | null, 
/**
 * What changed since `baseline`, as last computed.
 */
change: Change | null, 
/**
 * A revert is being written; no run or second revert may start.
 */
reverting: boolean, };

export type TaskStatus = { "kind": "in_progress" } | { "kind": "waiting_on_you" } | { "kind": "ready_for_review" } | { "kind": "failed", summary: string | null, } | { "kind": "kept" } | { "kind": "reverted" };

export type Usage = { 
/**
 * Includes cached prompt tokens.
 */
input: number, output: number, };

/**
 * What a tab shows. New view types need no layout work.
 */
export type View = { "kind": "game" } | { "kind": "file", path: string, } | { "kind": "changes", task: number, } | { "kind": "task", task: number, };
