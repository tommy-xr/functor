// Generated from the Rust app-protocol types by ts-rs. Do not edit.
// Regenerate: UPDATE_TS=1 cargo test -p functor-app-core --features ts ts_bindings

export type Activity = { stance: Stance, 
/**
 * The main agent's calls still waiting on their results, oldest first.
 */
calls: Array<Call>, 
/**
 * The last run's error or refusal, one short line; cleared by the
 * user's next message.
 */
error: string | null, 
/**
 * The subagents running, oldest first: while the call that started
 * them is out, or (one started in the background, whose call returns
 * at once; its end is not reported) while a call of its own is.
 */
subagents: Array<Subagent>, 
/**
 * How full the main agent's context is, once both its use and its
 * model's window are known.
 */
context: ContextFill | null, };

/**
 * The agent's part of a layout (D25): kept with it, so it survives restarts.
 */
export type AgentLayout = { 
/**
 * "Let the agent arrange views". Off, its view requests change nothing.
 */
arranges: boolean, 
/**
 * Open tabs the agent opened: marked, and closed by "Close agent tabs".
 */
tabs: Array<View>, 
/**
 * The layout before the first change of the agent's latest run that
 * changed it: "Restore my layout" returns to it. Dropped once the
 * layout is that again.
 */
restore: Layout | null, 
/**
 * That run, as `[task, run]`: its later changes keep the snapshot.
 */
restore_run: [number, number] | null, };

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
 * This app turns speech into text, so the composer has a mic.
 */
speech: boolean, 
/**
 * Every project's tasks, by id; their transcripts travel separately.
 */
tasks: Array<Task>, 
/**
 * Per task id, what its agent is doing (D34), refreshed after every event.
 */
activity: { [key in number]: Activity }, 
/**
 * Per task id whose change waits, its decision page's size and verdict
 * (D35), refreshed after every event.
 */
decisions: { [key in number]: Decision }, 
/**
 * Per project path with queued tasks or a reason a new one would
 * wait, its queue (D30), refreshed after every event.
 */
queues: { [key in string]: Queue }, 
/**
 * Decisions on where speech went that can still be moved (D29), with
 * their lines and moves, refreshed after every event.
 */
routes: Array<Routed>, 
/**
 * Per project path, the chat's current task (`None`: a new one), the
 * one the Tasks pane shows selected. Absent until a client first
 * chooses one for the project; not kept across restarts.
 */
current_task: { [key in string]: number | null }, 
/**
 * The agents this machine can run, in the order the picker lists them.
 */
harnesses: Array<Harness>, 
/**
 * Per project path, the agent last picked for its new tasks.
 */
harness_choice: { [key in string]: Harness }, 
/**
 * Per project path, how much of the user's own setup its runs read
 * (D39); absent: all of it.
 */
scope_choice: { [key in string]: Scope }, 
/**
 * Per project path, how its changes are reviewed (D31); absent: the
 * default.
 */
review_settings: { [key in string]: ReviewSetting }, 
/**
 * Project paths whose speech the model router places (D29; off by
 * default: it adds 2–3 s and sometimes times out to the fixed rule).
 */
model_routes: Array<string>, 
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
open_error: string | null, 
/**
 * What `functor init --list --json` lists, starters first.
 */
templates: Array<Template>, 
/**
 * Why there are no templates (no `functor`, or one too old to list them).
 */
templates_error: string | null, 
/**
 * What is wrong with the `functor` the app runs ([`CliProblem::message`]):
 * while set, nothing is started with it (D40). `None`: nothing found wrong.
 */
cli_problem: string | null, 
/**
 * The shell is checking `functor` again (Try again).
 */
cli_checking: boolean, 
/**
 * The new-project modal, while open.
 */
new_project: NewProject | null, 
/**
 * What the agent or a walkthrough step last pointed at (D25).
 */
highlight: Highlight | null, 
/**
 * Headsets, app-wide.
 */
devices: Devices, 
/**
 * The shell can ask a model where speech goes (D29); else the fixed rule decides.
 */
router: boolean, };

/**
 * A headset on a USB cable (task 2.4). Pairing over it needs no code: the
 * cable and the user's adb authorisation are the trust.
 */
export type Attached = { 
/**
 * adb's serial for it.
 */
serial: string, name: string, state: CableState, 
/**
 * Its pairing is being written.
 */
pairing: boolean, 
/**
 * "Pair over the cable" is offered ([`cable_pairable`]).
 */
pairable: boolean, };

/**
 * How a decision was made.
 */
export type By = "rule" | "model" | "you";

export type CableState = { "state": "unauthorized" } | { "state": "offline" } | { "state": "no_app" } | { "state": "not_debuggable" } | { "state": "ready", device: string | null, paired: HeadsetPairing, };

export type Call = { kind: CallKind, 
/**
 * Short: "reading weapons.fun", "running cargo test".
 */
label: string, };

/**
 * What a call touches.
 */
export type CallKind = "thinking" | "files" | "shell" | "network" | "game" | "agents" | "skills" | "ask";

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

export type Check = { 
/**
 * What was run, e.g. `functor test`.
 */
ran: string, 
/**
 * What came of it, e.g. "14 expects pass".
 */
result: string, ok: boolean, };

/**
 * UI → core.
 */
export type ClientMsg = { "type": "get_state" } | { "type": "set_active_project", path: string, } | { "type": "start_session" } | { "type": "check_cli" } | { "type": "stop_session" } | { "type": "input", input: GameInput, } | { "type": "transport", call: TransportCall, } | { "type": "watch_transport", on: boolean, } | { "type": "new_task", message: string, harness: Harness, scope?: Scope, } | { "type": "send_message", task: number, message: string, } | { "type": "stop_task", task: number, } | { "type": "move_queued", task: number, to: number, } | { "type": "start_queued", task: number, } | { "type": "move_utterance", id: number, to: Destination, } | { "type": "get_transcript", task: number, } | { "type": "set_harness", harness: Harness, } | { "type": "set_model_route", on: boolean, } | { "type": "set_scope", scope: Scope, } | { "type": "set_review", setting: ReviewSetting, } | { "type": "get_diff", task: number, } | { "type": "keep_change", task: number, } | { "type": "revert_change", task: number, tree: string, } | { "type": "layout", op: LayoutOp, } | { "type": "get_file", path: string, } | { "type": "open_folder", path: string, } | { "type": "pick_folder" } | { "type": "show_new_project", here: boolean, } | { "type": "close_new_project" } | { "type": "check_new_project", request: NewProjectRequest, } | { "type": "pick_parent" } | { "type": "create_project", request: NewProjectRequest, harness: Harness, } | { "type": "list_dirs", path: string, } | { "type": "remove_recent", path: string, } | { "type": "walkthrough_step", task: number, item: number, step: number, } | { "type": "clear_highlight", seq: number, } | { "type": "canvas_stacked", stacked: boolean, } | { "type": "select_task", task: number | null, if_unset: boolean, } | { "type": "rename_task", task: number, title: string, } | { "type": "delete_task", task: number, } | { "type": "allow_pairing", request: number, } | { "type": "deny_pairing", request: number, } | { "type": "forget_device", device: string, } | { "type": "pair_by_cable", serial: string, replace: boolean, };

/**
 * Tokens of context in use, of the model's `window`.
 */
export type ContextFill = { used: number, window: number, };

export type Corner = "top_left" | "top_right" | "bottom_left" | "bottom_right";

/**
 * A waiting change's first page, as far as the app derives it.
 */
export type Decision = { size: Size, depth: Depth, 
/**
 * One or two short sentences.
 */
reason: string, 
/**
 * The report's summary against the rules [`style_flags`] counts.
 */
style: Array<string>, };

export type Depth = "glance" | "look";

/**
 * A place an utterance can be moved to.
 */
export type Destination = { "kind": "steer", task: number, } | { "kind": "new" } | { "kind": "queued", task: number, };

export type Device = { 
/**
 * The device's own random id, hex.
 */
id: string, name: string, 
/**
 * A pairing is stored for it.
 */
paired: boolean, state: DeviceState, 
/**
 * The headset app's version, as it last said over its link.
 */
app_version: string | null, 
/**
 * Wall-clock ms, from its links.
 */
first_seen: number | null, last_seen: number | null, 
/**
 * Its debug runtime through the link: `http://127.0.0.1:<port>`, kept
 * while paired (it answers 503 while the device is away).
 */
tunnel: string | null, };

export type DeviceState = { "state": "away" } | { "state": "connected", link: LinkKind, 
/**
 * Percent.
 */
battery: number | null, charging: boolean, 
/**
 * The runtime build it runs.
 */
build: string | null, 
/**
 * The project it runs.
 */
project: string | null, 
/**
 * The last ping's round trip.
 */
latency_ms: number | null, } | { "state": "needs_update", update: Side, reason: string, since: number, } | { "state": "cannot_reach", reason: string, since: number, };

export type Devices = { 
/**
 * Where the device link listens (`host:port`); `None` when it is off.
 */
listening: string | null, 
/**
 * Paired devices, and unpaired ones whose app is too old to pair, by
 * name then id.
 */
list: Array<Device>, 
/**
 * Devices asking to pair, oldest first: the prompt shows one at a time.
 */
requests: Array<PairRequest>, 
/**
 * The title bar's one word for them all.
 */
headset: Headset, 
/**
 * `adb` runs here, so headsets on a USB cable are seen.
 */
adb: boolean, 
/**
 * Headsets on a USB cable, as adb lists them.
 */
attached: Array<Attached>, };

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

/**
 * A task's path evidence, for the change measured as `tree`.
 */
export type Evidence = { 
/**
 * The change's snapshot the new path runs (`Change::tree`).
 */
tree: string, state: EvidenceState, };

export type EvidenceState = { "kind": "taking" } | { "kind": "shown", before: string, after: string, } | { "kind": "same" } | { "kind": "withheld", reason: string, };

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
 * A window's place and size in points, its top left from the main screen's.
 */
export type Frame = { x: number, y: number, width: number, height: number, };

/**
 * Keyboard and mouse input in the debug server's `POST /input` shape.
 */
export type GameInput = { "type": "key", key: string, down: boolean, } | { "type": "mouse_move", x: number, y: number, } | { "type": "mouse_button", button: MouseButton, down: boolean, } | { "type": "mouse_wheel", delta: number, };

export type GamePlacement = { placement: Placement, mini: Mini, 
/**
 * Set when the Game tab leaves a pane; taken when it comes back.
 */
dock: Dock | null, 
/**
 * The paired headset's id while placed on one; kept while its link is
 * down. Absent from older layouts.
 */
headset: string | null, };

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
 * The state word for all devices: the one that most needs the user.
 */
export type Headset = "not_paired" | "asking_to_pair" | "away" | "connected" | "needs_update" | "cannot_reach";

/**
 * The pairing a headset's app holds, as its file says.
 */
export type HeadsetPairing = { "with": "nobody" } | { "with": "this" } | { "with": "other", name: string, };

/**
 * What the agent or a walkthrough step last pointed at, until the user
 * scrolls it out of view or its tab stops being shown.
 */
export type Highlight = { project: string, view: View, 
/**
 * A file's lines, `[first, last]`, 1-based.
 */
lines: [number, number] | null, 
/**
 * A changes view's file, relative to the repo root.
 */
file: string | null, 
/**
 * New per highlight, so showing the same again scrolls again.
 */
seq: number, };

/**
 * One transcript entry, in the order the UI shows them.
 */
export type Item = { "kind": "user", text: string, 
/**
 * Said aloud in a headset, not typed.
 */
spoken?: Spoken, } | { "kind": "agent", text: string, partial: boolean, } | { "kind": "tool", name: string, input: JsonValue, result: string | null, 
/**
 * The harness's id for the call, when it gives one.
 */
id?: string, 
/**
 * The call that started the subagent making this one.
 */
parent_id?: string, 
/**
 * Whether the result is an error, when the harness says.
 */
is_error?: boolean, } | { "kind": "usage", input: number, output: number, detail?: UsageDetail, } | { "kind": "finished", ok: boolean, summary: string | null, } | { "kind": "stopped" } | { "kind": "kept" } | { "kind": "reverted" } | { "kind": "walkthrough", steps: Array<WalkthroughStep>, current: number | null, };

export type JsonValue = number | string | boolean | Array<JsonValue> | { [key in string]: JsonValue } | null;

/**
 * What a changed file is, by its extension (and, for game logic, its place).
 */
export type Kind = "game_logic" | "assets" | "config" | "other";

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
game: GamePlacement, 
/**
 * What the agent arranged (D25). Absent from older layouts.
 */
agent: AgentLayout, 
/**
 * Panes in windows of their own, in the order they were popped out.
 * Absent from older layouts.
 */
windows: Array<PopOut>, };

/**
 * A change to the layout. Ops naming a pane or split that is gone (another
 * window changed the layout first) change nothing.
 */
export type LayoutOp = { "type": "open", view: View, target: Target, } | { "type": "move", view: View, pane: number, index: number, } | { "type": "close", view: View, } | { "type": "close_pane", pane: number, } | { "type": "split", pane: number, direction: Direction, } | { "type": "focus", pane: number, } | { "type": "resize", split: number, ratio: number, } | { "type": "reset" } | { "type": "place", placement: Placement, } | { "type": "place_on_headset", device: string, } | { "type": "set_mini", mini: Mini, } | { "type": "restore" } | { "type": "close_agent_tabs" } | { "type": "agent_arranges", on: boolean, } | { "type": "pop_out", pane: number, } | { "type": "dock", window: number, } | { "type": "set_frame", window: number, frame: Frame, };

export type LineKind = "hunk" | "context" | "added" | "removed" | "note";

export type LinkKind = "wifi" | "cable";

/**
 * An event on the timeline: an input edge or a hot reload, with the kind
 * and label the web timeline gives it.
 */
export type Mark = { id: number, frame: number, 
/**
 * `key-down`, `mouse-button-up`, `ui-input`, `reload-ok`, ….
 */
kind: string, label: string, };

/**
 * The mini view: the canvas corner it sits in and its width in CSS
 * pixels ([`MINI_MIN`]..=[`MINI_MAX`]); its height follows the game's
 * aspect ratio, and the page shrinks it to fit a smaller canvas.
 */
export type Mini = { corner: Corner, width: number, };

/**
 * A hidden `functor run native --mirror <tunnel>` drawing the headset's
 * frames, streamed to the Game tab while it shows.
 */
export type Mirror = { "kind": "starting" } | { "kind": "running", debug_url: string, } | { "kind": "failed", reason: string, };

/**
 * The game as the task's baseline was taken: where both paths start.
 */
export type Moment = { "kind": "taken", frame: number, steps: number, } | { "kind": "missing", reason: string, };

export type MouseButton = "left" | "right" | "middle";

/**
 * A one-tap move of a decision.
 */
export type Move = { to: Destination, 
/**
 * "Make it a new task", "Steer “…” instead".
 */
label: string, };

/**
 * The new-project modal, while it is open.
 */
export type NewProject = { 
/**
 * The name and parent folder the modal starts with ("New project here"
 * fills both; a picked folder replaces `parent`).
 */
name: string, parent: string, step: Step, };

/**
 * What the modal asks for.
 */
export type NewProjectRequest = { name: string, 
/**
 * Where the project's folder goes: absolute, or `~/…`.
 */
parent: string, template: string, 
/**
 * Run `git init` when the folder is not in a git work tree already.
 */
git: boolean, 
/**
 * The first task's message; blank for none.
 */
describe: string, };

export type Node = { "kind": "pane" } & Pane | { "kind": "split", id: number, direction: Direction, 
/**
 * The first child's share, in thousandths ([`RATIO_MIN`]..=[`RATIO_MAX`]).
 */
ratio: number, first: Node, second: Node, };

/**
 * Why a file's lines are not in the diff.
 */
export type Omitted = "binary" | "large" | "budget" | "unreadable";

/**
 * A device asking to pair (unauthenticated: its id and name are claims).
 */
export type PairRequest = { request: number, 
/**
 * The id it claims, hex.
 */
device: string, name: string, app_version: string, 
/**
 * Both sides show it.
 */
code: string, 
/**
 * Wall-clock ms.
 */
expires_at: number, 
/**
 * The user allowed it; the headset's own confirmation is awaited.
 */
allowed: boolean, 
/**
 * It claims the id of a device already paired.
 */
repair: boolean, };

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
export type Placement = "pane" | "mini" | "window" | "headset";

/**
 * What creating the project will do, for the modal and for the shell.
 */
export type Plan = { 
/**
 * The project's folder.
 */
path: string, 
/**
 * It is inside a git work tree already (no `git init` is offered).
 */
in_git: boolean, 
/**
 * `git init` runs after the template's files are written.
 */
git_init: boolean, 
/**
 * The steps, in order, as the modal lists them.
 */
steps: Array<string>, };

/**
 * A pane the user popped out into its own window (D20): one pane of tabs,
 * named by its pane's id. Only the user opens and closes one; the agent
 * shows a view already in it there, and opens new ones in the main window.
 */
export type PopOut = { pane: Pane, 
/**
 * The main window's pane its tabs go back to when it closes (else the
 * focused one): the pane beside it when it was popped out.
 */
home: number | null, 
/**
 * Where the window was, once its shell reported it.
 */
frame: Frame | null, };

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
 * A project's queue, derived after every event.
 */
export type Queue = { 
/**
 * Its queued tasks, the next to start first.
 */
tasks: Array<number>, 
/**
 * Why a task cannot start in the project now; `None`: it would start at once.
 */
wait: Wait | null, 
/**
 * A task the user starts now would queue ([`holds`]).
 */
holds: boolean, };

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
 * The agent's account of its change (`present`), stored with the task; a
 * separate review's (`report`, D31) replaces it. A new run, a later edit to
 * the files or the end of the review clears it.
 */
export type Report = { 
/**
 * One paragraph in Simplified Technical English.
 */
summary: string | null, 
/**
 * What was not verified, what was assumed, side effects; `None`: the
 * agent did not say (unlike `Some([])`, "nothing to note").
 */
notes: Array<string> | null, 
/**
 * What was run, and the result.
 */
evidence: Array<Check>, 
/**
 * Written by a separate review, not the agent that made the change.
 */
reviewed: boolean, };

/**
 * A task's review of its waiting change, kept until the change is decided,
 * replaced or measured again differently.
 */
export type Review = { 
/**
 * The run whose change it reviews; its events carry it.
 */
run: number, harness: Harness, 
/**
 * As asked, until the agent reports its own.
 */
model: string | null, state: ReviewState, 
/**
 * The latest (cumulative) usage it reported.
 */
usage: Usage | null, 
/**
 * The latest usage split it reported, when the harness gives one.
 */
detail?: UsageDetail, };

/**
 * How a project's changes are reviewed; stored per project.
 */
export type ReviewSetting = { 
/**
 * Prepare the report in a separate run; off, the agent's own is shown.
 */
on: boolean, 
/**
 * `None`: the task's own.
 */
harness: Harness | null, 
/**
 * `None`: [`default_model`] for the harness.
 */
model: string | null, 
/**
 * Added to the review's instructions, e.g. a skill to follow.
 */
instructions: string | null, };

export type ReviewState = { "kind": "preparing" } | { "kind": "prepared" } | { "kind": "failed", reason: string, };

/**
 * One `"entries"` role: `"client.fun"`, or `{ "file", "module" | "prefix" }`
 * for roles sharing a file.
 */
export type Role = { name: string, file: string, module?: string, prefix?: string, };

/**
 * A decision still open to a move, derived after every event: the
 * message's place (task and transcript index), its line, and the moves.
 */
export type Routed = { id: number, task: number, item: number, 
/**
 * As shown on the message: "Steered “…”".
 */
line: string, by: By, moves: Array<Move>, };

export type Run = { 
/**
 * The index of the user message that started it; events for another
 * run of the task are dropped.
 */
id: number, 
/**
 * The latest (cumulative) usage, added as an item when the run ends.
 */
usage: Usage | null, 
/**
 * View requests the run made ([`crate::agent_view::MAX_VIEW_REQUESTS`]).
 */
views: number, 
/**
 * The latest usage split, when the harness gives one; added to the
 * usage item when the run ends.
 */
detail?: UsageDetail, 
/**
 * The agent reported its start, so it has the run's messages.
 */
started: boolean, };

/**
 * How much of the user's own setup an agent run reads (D39): instructions,
 * skills, memory and settings. The app's permissions apply at every level;
 * the allow rules and hooks in the user's settings come only with them.
 */
export type Scope = "user_and_project" | "project" | "isolated";

/**
 * Core → UI.
 */
export type ServerMsg = { "type": "state", state: AppState, } | { "type": "transcript", task: number, items: Array<Item>, } | { "type": "item", task: number, index: number, item: Item, } | { "type": "diff", task: number, tree: string | null, files: Array<FileDiff>, more: number, error: string | null, } | { "type": "file", root: string, project: string, path: string, text: string | null, error: string | null, } | { "type": "dirs", path: string, dirs: Array<string>, error: string | null, } | { "type": "new_project_plan", request: NewProjectRequest, plan: Plan | null, error: string | null, } | { "type": "rejected", reason: string, } | { "type": "transport", transport: Transport | null, } | { "type": "marks", marks: Array<Mark>, };

export type Session = { 
/**
 * Unique per start; events from an older session's child are dropped.
 */
id: number, 
/**
 * The child's process id once spawned.
 */
pid: number | null, status: SessionStatus, 
/**
 * A headset session's mirror on this computer (D5); `None` for a
 * local session, or without a GPU.
 */
mirror: Mirror | null, };

export type SessionStatus = { "kind": "stopped" } | { "kind": "starting" } | { "kind": "running", debug_url: string, 
/**
 * The game's pointer surface (`/state.input.mouse.surface_width`,
 * `surface_height`): the space `/input` mouse coordinates are in.
 */
surface: [number, number] | null, } | { "kind": "exited", reason: string, } | { "kind": "failed", reason: string, };

/**
 * Which app needs the update.
 */
export type Side = "headset" | "desktop";

/**
 * A change's size: lines over the listed files, files including those
 * past the list (whose kinds and lines are unknown).
 */
export type Size = { added: number, removed: number, files: number, 
/**
 * Each kind once, in [`Kind`]'s order.
 */
kinds: Array<Kind>, };

/**
 * Where a spoken message came from (plan 4.1/4.3): the headset, and what
 * the user was looking at when they spoke, as text.
 */
export type Spoken = { 
/**
 * The headset's name.
 */
headset: string, 
/**
 * The capsule, rendered: "at frame 1234 (20.5 s), head at …".
 */
context?: string, 
/**
 * Where the router put it (D29): "Steered “…”", "Queued behind “…”".
 */
route?: string, };

export type Stance = "thinking" | "working" | "waiting" | "done" | "failed";

/**
 * Where creating a project is.
 */
export type Step = { "kind": "editing" } | { "kind": "creating", path: string, } | { "kind": "opening", path: string, } | { "kind": "failed", reason: string, done: Array<string>, not_done: Array<string>, };

/**
 * A subagent at work, under the call that started it.
 */
export type Subagent = { 
/**
 * What it was asked to do: its call's description.
 */
label: string, 
/**
 * Its own calls still waiting on their results, oldest first.
 */
calls: Array<Call>, };

/**
 * Where [`LayoutOp::Open`] puts a view. One already open moves only to a
 * named `Pane`; otherwise it is shown where it is.
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
 * What its runs read of the user's setup (D39), fixed when it starts:
 * its follow-ups resume a conversation that holds the instructions it
 * read. Tasks saved
 * before it was kept read all of it.
 */
scope: Scope, 
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
 * Waiting its turn in its project's queue (D30): lower goes first. A
 * queued task has never run; its transcript is the user's messages.
 */
queued: number | null, 
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
reverting: boolean, 
/**
 * The game as `baseline` was taken (D38), when there was a game to ask.
 */
moment: Moment | null, 
/**
 * The predicted paths before and after `change` (D38).
 */
evidence: Evidence | null, 
/**
 * The last run's account of the change (D35), from its `present`, or
 * its review's (`Report::reviewed`), which replaces it.
 */
report: Report | null, 
/**
 * The separate run preparing the report on the waiting change (D31).
 */
review: Review | null, 
/**
 * When it was started: the shell's wall clock, ms since the Unix
 * epoch. Last active: a message, a run's event or a review. Both absent
 * from tasks saved before they were kept.
 */
created_at: number | null, 
/**
 * Last active, as `created_at`.
 */
updated_at: number | null, };

export type TaskStatus = { "kind": "queued" } | { "kind": "in_progress" } | { "kind": "waiting_on_you" } | { "kind": "ready_for_review" } | { "kind": "done" } | { "kind": "failed", summary: string | null, } | { "kind": "kept" } | { "kind": "reverted" };

/**
 * One of `functor init --list --json`'s entries.
 */
export type Template = { 
/**
 * What `functor init <name>` takes.
 */
name: string, description: string, tags: Array<string>, kind: TemplateKind, };

export type TemplateKind = "starter" | "example";

/**
 * Tokens split by how the prompt side was billed (`mis-agent`'s).
 */
export type TokenUsage = { 
/**
 * Prompt tokens neither read from nor written to the cache.
 */
input: number, cache_read: number, cache_creation: number, output: number, 
/**
 * The part of `output` spent reasoning, if the harness says.
 */
reasoning_output: number | null, };

/**
 * What the bar shows: `GET /state`'s `frame` and `clock`, with the bar's
 * seeks applied until the runtime has answered them.
 */
export type Transport = { 
/**
 * The runtime reports its clock (debug protocol v37): without it there
 * is a frame to show but nothing to drive.
 */
clock: boolean, 
/**
 * Pinned: paused, stepped, or parked on a seek.
 */
paused: boolean, 
/**
 * The recorded steps behind the state shown (`/state.frame`).
 */
frame: number, 
/**
 * The seekable window `[oldest, newest]`, numbered as `frame`; `None`
 * until a step is recorded.
 */
recorded: [number, number] | null, 
/**
 * Bumps when recorded frames are replaced: a step or resume from a
 * seeked frame branches, and so may a hot reload.
 */
generation: number, 
/**
 * The `id` of the last seek asked for (0: none), so the page that asked
 * stops waiting for it.
 */
seek: number, 
/**
 * The bar's Extrapolate switch (D38). A peek leaves it as it is.
 */
extrapolate: boolean, 
/**
 * Successful hot reloads since the program loaded: a new one is an
 * edit landing, which the page peeks at while paused.
 */
revision: number, };

/**
 * What the bar asks of the session.
 */
export type TransportCall = { "type": "pause" } | { "type": "resume" } | { "type": "step" } | { "type": "seek", frame: number, id: number, } | { "type": "extrapolate", on: boolean, } | { "type": "peek", on: boolean, };

export type Usage = { 
/**
 * Includes cached prompt tokens.
 */
input: number, output: number, };

/**
 * A run's usage as the harness splits it. Claude sends one after each API
 * message, with the end-of-run fields last; Codex sends `total` at the end.
 */
export type UsageDetail = { 
/**
 * Cumulative for the run, subagents included.
 */
total: TokenUsage, 
/**
 * The main agent's latest API message: its `prompt()` is how full the
 * context is now.
 */
last: TokenUsage | null, 
/**
 * Claude Code's `num_turns`: not exactly its API requests.
 */
turns: number | null, cost_usd: Usd | null, 
/**
 * Time spent in API calls.
 */
api_ms: number | null, 
/**
 * The main model's context window, in tokens.
 */
context_window: number | null, };

/**
 * US dollars. Read from JSON, which has no NaN, so equality is total.
 */
export type Usd = number;

/**
 * What a tab shows. New view types need no layout work.
 */
export type View = { "kind": "game" } | { "kind": "file", path: string, } | { "kind": "changes", task: number, } | { "kind": "task", task: number, };

/**
 * What a view request or walkthrough step shows: one of the run's
 * project's files (`lines` `[first, last]`, 1-based), its task's changes
 * (scrolled to `path`), or the game. Paths are relative to the project.
 */
export type ViewTarget = { "kind": "file", path: string, lines: [number, number] | null, } | { "kind": "changes", path: string | null, } | { "kind": "game" };

/**
 * Why a project's next task cannot start now.
 */
export type Wait = { "kind": "running", task: number, } | { "kind": "reverting", task: number, } | { "kind": "decision", task: number, } | { "kind": "measuring", task: number, };

export type WalkthroughStep = { title: string, note: string | null, view: ViewTarget, };
