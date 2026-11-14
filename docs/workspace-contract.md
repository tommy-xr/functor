# The workspace contract (S9)

Status: **proposal, v0** — a document to review before anything is built on it.
Nothing here is implemented. Sources are named by file and type; "unsure" marks
what was not confirmed by reading the code.

A **workspace** is any surface a person works from: the desktop page, the
headset panel, and (D44) a workshop scene written in Functor Lang. The
contract is one versioned schema, owned by the app core, of the **state** a
workspace may read and the **actions** it may request. The rules — confirmation,
consent, scope, limits — are enforced where an action lands, never in a client.

Read with `docs/ideas/ai/desktop-app-revisions-v1.md` (D12, D25, D27–D45) and
`docs/debug-runtime.md`. D41–D45 are in open PRs (#1105–#1107) at the time of
writing; D42 (`list_projects` / `open_project`) was not found on main or in an
open PR and is taken from the spike's brief.

## 1. Inventory: three contracts today

| | Contract | Where | Direction | Encoding | Version |
| --- | --- | --- | --- | --- | --- |
| **P** | The page | `app/functor-app-core`: `AppState` (`state.rs`), `ClientMsg` / `ServerMsg` (`protocol.rs`); generated `app/ui/src/generated/protocol.ts` | both, over `/ws` | JSON | none (page and core ship in one binary; `UI_STALE` warns) |
| **H** | The headset | `runtime/functor-device-link`: `panel.rs` (channels 3, 4), `voice.rs` (channels 1, 2); projection in `functor-app-core/src/panel.rs`; conversion in `app/functor-app/src/link.rs`; `/state.link.*` | both, over the device link | hand-written big-endian binary | `panel::VERSION` (u8), `voice::VERSION` (u8), link `VERSION` (u16); each refused unless equal |
| **A** | The agent's app tools | `cli/src/commands/mcp_app.rs` (`show_view`, `present`, `task_change`, `report`), `mcp.rs` (`app_context`); app side `functor-app-core/src/agent_view.rs` (`ViewRequest`, `ViewReply`, `AppContext`), `POST /agent/view` (`functor-app/src/server.rs`) | both, loopback HTTP | JSON, `deny_unknown_fields` on tool args | none |

### State

"—" means the contract does not carry it.

| Contract name | P | H | A | Agreement |
| --- | --- | --- | --- | --- |
| `projects`, `open` | `AppState.projects: Vec<Project>` (name, path relative to the repo root, entries), `active`; also `repo_root`, `folder` (absolute), `recents` (absolute) | `Panel.project: Option<String>` (the path, ≤ 128 bytes) | `AppContext.project` (the task's), `active_project` | Same id (the relative path). P also carries absolute paths. |
| `games` | `AppState.session: Option<Session>` — one, the active project's; `SessionStatus::Running { debug_url, surface, pointer }`; `Mirror::Running { debug_url }`; clock as `ServerMsg::Transport` / `Marks` | `Panel.placed: bool` | `AppContext.session: Option<String>` (the debug URL), `session_mode` | **Diverge.** P and A hold the raw debug URL; H holds a flag. No id a second game could use. |
| `tasks` | `AppState.tasks: Vec<Task>`, all projects, flat; `TaskStatus` (8 values); harness, scope, model, `queued`, `created_at` / `updated_at` | `Panel.tasks` ≤ 6 of the open project: id, title (≤ 96), stance, activity | none (a run knows only its own task, implicitly, by its bearer secret) | Same id (`u64`). **Status diverges** (below). Flat everywhere. |
| task stance | `TaskStatus` × `activity::Stance` (5: thinking, working, waiting, done, failed) | core `PanelTask { stance, queued: bool }` → link `panel::Stance` (6: + `Queued`) | — | **Three vocabularies** for one thing. Kept / Reverted / ReadyForReview exist only in `TaskStatus`; H sees them as `Done` or `Waiting` plus `decision`. |
| activity | `AppState.activity[task]`: `calls: Vec<Call { kind, label }>`, `error`, `subagents: Vec<Subagent { label, calls }>`, `context: ContextFill { used, window }` | `Task.activity: String` (first call's label, else the error; ≤ 96) | — | H is a lossy projection: no kind, subagents or context. Subagents have **no id** anywhere. |
| `now` / current task | `AppState.current_task[project]` (what the chat shows; a client's choice) | `Panel.now` (computed: running, else waiting, else latest) | — | **Two concepts under similar names.** H's `Select` sets P's `current_task`, not `now`. |
| conversation | `ServerMsg::Transcript` / `Item` — whole transcript, with tool inputs and results (`Item::Tool { input, result }`) | D41 (#1105, open): `Panel.conversation`, last 12 items × ≤ 600 bytes, roles you / agent / app, no tool inputs or results | — | Diverge by design; D41's bounded form is the one a workspace may have. |
| decision | `AppState.decisions[task]: Decision { size: Size { added, removed, files, kinds }, depth, reason, style }` + `Task.report: Report { summary, notes, evidence: Vec<Check>, reviewed }` + `Task.change: Change { files, more, tree }` + `Task.evidence` | `Task.decision: Decision { tree, size: String ("+12 -3, 1 file, game logic"), glance: bool, reason, summary, notes ≤ 3 }` | writes it: `present { summary, notes, evidence }`, `report { … }` | **Diverge.** Structured across four fields on P; one flattened, pre-worded struct on H; no checks or evidence on H. |
| queue | `AppState.queues[project]: Queue { tasks, wait: Option<Wait>, holds }` | `Panel.waiting: String` (the wait, worded in core `panel.rs`) | — | H is P pre-worded; the wording lives in the projection, not the page. |
| routes | `AppState.routes: Vec<Routed { id, task, item, line, by, moves }>` | `Panel.route: Option<Route { id, line, moves ≤ 3 }>` — this headset's latest only | — | Same id. `route::Destination` and `panel::Destination` are the same enum written twice, converted by hand in `link.rs`. |
| request progress | none as a record: `AppState.voice: Vec<voice::Job>` (not serialised), `voice::Stage` (3) | `voice::Status` on channel 2 (`Heard`, `Transcribed`, `Working`, `Reloaded`, `Failed`, `Idle`); `/state.link.voice.stage` (9 strings, headset-local ones included) | — | **Diverge**: three stage lists, speech only, nothing for typed requests. D43 replaces all three. |
| view, highlight | `AppState.layouts[project]: Layout`, `highlight: Option<Highlight>` | — | `AppContext.layout: LayoutContext` (panes, windows, game placement, agent tabs) | A is a projection of P. |
| devices | `AppState.devices: Devices` (list with `DeviceState`, `app_version`, `tunnel` URL, `conn`; pair requests with `code`; cable state) | its own side: `/state.link { paired, linked, pairing, voice }` | — | Two halves of one link, not mirrored. |
| settings | `harnesses`, `harness_choice`, `scope_choice`, `review_settings`, `model_routes`, `speech`, `router`, `no_gpu`, `tracks_changes` | — | — | P only. |
| code | `AppState.files: Vec<String>` (the open project's loadable files); `Change.files` per task; `ServerMsg::File` (text), `ServerMsg::Diff` (lines) | — | `task_change` (review only): files, counts and the diff | No definitions anywhere. File text and diffs are P- and review-only. |
| refusal | `ServerMsg::Rejected { reason }` — a reply or a broadcast, not tied to a message | `Update::Refused { reason }` ("the last action") | `ViewReply { ok, message }`, tied to its HTTP request | **Three shapes**; none says "accepted" except A. |

### Actions

| Contract name | P (`ClientMsg`) | H (`panel::Action`, voice) | A | Agreement |
| --- | --- | --- | --- | --- |
| say / type | `NewTask { message, harness, scope }`, `SendMessage { task, message }` (follow-up, steer, or add to queued — by the task's state) | a `Clip` on channel 1 → transcribed → the router places it | — | H cannot name a recipient; P must. No "to the manager". |
| move a request | `MoveUtterance { id, to }` | `Move { id, to }` | — | Same; H limited to its own utterances (`panel::client_msg`). |
| queue | `MoveQueued`, `StartQueued` | — | — | P only. |
| stop | `StopTask { task }` | `Stop { task }` | — | Same. Confirmed by a second press on H (`link_panel.rs`), not on P, not in the core. |
| keep | `KeepChange { task }` | `Keep { task }` | — | Same. Confirmed on H only. |
| revert | `RevertChange { task, tree }` | `Revert { task, tree }` | — | Same. The core checks `tree` (what the user saw) and that nothing else runs; the confirmation itself is in each client. |
| select / open | `SelectTask`, `Layout { op }` (17 ops), `WalkthroughStep`, `ClearHighlight` | `Select { task }` | `show_view`, `present` (≤ 8 steps; ≤ 20 requests a run) | Three entry points to "show this". |
| report on a change | — | — | `present`, `report` (review) | A only. |
| project | `SetActiveProject`, `OpenFolder`, `PickFolder`, new-project messages, `RemoveRecent` | — | D42 (not found) | P only. |
| game | `StartSession`, `StopSession`, `Input`, `Transport`, `WatchTransport`, `Layout { op: Place / PlaceOnHeadset }` | — | the debug tools (`get_state`, `step`, `send_input`, …) against the session URL | P through the core; A straight to the debug server. |
| settings | `SetHarness`, `SetScope`, `SetReview`, `SetModelRoute` | — | — | P only. |
| devices | `AllowPairing`, `DenyPairing`, `ForgetDevice`, `PairByCable` | pairing on the headset's own overlay | — | Out of a workspace's reach by design. |
| content | `GetFile`, `GetDiff`, `GetTranscript`, `ListDirs` | — | `task_change` | Page-only. |
| task upkeep | `RenameTask`, `DeleteTask` | — | — | P only. |

### What the inventory shows

1. The headset contract is already a **projection of the page's**: every panel
   action becomes a `ClientMsg` in `panel::client_msg`, and `panel::panel` is
   a pure function of `AppState`. The workspace contract makes that the rule.
2. The same concept is **re-typed per transport**: stance (three enums),
   destination (two), decision (structured vs worded), refusal (three).
3. **Confirmation lives in clients.** The only rule the core holds for
   Keep / Revert / Stop is the revert's `tree` witness. D44 needs them behind
   the contract.
4. The page's state holds things no forked scene may have: absolute paths,
   the debug URL (which drives the game and loads code), tunnel URLs, pairing
   codes, tool inputs and results, file text.
5. Nothing is a tree, nothing names an agent, and only one game exists.

## 2. The schema, v0

Rust-like notation. `≤ n` is a bound in bytes of UTF-8 (text) or items
(lists); text over its bound is cut at a character boundary, lists keep the
most relevant. All paths are relative to the project directory, `/`-separated.
Times are ms since the Unix epoch; durations ms.

### Identity

```rust
type ProjectId = String; // ≤ 128: path relative to the workspace root (today's Project.path)
type TaskId    = u64;    // today's Task.id; never reused within a store
type AgentId   = String; // ≤ 80: "<task>" for a task's main agent,
                         //        "<task>/<call>" for a subagent (the harness's id of the call that started it)
type RequestId = u64;    // one per thing said or typed (D43); new
type ForkId    = u64;    // new (D44)
type GameId    = u64;    // today's Session.id: unique per start
type DeviceId  = String; // today's Device.id (hex)
type PendingId = u64;    // new; stable while the entry waits
type Seen      = String; // ≤ 64: an opaque witness of what the user was shown (today: Change.tree)
```

Unsure: a harness does not always give call ids (`Item::Tool.id` is
optional), so a subagent's `AgentId` falls back to its position among the
task's live subagents and may not be stable across a run for that harness.

### State (read-only)

```rust
struct Workspace {
    contract: u16,              // this schema's version
    caps: Vec<Cap>,             // what this app offers this client (§3)
    seq: u64,                   // bumps on every change
    projects: Vec<Project>,     // ≤ 64
    open: Option<ProjectId>,
    games: Vec<Game>,           // ≤ 8
    tasks: Vec<Task>,           // ≤ 64 of the open project; a tree by `parent`
    agents: Vec<Agent>,         // ≤ 32: live ones, and those waiting on the user
    requests: Vec<Request>,     // ≤ 8: in flight, or ended in the last 30 s
    pending: Vec<Pending>,      // ≤ 16, longest-waiting first
    decisions: Vec<Decision>,   // one per task or fork whose change waits
    queue: Option<Queue>,       // the open project's
    routes: Vec<Route>,         // ≤ 8, still movable
    forks: Vec<Fork>,           // ≤ 8
    code: Option<CodeMap>,      // the open project's
    devices: Vec<Device>,       // ≤ 8
    settings: Settings,
    focus: Focus,
}

struct Project { id: ProjectId, name: String /* ≤ 64 */, roles: Vec<String> /* ≤ 8 */ }

struct Game {
    id: GameId,
    project: ProjectId,
    fork: Option<ForkId>,       // the version it runs; None: the working tree
    place: Place,
    status: GameStatus,
    clock: Option<Clock>,       // None until it first answers
    frames: Frames,
}
enum Place      { ThisComputer { shown: Shown }, Headset { device: DeviceId, mirrored: bool } }
enum Shown      { Pane, Mini, Window, None }          // today's layout::Placement / SessionMode
enum GameStatus { Starting, Running, Stopped, Failed { reason: String /* ≤ 192 */ } }
struct Clock    { paused: bool, frame: u64, recorded: Option<[u64; 2]>, revision: u64 }
/// How a client gets pixels. Never a URL into the debug server's control routes.
struct Frames {
    stream: bool,               // a JPEG stream exists (`GET /stream`); the transport says where
    mirror: bool,               // a drawable frame exists (`GET /mirror`)
    embed: bool,                // the host can draw this game inside a scene (in-process only)
}

struct Task {
    id: TaskId,
    parent: Option<TaskId>,     // manager → worker → sub-worker (D45, D44)
    role: Role,
    title: String,              // ≤ 96
    status: Status,
    queued: Option<u32>,        // place in the queue, 0 next
    agent: Option<AgentId>,     // its main agent, while it has one
    harness: Harness, model: Option<String> /* ≤ 64 */, scope: Scope,
    fork: Option<ForkId>,
    created_at: Option<u64>, updated_at: Option<u64>,
}
enum Role    { Manager, Worker }
enum Harness { Claude, Codex }
enum Scope   { UserAndProject, Project, Isolated }   // D39
/// One vocabulary. `Failed` carries no text here: `Agent.error` and the conversation do.
enum Status  { Queued, Thinking, Working, Waiting, Review, Done, Failed, Kept, Reverted }

struct Agent {
    id: AgentId,
    task: TaskId,
    parent: Option<AgentId>,    // a subagent's starter; a worker's manager
    label: String,              // ≤ 96: the task's title, or what a subagent was asked
    stance: Stance,
    at: Vec<Touch>,             // ≤ 4 calls out now, oldest first: where it stands
    error: Option<String>,      // ≤ 192
    context: Option<Fill>,
    folded: u32,                // agents below it that are not listed (depth > 2)
}
enum Stance { Thinking, Working, Waiting, Done, Failed }      // today's activity::Stance
struct Fill  { used: u64, window: u64 }
struct Touch { kind: TouchKind, label: String /* ≤ 96 */, target: Target }
enum TouchKind { Thinking, Files, Shell, Network, Game, Agents, Skills, Ask }   // today's CallKind
enum Target {
    File { path: String /* ≤ 256 */, def: Option<String> /* ≤ 96 */, write: bool },
    Game { id: GameId },
    Network,                    // no host, no URL
    Agent { id: AgentId },
    User,
    None,
}

/// D43: one record per request, the same on every surface.
struct Request {
    id: RequestId,
    from: Source,
    text: Option<String>,       // ≤ 600: as typed, or as heard once transcribed
    step: Step,
    steps: Vec<Done>,           // ≤ 5: finished steps with their durations
    since: u64,                 // when `step` began
    task: Option<TaskId>,       // once placed
    route: Option<u64>,         // its Route, while movable
    outcome: Option<Outcome>,
}
enum Source  { Typed, Spoken { device: Option<DeviceId> } }
enum Step    { Received, Transcribed, Processing, Agent, Outcome }
struct Done  { step: Step, ms: u32 }
enum Outcome { Answered, Changed, NeedsYou, Failed { step: Step, reason: String /* ≤ 96 */ } }

/// D44: everything waiting on the user, in order.
struct Pending {
    id: PendingId,
    who: AgentId,
    task: TaskId,
    kind: PendingKind,
    line: String,               // ≤ 192: the question, the decision's reason, what it would run
    since: u64,
    seen: Option<Seen>,         // for Decision: the witness Keep / Revert must name
}
enum PendingKind { Question, Decision, Approval }

struct Decision {
    task: TaskId, fork: Option<ForkId>,
    seen: Seen,
    size: Size,
    glance: bool,               // "approve at a glance" vs "look into this" (D35)
    reason: String,             // ≤ 192
    summary: String,            // ≤ 640
    notes: Vec<String>,         // ≤ 3 × ≤ 192
    checks: Vec<Check>,         // ≤ 4
    reviewed: bool,
    evidence: Option<EvidenceRef>,
}
struct Size  { added: u32, removed: u32, files: u32, kinds: Vec<Kind> }
enum Kind    { GameLogic, Assets, Config, Other }
struct Check { ran: String /* ≤ 96 */, result: String /* ≤ 96 */, ok: bool }
enum EvidenceRef { Taking, Shown { before: ImageRef, after: ImageRef }, Same, Withheld { reason: String } }
type ImageRef = String;        // ≤ 64: a name the transport resolves; not a path

struct Queue { tasks: Vec<TaskId>, wait: Option<Wait>, holds: bool }
enum Wait    { Running { task: TaskId }, Reverting { task: TaskId }, Decision { task: TaskId }, Measuring { task: TaskId } }

struct Route { id: u64, request: RequestId, task: TaskId, line: String /* ≤ 192 */, by: By, moves: Vec<Move> /* ≤ 3 */ }
enum By      { Rule, Model, Manager, You }
struct Move  { to: Destination, label: String /* ≤ 96 */ }
enum Destination { Steer { task: TaskId }, New, Queued { task: TaskId } }

/// D44: a worker's own copy of the code.
struct Fork {
    id: ForkId,
    parent: Option<ForkId>,     // None: forked from the working tree
    task: TaskId, agent: AgentId,
    changed: Vec<Changed>,      // ≤ 32
    more: u32,
    game: Option<GameId>,
    merge: Merge,
}
struct Changed { path: String /* ≤ 256 */, added: u32, removed: u32, binary: bool }
enum Merge { Working, Ready, Merging, Merged, Conflict { files: Vec<String> /* ≤ 8 */ }, Dropped }

/// D44's shelf: names and sizes, never contents.
struct CodeMap { files: Vec<CodeFile> /* ≤ 256 */, more: u32, rev: u64 }
struct CodeFile {
    path: String,               // ≤ 256
    lines: u32,
    defs: Vec<Def>,             // ≤ 32, top-level only
    more_defs: u32,
    changed_by: Vec<TaskId>,    // ≤ 4: tasks whose waiting change touches it
}
struct Def { name: String /* ≤ 96 */, line: u32, lines: u32, changed: bool }

struct Device { id: DeviceId, name: String /* ≤ 64 */, link: Link, battery: Option<u8>, app_version: Option<String> /* ≤ 32 */ }
enum Link { NotPaired, AskingToPair, Away, Connected { via: Via }, NeedsUpdate { side: Side }, CannotReach }   // D27
enum Via  { Wifi, Cable }
enum Side { Headset, Desktop }

struct Settings {
    harnesses: Vec<Harness>, harness: Harness,
    scope: Scope, review: bool, model_route: bool,
    speech: bool, tracks_changes: bool,
}
struct Focus {
    task: Option<TaskId>,       // today's current_task: the conversation in front
    now: Option<TaskId>,        // today's Panel.now: running, else waiting, else latest
    view: Option<ViewTarget>,   // today's Highlight, as a target
}
enum ViewTarget { File { path: String, lines: Option<[u32; 2]> }, Changes { task: TaskId, path: Option<String> }, Game { id: GameId }, Task { id: TaskId } }
```

The conversation is asked for, not part of `Workspace` (it changes per token):

```rust
/// D41's bound: the latest 12, oldest first.
struct Conversation { task: TaskId, earlier: bool, items: Vec<Line> /* ≤ 12 */ }
struct Line { role: Speaker, text: String /* ≤ 600 */, cut: bool, spoken: bool, partial: bool }
enum Speaker { You, Agent, App }   // App: route lines, "3 steps, last: running cargo test", Done / Failed / Stopped / Kept / Reverted
```

### Actions

Every action is `Act { id: u32, action }` and is answered once:

```rust
enum Reply {
    Accepted,                                  // the state change follows in `Workspace`
    Refused { reason: String /* ≤ 192 */ },    // nothing changed
    Confirm { ticket: u64, line: String },     // the app wants the user's confirmation first (below)
}
```

Issuers: **P** the page, **H** the headset panel, **W** a workshop scene,
**M** the manager agent (through its tool server), **A** a worker's tool
server. "Host" is the shell around a client that the client cannot edit: the
page itself for P, the runtime for H and W.

| Action | Arguments | Who | The app enforces | Reply |
| --- | --- | --- | --- | --- |
| `Say` | `to: Recipient` (`Manager` \| `Task(TaskId)` \| `Agent(AgentId)`), `text ≤ 4000`, `scope: Option<Scope>` | P, H, W | Empty text refused. `Manager`: the router or manager places it (new, steer, queue). `Task` / `Agent`: only one of the open project; a steer to a subagent reaches its piece only and the manager is told (D45). `scope` only on a new task, and only P may set it. 1 per second per client. | `Accepted` with the `RequestId` in `requests`; `Refused` |
| `Speak` | a clip (transport-specific) + capsule | H, W | 60 s, one at a time per device, consent to the microphone on the device; then as `Say { to: Manager }`. | as `Say` |
| `MoveRequest` | `route: u64`, `to: Destination` | P, H, W | `to` must be one of the route's `moves`; H and W only their own device's requests. | `Accepted` \| `Refused` ("can no longer be moved") |
| `MoveQueued` | `task`, `to: u32` | P, M | The task is queued. | `Accepted` \| `Refused` |
| `StartQueued` | `task` | P, M | Refused while a run or a revert holds the project. | `Accepted` \| `Refused` |
| `Stop` | `task` (and all below it) | P, H, W, M | The task or a descendant runs. Confirmation when it stops more than one run. | `Accepted` \| `Confirm` \| `Refused` |
| `Keep` | `task` or `fork`, `seen: Seen` | P, H, W | A change waits; `seen` is current; for a fork, its merge is `Ready` and the combined result was shown (D44). Confirmation. Never M or A. | `Confirm` → `Accepted` \| `Refused` |
| `Revert` | `task` or `fork`, `seen: Seen` | P, H, W | As today: `seen` current, no other run or revert, every changed file listed. Confirmation. Never M or A. | `Confirm` → `Accepted` \| `Refused` |
| `Answer` | `pending: PendingId`, `text ≤ 4000` or `choice: u8` | P, H, W | The entry still waits and is a `Question`. | `Accepted` \| `Refused` |
| `Approve` | `pending: PendingId`, `allow: bool`, `seen` | P, H, W | The entry is an `Approval`; `seen` names exactly what would run. Confirmation, always. Never M or A. | `Confirm` → `Accepted` \| `Refused` |
| `Confirmed` | `ticket`, `yes: bool` | **hosts only** | The ticket is this client's, unexpired (30 s), and what it guards is unchanged. | `Accepted` \| `Refused` |
| `Select` | `ViewTarget` | P, H, W; A as `show_view` (own task's project, ≤ 20 a run) | The target exists in the open project. Changes what is in front (`focus`), never the user's text cursor. | `Accepted` \| `Refused` |
| `Present` | steps ≤ 8, summary, notes, checks | A (own run); a review as `Report` | As today (`agent_view.rs`). | `Accepted` with words \| `Refused` |
| `OpenProject` | `ProjectId` | P, H, W, M | Listed in `projects`. Refused while a run is active there if it would stop it. Opening a folder by path stays page-only. | `Accepted` \| `Refused` |
| `GoIn` / `ComeOut` | `game: GameId`, `at: Option<[f32; 3]>` | H, W | The game runs and has a 1:1 inside; its state is unchanged by going in or out (S7). | `Accepted` \| `Refused` |
| `Place` | `game`, `Place` | P | As today's `LayoutOp::Place` / `PlaceOnHeadset`. | `Accepted` \| `Refused` |
| `Clock` | `game`, `Pause` \| `Resume` \| `Step` \| `Seek { frame }` | P, W, H | The game runs; one in flight per game. No input injection, no code. | `Accepted` \| `Refused` |
| `GetConversation` | `task` | P, H, W | A task of the open project; ≤ 2 per second. | `Conversation` \| `Refused` |
| layout | — | workspace-local | **Never reaches the app.** A scene's layout is its own model; the page's `LayoutOp` and the panel's page/focus stay in their clients. | — |

**Confirmation.** For `Confirm`, the app keeps the action and returns a
ticket; the **host** shows `line` on a surface the client does not draw (the
page's own dialog; the runtime's overlay in the headset, as D28's panel is
drawn) and sends `Confirmed`. A scene cannot send `Confirmed`, so an edited
workshop cannot remove the step. This generalises today's `tree` witness and
moves the panel's second press (`link_panel.rs`) and the page's revert dialog
(`DiffView.tsx`) behind the contract.

**What an agent may run** is not an action: it stays the run's permissions
(`functor-app/src/agent.rs`, `permissions`) and scope (D39). Today a refused
tool is reported after the fact (`Agent.error`); `Approval` entries need the
harness to ask first — unsure whether it can (S2 left "ask mode" unchecked).

### Never in the contract

| Never | Today it is in | Instead |
| --- | --- | --- |
| The page's session token, a run's bearer secret, pairing secrets and codes | `server::Token`, `agent::token`, `PairRequest.code` | Nothing. Pairing stays a page / headset-overlay flow. |
| A debug-server URL or a tunnel URL (clock, input, `/load-project`: it can run code) | `SessionStatus::Running.debug_url`, `Mirror`, `Device.tunnel`, `AppContext.session` | `Game.frames` + the `Clock` action. A worker's tool server keeps its URL outside the contract. |
| Absolute paths, process ids | `repo_root`, `folder`, `recents`, `Session.pid` | Project-relative paths; `ProjectId`. |
| File contents and diff lines | `ServerMsg::File`, `ServerMsg::Diff`, `task_change` | Names, line counts, definition names (`CodeMap`, `Changed`). |
| Tool inputs and results | `Item::Tool { input, result }` | `Touch.label` (the chat's own short label), `Line` for runs of steps. |
| The capsule (head and hand poses) of other devices, raw audio | `Spoken.context`, channel 1 | A request's `text`. |
| Usage in money | `UsageDetail.cost_usd` | `Fill` only. |
| Anything that runs a command | — | No action takes a command, a path outside the project, or code. |

Allowed text, exhaustively: titles (≤ 96), labels (≤ 96), reasons and lines
(≤ 192), a decision's summary (≤ 640) and notes, a request's text (≤ 600),
conversation lines (≤ 12 × 600, as D41 defines them), file and definition names.

The **page** is first-party and keeps more than the contract (file text,
diffs, the full transcript, folders, pairing, the debug URL for its own game
view): those stay as page-only messages beside the contract, named as such.
The page's game view moving off the raw debug URL is desirable, not required.

## 3. Versioning and compatibility

| Topic | Rule |
| --- | --- |
| Version | One `contract: u16`. v0 while unreleased (may break); v1 at the first release a user's scene can depend on. |
| Changes | Additive within a version: new optional fields, new enum values, new actions, new caps. A removal or a change of meaning is a new version. |
| Unknowns | A client ignores unknown fields and treats an unknown enum value as that enum's documented fallback (`Status` → `Working`, `TouchKind` → `Thinking`, `Target` → `None`, `PendingKind` → `Decision`, `Merge` → `Working`). The app refuses an unknown action with `Refused`. |
| Hello | A client opens with `Hello { contract_min, contract_max, client: String ≤ 32, caps }`; the app answers the version it will speak, or `Refused { reason }` naming which side to update (D27's `NeedsUpdate`). Two versions served at once: current and previous. |
| An older client | Sees the projection for its version: fields it does not know are absent, lists stay within its bounds. An old headset app keeps working as the panel it is — unlike today, where `panel::VERSION` 1 ≠ 2 leaves "waiting for the desktop". |
| A forked workshop scene | Declares the version it was written against (a constant in the scene, checked at load). Older than `contract_min`: the built-in default loads and says why (D44, "cannot lock the user out"). |
| Caps | `Cap`: `tree`, `forks`, `code_map`, `requests`, `pending`, `conversation`, `voice_out`, `embed`, `go_in`, `clock`. The app lists what it offers; the client lists what it draws. A section without its cap on both sides is omitted (empty), not sent and ignored. |
| Bounds | Per field as written in §2. Per update: the whole `Workspace` ≤ 48 KB encoded for the link (one link message is 65 535 bytes), ≤ 256 KB for in-process and JSON clients; over it, lists are cut in this order: `code.files`, `forks[].changed`, `tasks` (oldest done first), never `pending`, `requests` or `decisions`. |
| Updates | **Snapshot, then changes by section.** After `Hello` the client gets the whole `Workspace`. Then `Changed { seq, sections }`, each section sent whole (`tasks`, `agents`, `requests`, …). No field-level patches. A client that sees a gap in `seq` asks for a snapshot. |
| Rate | At most 2 updates a second per client (D41's 500 ms); `requests` and `pending` changes are sent at once. The latest wins; nothing is queued behind a slow client (today a page 32 messages behind is dropped and reconnects: `CLIENT_QUEUE`). |
| Ordering | `seq` orders updates. A `Reply` comes before the update that shows its effect. `Act.id` is the client's and is echoed. |
| Ids | Stable for the life of the thing; never reused within a store (`TaskId`, `RequestId`, `ForkId`), within an app run (`GameId`, `PendingId`, route ids). An `AgentId` is stable for its run. |

## 4. Transports: views of one contract

| Client | Transport | What it carries | Today → then |
| --- | --- | --- | --- |
| Page | WebSocket JSON (`/ws`) | The contract + page-only messages | `ServerMsg::State { AppState }` whole → `AppState` embeds `Workspace` as one field; the page reads shared concepts from it. Whole-state stays for the page. |
| Headset panel | Link channels 3 / 4 (binary) | A bounded projection: `open`, `focus`, `tasks` ≤ 6, `pending`, `decisions`, `queue.wait`, `routes` (own), `requests` (own), `Conversation` | `panel::panel(state, device)` becomes `project(workspace, PanelV)`; the encoder stays hand-written but is driven from the contract's types. Channels 1 / 2 stay (audio in; `Status` becomes the request's `Step`). |
| Workshop scene | In-process messages to a typed Functor Lang module | The contract as plain values; actions as effects | New. Sketch below. |
| Development | Recorded state files | `Workspace` snapshots + `Changed` + `Conversation`, as JSON lines with times; actions answered from a script | New: `functor run native --workspace-recording <file>`; the app writes one with a developer-menu switch. |
| Worker's tool server | Loopback HTTP (`/agent/view`) | `Select` / `Present` / `Report`, `AppContext` | Unchanged; its types are named in the contract. |
| Manager's tool server | Loopback HTTP | `Workspace` (read), `Say`, `MoveQueued`, `StartQueued`, `Stop`, `OpenProject` | New (D45). |

The game itself is not re-exported: a workspace holds `GameId` and gets
frames by the transport's means — the page from `/stream`, a mirror from
`/mirror`, a scene by embedding — and the debug runtime stays the game's API
for tools and tests.

### The Functor Lang module (sketch)

`workspace.funi`, provided only when the runtime hosts a workspace scene. The
types mirror §2 (records and variants; numbers are `float`; ids are branded).

```
//! The app's workspace: what a workshop scene may read and ask for.

type t = { contract: float, seq: float, projects: List<Project>, open: Option.t<ProjectId>,
           games: List<Game>, tasks: List<Task>, agents: List<Agent>, requests: List<Request>,
           pending: List<Pending>, decisions: List<Decision>, forks: List<Fork>,
           code: Option.t<CodeMap>, devices: List<Device>, settings: Settings, focus: Focus }

type reply =
  | Accepted
  | Refused(reason: string)
  | Confirming            // the host is asking the user; a second reply follows

type recipient =
  | Manager
  | Task(id: TaskId)
  | Agent(id: AgentId)

/// The workspace as it changes: one message per update, the whole value.
let changes : ((t) => 'msg) => Sub.t
/// A task's bounded conversation, while subscribed.
let conversation : (TaskId, (Conversation) => 'msg) => Sub.t

let say         : (recipient, string, (reply) => 'msg) => Effect.t
let moveRequest : (RouteId, Destination, (reply) => 'msg) => Effect.t
let stop        : (TaskId, (reply) => 'msg) => Effect.t
let keep        : (Subject, Seen, (reply) => 'msg) => Effect.t
let revert      : (Subject, Seen, (reply) => 'msg) => Effect.t
let answer      : (PendingId, string, (reply) => 'msg) => Effect.t
let approve     : (PendingId, bool, Seen, (reply) => 'msg) => Effect.t
let select      : (ViewTarget, (reply) => 'msg) => Effect.t
let openProject : (ProjectId, (reply) => 'msg) => Effect.t
let goIn        : (GameId, Option.t<Vec3.t>, (reply) => 'msg) => Effect.t
let clock       : (GameId, ClockCall, (reply) => 'msg) => Effect.t

/// The running game, drawn inside this scene (the miniature).
let game : (GameId) => Scene.t
/// A decision's before / after image.
let image : (ImageRef) => Texture.t
```

State arrives as a **subscription** and actions are **effects** with a tagger,
so the scene stays a pure `update` over messages: state and replies land in
the recorder's frame log, and a recorded session replays with no app
(`ReplayEffects`). No `impure` read is needed. There is no `confirmed`.

What the engine needs to make it real:

| Need | Note |
| --- | --- |
| A host seam for app-provided subscriptions and effects | Registered in the typed external registry; delivered through `drain_effects` like `Sub.connect` messages. The runtime, not the scene, holds the connection to the app (in-process on desktop; the device link's panel channel in the headset). |
| `Workspace.game` | A scene node that draws another program's frame (S6): the `/mirror` frame on a headset whose game runs elsewhere, a second producer in one process otherwise. |
| A host-drawn confirmation and "reset my workspace" | Outside the scene's `draw` (as the link panel and toasts are today). |
| A second program per frame | D44's "time to run two programs in one headset frame". |
| A fallback scene | Load failure or a contract older than `contract_min` loads the built-in default. |

### One source, generated

| Artifact | From | Drift caught by |
| --- | --- | --- |
| Rust types (`functor-workspace-contract`, a small crate with no app or runtime dependency, so `functor-device-link` and the runtimes can use it) | the source of truth | — |
| `app/ui/src/generated/protocol.ts` | `ts-rs`, as today | `functor-app-core/src/ts_bindings.rs` (already fails CI when stale) |
| `workspace.funi` types and signatures | a generator test over the same types | the existing `.funi` ≡ registry drift test, plus a `UPDATE_FUNI=1`-style check like `ts_bindings` |
| Link encoding (panel channel) | hand-written `encode` / `decode` per version | a round-trip test per contract type, and a test that every field of the projection is either encoded or listed as left out |
| Bounds | constants in the contract crate | one test: the largest legal `Workspace` fits its transport's limit |
| Recorded state | serialised `Workspace` | a committed recording replayed in CI against the default workshop scene |

## 5. Gaps, in order

What D44 and D45 need that no contract carries today:

| # | Gap | Needed for | Exists today |
| --- | --- | --- | --- |
| 1 | **One refusal / reply shape, with confirmation in the app** | every client; the precondition for an editable scene | three shapes; confirmation in two clients |
| 2 | **Request progress** (`Request`) | D43; the toast, the chat line, the room | `voice::Job` (speech only, not serialised); 4.4c in flight |
| 3 | **The pending list as such** (`Pending`) | D44's ordered queue at the desk; D45's "its list" | derivable: `Stance::Waiting` + `decisions` + `CallKind::Ask`; no order, no id, no approvals |
| 4 | **Agents as entries with ids and targets** (`Agent`, `Touch.target`) | figures standing at a file, the game, the network | `Activity.calls` have a kind and a label; the path is inside the label; subagents have no id |
| 5 | **The task tree and the manager** (`Task.parent`, `Role`, `Recipient::Manager`, `By::Manager`) | D45; S8's tasks tree | flat tasks; the router is not a task |
| 6 | **Game handles** (`Game`, more than one) | a miniature per fork; before / after side by side | one `Session`, addressed by URL |
| 7 | **Forks** (`Fork`, `Merge`, fork-level Keep / Revert) | D44's lanes; S8 | none (one working tree, one baseline per task) |
| 8 | **The code map** (`CodeMap`, definitions) | the shelf | `AppState.files` (names only); the language crate can list definitions — unsure of an existing headless entry point that returns names with line spans |
| 9 | **Voice out and sound hooks** | "Changed. Ready to look at."; a chime where an agent stands | none. Proposed as derived, not stored: a client speaks `Request.outcome` and chimes on a new `Pending`; add `say: Option<String ≤ 192>` to `Request` and `Pending` for the manager's words (D45), behind cap `voice_out` |
| 10 | **`OpenProject` outside the page** | D42; D45's "switch project" | `SetActiveProject` (page only) |

Duplicated today, to merge first: stance (`TaskStatus` / `activity::Stance` /
`panel::Stance` / `PanelTask.queued`); `route::Destination` ≡
`panel::Destination`; the decision's two shapes; three refusals; three
request-stage lists; `now` vs `current_task`.

### Migration, each step one reviewable PR

The page and the panel work after every step.

| Step | Change | Keeps working because |
| --- | --- | --- |
| 1 | **The contract crate, shared vocabulary.** Move the types both sides already agree on into `functor-workspace-contract`: `Stance`, `Status`, `Destination`, `Move`, `Wait`, `Size` / `Kind`, `Check`, `ViewTarget`, ids, bounds. `functor-app-core` and `functor-device-link` re-export them; `link.rs`'s hand conversions go. | Type moves only; wire bytes and JSON unchanged (tests pin both). |
| 2 | **`Workspace` as a pure projection.** `fn workspace(&AppState) -> Workspace` in the core, with the parts that exist (projects, one game, flat tasks with `Status`, agents from `Activity`, decisions, queue, routes, devices, settings, focus). `AppState` serialises it as one field; `panel::panel` is rewritten as a projection of it. Generated TS gains the types. | The page ignores the new field; the panel's bytes are unchanged (golden bytes test). |
| 3 | **Replies and confirmation behind the contract.** `Act { id }` / `Reply` on all three transports; `Confirm` tickets in the core; the page's revert dialog and the panel's second press become the hosts' `Confirmed`. `Rejected` / `Refused` stay as aliases for one release. | Old messages still accepted; an old headset's actions take the unconfirmed path only where the core allows it today. Needs a panel version, so do it with step 4. |
| 4 | **`Request` and `Pending`; `Hello` and caps.** Land D43's record (4.4c) as `Workspace.requests`, derive `pending`; the link's panel channel negotiates a version range instead of strict equality; `voice::Status` is fed from `Request.step`. | The page reads the same record for its chat line; a headset that sends no `Hello` gets today's panel. |
| 5 | **Agents with ids and targets; the tree.** `Touch.target` from the call's parsed input (the core already derives the label from it); `AgentId`s; `Task.parent` and `Role` with the manager (D45, S8). | Additive fields; the page's agent cards read the old `Activity` until they move. |
| 6 | **The scene transport.** The `.funi` module, the host seam, recordings, and a default scene that draws tasks, pending and the conversation (no miniature yet). Forks, more games and the code map follow as S6–S8 land, each as a cap. | New client only. |

## 6. Open questions

| # | Question | Recommendation |
| --- | --- | --- |
| 1 | Does **Keep** need confirmation everywhere? D44 says Keep, Revert and Stop; the page keeps with one click today. | Confirm Revert, Approve, fork Keep and a Stop that ends more than one run, on every client. Let a plain Keep of one task pass without it on the page; require it from H and W. The rule is the app's, per client kind. |
| 2 | May a workshop scene **drive the game's clock** (the dial on the miniature), and its input? | Clock yes, through the `Clock` action. Input no in v0: going in is how the user plays; a scene that injects input is a command channel. |
| 3 | Is the **manager a task**? | Yes: one long-lived `Task { role: Manager }` per project, parent of the workers. It gives the tree a root, the conversation a home, and reuses stop / status. It has no change to keep. |
| 4 | How much of the **code map** on a headset: every file, or changed and nearby? | Bound it (256 files, 32 definitions each), changed files first, `more` counted; the city (D33) pages by directory later. |
| 5 | One **contract crate** now, or keep types in `functor-app-core` and generate the rest? | The crate, in step 1: the link and the runtimes cannot depend on the app core, and that is why the types were written twice. |
| 6 | Should the **page** stop receiving the debug URL and file contents, to be a client like the others? | Not now. The page is first-party; name its extras "page-only" and move the game view to a handle only when a second game (forks) forces it. |
