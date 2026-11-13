# Desktop app — implementation plan v1

Builds the design in [`desktop-app-revisions-v1.md`](desktop-app-revisions-v1.md)
(decisions D1–D15) on top of the intent in [`../desktop-app.md`](../desktop-app.md).
Written to be handed to implementation agents one task at a time.

## How to work this plan

- **One task = one draft PR**, stacked on the task before it. Title prefix per
  `CLAUDE.md` (`feat(app): …`). Mark ready only when the task's "Verify" has
  been re-run after review fixes.
- **Sequential.** One implementation agent at a time. Tasks marked ∥ touch
  disjoint files and may run as a second agent.
- **Commit early**, push the branch as soon as it has anything worth keeping,
  and tick the task in the status note (kept outside the repo, next to the
  other project notes) when its PR opens and again when it merges.
- **Review** every non-trivial task with `/xreview` before marking ready.
- **Each task must be checkable without a window**: a test, a `--print` text
  dump, or a scripted browser run. If a task cannot be, say so in the PR.
- **Scope**: do only the task. Note anything else you find in the PR body.
- Spikes (S-tasks) produce a short findings section appended to this file and
  no merged code unless stated.

## Shape

```
app/
  functor-app-core/   lib  — state, protocol, projects, sessions, agent, tasks,
                             devices, pr builds, capsules. No UI, no window.
  functor-app/        bin  — axum server (JSON + WebSocket + static UI),
                             `--headless`, `--print`, and the wry window.
  ui/                 TS/React, built with the same esbuild setup as `site/`.
```

- The **core is a pure-ish state machine**: `AppState` + `update(state, event)
  -> (state, commands)`; IO (processes, sockets, files) lives in a thin driver
  that executes commands and feeds results back as events. This is what makes
  `--print` and the tests cheap.
- **Wire protocol** (`ClientMsg` / `ServerMsg`) is serde types in the core with
  TS types generated the way the debug protocol's are (`UPDATE_TS=1`); CI fails
  when stale.
- **A game session is a child process** — `functor run native --hidden
  --debug-port 0` — spoken to over the debug runtime's HTTP. It is never
  in-process: GLFW and the webview both need the macOS main thread.
- **The agent** runs through the `mis-agent` crate (git dependency on the
  MakeItSo repo, pinned to a revision): `Agent::run(task, prompt, RunOpts, tx)`
  streaming `AgentEvent`s. It is offered one loopback HTTP MCP server per run.
- **The agent's game tools** are the existing `functor mcp` tools, served over
  HTTP as well as stdio (task 1.6) so they fit that one-server slot. The tool
  router lives in the `functor` binary, so the core runs `functor mcp --http`
  as a child rather than hosting the tools itself.
- `functor app` launches the sibling `functor-app` binary. The `functor` CLI
  gains no webview dependency and its build time must not move.

## M0 — spikes (do first, in order)

**S1 — window and game view.** Open a wry window on a local page while a
hidden `functor run native` child runs. Measure frames per second and latency
of showing the child's frames in the page at 1280×720, release build, via
`POST /capture` in a loop.
- Decide: is `/capture` polling enough (target ≥ 30 fps, < 80 ms), or does the
  debug server need a streaming route (`GET /stream`, JPEG frames over one
  response)? If needed, that route becomes task 1.4a.
- Also confirm: keyboard and mouse forwarded through `POST /input` feel usable.

**S2 — agent and tools.** In a scratch crate inside the workspace: add
`mis-agent` as a git dependency and check it builds with this workspace's
toolchain and tokio; run Claude Code through it against `examples/hello` with
an HTTP MCP server that exposes one tool proxying `GET /state`.
- Decide: is one MCP slot named `mis` workable as is, or does `mis-agent` need
  a small upstream change (server name, more than one server)? If so, open
  that PR upstream first.
- Record the `AgentEvent` variants the UI will have to render.

## M1 — desktop chat + game view

| # | Task | Verify |
| --- | --- | --- |
| 1.1 | `functor-app-core` skeleton: `AppState`, `update`, protocol types + TS generation, project discovery (every `functor.json` under the repo root; active project = nearest to the start directory). `functor-app --headless --print` dumps state as text. | Unit tests on `update`; `--print` in `examples/` lists the projects. |
| 1.2 | Session manager: start/stop the hidden child, read the bound port from its `[debug-server] listening` line, poll `/state` (frame, paused, problems), restart on project switch. | Integration test using `--headless` (no GPU), runs in CI. |
| 1.3 | Server and UI shell: axum serves the UI and a WebSocket carrying the protocol. UI: title bar, rail (Explorer, Tasks, Devices), side pane, canvas, chat dock; MIS colour tokens lifted into one stylesheet; Functor logo; project switcher. | Playwright: page loads, rail switches panes, state arrives over the socket. Desktop and narrow widths. |
| 1.4 | Game view: show the session's frames from `/stream` (1.4a), forward input on focus, "Running in this window" picker, "Open in window". | Playwright against a `--fixed-time` session: the canvas shows a non-blank frame that matches a `/capture`. |
| 1.4a | `GET /stream` on the debug server (decided by S1): JPEG frames encoded off the render thread, sized to the on-screen view, sent only while a client is connected. Serve `multipart/x-mixed-replace` for `<img>` and the same framing as `application/octet-stream` for `fetch` (WKWebView's `fetch` rejects the multipart type). Pace hidden sessions by frame time instead of a fixed 16 ms sleep. Serve input on its own connection so it never queues behind a frame. | Bench in the PR: fps, read-to-drawn ms and game CPU at 1280×720 and 2560×1440 against the S1 numbers; `frame_bench` unchanged. |
| 1.4b | Input coverage for the game view: modifier, Tab and function keys on the `/input` wire, and relative mouse (pointer lock) for free-look games. | Unit tests on key names; Playwright: pointer-locked moves reach `/state.input`. |
| 1.5 | Transport: reuse `runtime/functor-runtime-web/scrubber.js` and `timeline-model.js` against the native session's `/time`, `/rewind`, `/input-log`. No redrawn copy. | Existing `timeline-model` tests still pass; Playwright: pause, step, rewind change `/state.frame`. |
| 1.6 ∥ | `functor mcp --http <port|0>`: the same tool router over rmcp 2.2's streamable-HTTP server (feature `transport-streamable-http-server`, mounted in axum at `/mcp`), bearer token from an env var, bound URL printed on stderr. `--attach <debug-url>` registers an existing session at start. An `app_context` tool returns what the app last posted to it (open view, selected file, active session). No tool may be named `report_*` or `approve`. | Existing MCP tests on both transports; a request without the token is refused. |
| 1.6a ∥ | Upstream, in MakeItSo's `mis-agent`: hide only its own three report tools instead of every `mcp__mis__*` call, so our game-tool calls reach the event stream; map Codex `mcp_tool_call` and `file_change` items to `ToolUse` / `ToolResult`. Pin the new revision here. | Upstream tests; the S2 spike's Claude run shows `tool_use` for `game_state`. |
| 1.7 | Agent runner: a task = one conversation; the core spawns `functor mcp --http 0 --attach …` as a child and passes it to `mis-agent` as the run's one MCP server; stream events to the UI; Stop; persist tasks and transcripts per project under `~/.functor/app/`. Protocol types mirror `AgentEvent` (it has no TS derives); tool results are paired to calls by order. Runs use folder-scoped permissions on the project and load the user's own agent settings (D19); keep that one choice in a single place so an isolated mode can be added. A `FakeAgent` replaying the event logs recorded in S2 drives the tests. | Unit tests with `FakeAgent`; one opt-in live test. |
| 1.8 | Chat UI: composer (D15) with context chips, steps, cards for captures and changes with "open in canvas", result pill. | Playwright with `FakeAgent`. |
| 1.9 | Changes: per-task diff of the working tree, diff view in the canvas, Keep / Revert. | Test on a temp git repo. |
| S4 | Spike before 1.9b: pick the pane/tab library. Check a React docking library (first candidate: `dockview`) for split right/down, floating panes (for Mini), pop-out windows, serialisable layout, keyboard access, and that pop-out works inside WKWebView; fall back to a small hand-written split tree if it does not fit. | Findings appended here. |
| 1.9b | Tabs and splits (D20): the canvas becomes panes of tabs with the four pane actions; Game, file, Changes and task page are view types; opening rules, empty state, layout held by the core per project with a reset. Replaces the fixed Game / Code / Diff tabs. | Core unit tests for the layout reducer; Playwright: split, move, close, reload restores the layout; one game stream open at any time. |
| 1.9c | Game placements (D21, D22): one placement control — Pane, Mini (floating, draggable, resizable, dock back), Own window — with Start/Stop beside it (Start: paused on frame 0, D37); Pane ↔ Mini moves the same stream. The Headset placement is added in M2. | Playwright: each placement shows a live frame or the note; mini survives tab and pane changes; one stream open. |
| 1.10 | Tasks view (D12): grouped list, task page with what the agent was given, steps, change, before/after captures. | Playwright with `FakeAgent`. |
| 1.11 | Window and launcher: wry window around the served page; `functor app` finds and starts `functor-app`; macOS only. | `functor app --headless --print` in CI; window checked by hand once. |
| 1.12 | Docs page for the app, and `pr-visuals` captures of the three views. | — |
| 1.13 | Open any folder (D23): open-folder action, recent projects kept by the core, the app no longer assumes its start directory is the workspace; non-git folders work with review features off. | Core unit tests; Playwright with a temp folder outside any repo. |
| 1.14 ∥ | Templates in the CLI (D24): a template manifest (name, description, tags, preview), `functor init --list` (text and JSON) and `--from <template>`; add blank, 2D and VR starters; ship chosen examples as templates. | CLI tests: every listed template creates a project that passes `functor build`. |
| 1.15 | New-project modal (D24): name, folder with a native folder picker, optional git init, the template gallery with search and tags, optional description that becomes the first task; creates, switches to and starts the project. | Playwright with a fake template list; an end-to-end create in a temp folder. |
| 1.16 | Agent-driven view (D25), after 1.9b: view tools on the agent's tool server that call back into the core (open/focus, arrange, highlight, game moment), the walkthrough card in the chat with its first step applied automatically when a run ends (a setting turns that off), the "never takes the wheel" rules and "restore my layout". | Core unit tests for the rules; Playwright with a fake-agent fixture that issues view calls; focus and cursor unchanged. |
| 1.17 | Agent activity (D34), core: classify each call into the fixed kinds (shell commands by what they touch), a stance per task, context fill, and subagent calls tied to their parent. The last two need more from the agent runner's event stream. | Core unit tests on recorded event logs; a classifier table test. |
| 1.18 | Agent cards (D34), desktop: avatar, thought bubble and context bar per running task in the Tasks pane and the chat dock; engines and subagents told apart by shape. | Playwright with `FakeAgent`; stills of every stance. |
| 1.22 | The game opens paused (D37): opening a project starts its game hidden and paused on frame 0 (`functor run native --paused`); the game view's Start prompt goes; Stop game / Start game beside the placement control; the agent is told the game starts paused. | Clock and core unit tests; Playwright: opening shows a frame and a paused clock at frame 0 with no click, and Play advances it. |
| 1.19 | Agent-run metrics: a fixed set of scenarios run through `functor-app task`, recording turns, tokens, time and outcome per scenario to a file, so prompt and settings changes can be compared and regressions seen. | The scenarios run against `FakeAgent` in CI; one opt-in live run with its numbers recorded here. |
| 1.20 | Extrapolate in the app (D38): the control on the transport bar, the path drawn in the game view, and the automatic peek when an edit lands while paused; reuses the runtime's extrapolation and the web scrubber's model. Then re-record the site's app-loop capture with it. | Core and shell tests; Playwright with a fixture edit: the path changes after the edit; stills. |
| 1.21 | Extrapolation as evidence (D38): the old and new paths from the task's moment on the change card, taken headlessly from the task's baseline and current files with the same inputs. | A fixture change whose two paths differ; captures match goldens. |

## M2 — devices and the Wi-Fi link

| # | Task | Verify |
| --- | --- | --- |
| 2.1 ∥ | `functor devices` (text and `--json`): adb wrapper listing headsets with link, battery and build. | Tests against a fake `adb` on `PATH`. |
| 2.2 | Device link, desktop side: WebSocket listener with token auth, request/response tunnelling of debug-runtime calls, a local port that proxies to the device through the link, mDNS advert. A loopback **device stub** binary speaks the device side for tests. | Tests: stub connects, `curl` to the proxied port reaches the stub; bad token refused. |
| 2.3 | Device link, Quest side: read the pairing file, find the desktop by mDNS, dial with the token, serve tunnelled requests into the existing debug handler, reconnect with backoff. | Stub-level tests in common code; on-device check per the `vr-device-loop` skill. |
| 2.4 | Pairing: on cable attach, write token and desktop address to the headset over adb and enable adb over Wi-Fi. Devices pane shows link, cable, battery, build, Forget. | Fake `adb` tests; headset when attached. State in the PR when none was. |
| 2.4a | Pairing without a cable (D26): the headset app lists desktops it finds and shows a code; the desktop shows the same code and Allow / Not mine; the pairing secret is exchanged only after Allow. | Stub pairs end to end; a wrong or expired code is refused; nothing typed on the device side. |
| 2.4c | Connection states (D27): the six states as core state from link and adb events, the title-bar pill, the Devices pane cards with their one action each, headset-app update over the link. | Core unit tests per transition; Playwright with a stub device walking every state. |
| 2.4b | Headset placement (D21): the fourth game placement appears while a headset is linked; choosing it runs the project on the device and turns the desktop game view into the mirror placeholder (the mirror itself is 4.5). | Stub-driven Playwright. |
| 2.5 | `run vr` over the link: source and asset pushes go through the tunnel when a link is up, adb forward otherwise. | Existing `run vr` tests; stub test. |

## M3 — pull-request builds

| # | Task | Verify |
| --- | --- | --- |
| 3.1 ∥ | `functor pr list` / `functor pr install <n>`: port `scripts/pr.mjs` and `pr-builds.mjs` to Rust in the CLI; `npm run pr` calls it. | Port the existing `test:pr` cases; recorded `gh` output. |
| 3.2 | Devices pane section (D14): list, "Install on headset", progress, "on the headset now". | Playwright with a fake build list. |
| 3.3 | Reusable GitHub Action that builds and publishes the artifacts, and the setup doc the pane links to. | Run it on this repo's own workflow. |

## M4 — voice from the headset

| # | Task | Verify |
| --- | --- | --- |
| 4.1 | Feedback capsule: the type, and `functor-app capsule --from-session` that builds one from a desktop session (timeline window, capture, poses, pointed-at object by raycast). Turning a capsule into an agent prompt with attachments. | Unit tests; a capsule built from `examples/vr-lab` under the XR emulator. |
| 4.2 | Speech-to-text on the desktop: Whisper via `whisper-rs`, model downloaded on first use to `~/.functor/models`; composer mic uses it. | Test with a checked-in short clip; model download behind a flag in CI. |
| 4.3 | Quest: `RECORD_AUDIO`, mic capture, thumbstick push-to-talk, clip + capsule data sent over the link. | Stub sends a recorded clip end to end; headset for the mic. |
| 4.4 | Headset status toast (heard → working → reloaded) driven over the link. | Capture on device or emulator. |
| 4.4a | Overlay panel (D28): the runtime draws the panel over any game on the menu button — Now and Talk first, then Tasks — fed over the link; the wrist light when closed. | Emulator captures; device when attached; `frame_bench` unchanged when the panel is closed. |
| 4.4b | Router (D29, D30): one model call per utterance splitting it into steer / new task, the decision shown with overrides on desktop and headset, steer as stop-and-resume, a task queue that starts the next task with every later utterance attached. | Core unit tests for the queue; router tests on recorded utterances; fake-agent end to end. |
| S3 | Spike before 4.5: can `/state.model` round-trip into a second session for `examples/vr-lab` and two other examples? List what breaks (closures, host handles). | Findings appended here. |
| 4.5 | Mirror view (D5): desktop draws from the headset's state with head view and switchable dev cameras (D18, via the debug camera); fall back to the headset's own capture where S3 says the model does not round-trip. | Stub feeds recorded state; frame matches a golden. |
| S5 | Spike before 4.7: two named state branches kept alive at once and switched with the code (D32), on desktop then device; what fails and what it costs in memory. | Findings appended here. |
| 4.6 | Presentation (D31, D35), format and desktop: the first page (size of the change, before/after in place, a one-paragraph Simplified Technical English summary, evidence, things to note, the decision) with the four parts as tabs behind it; the four-part format, stored with the task; before/after captures taken headlessly from the task's baseline and current files; the desktop card renders all parts; prepared when a task needs a decision. | Core and shell tests; captures match goldens for a fixture change; Playwright. |
| 4.6a | Presentation agent (D31): a separate, read-only run with its own configurable agent, model and skill, and MCP tools for reading the change, capturing either version and assembling the presentation. | Fake-agent tests through the real tool path; one live run with its token cost recorded. |
| 4.7 | Before and after in place, and the lab (D31, D32): switching versions by branch; the lab from a capsule's recorded moment; both in the headset panel. | Desktop tests under the XR emulator; device when attached. |
| 4.8 | Agents in the room (D34): the characters as a Functor scene fed by 1.17, drawn by the runtime as an overlay beside the panel; look-and-press opens a task, look-and-talk steers it. The desktop cards move to the same scene if it holds up. | Emulator captures; device when attached; `frame_bench` unchanged when hidden. |
| 4.9 | Presentation in the world (D36): notes and measured lines pinned to objects first; then two versions side by side and the ghost of the old, after S5. Drawn by the runtime as an overlay; each mark tied to its panel item. | Emulator captures; device when attached; `frame_bench` unchanged when nothing is shown. |

## M5 and later

Code city as a rendering of presentations (D33).

Developer menu and build info; agent-authored tuning UI (D7), which needs UI
render targets wired in VR and the VR gamma fix first; TTS (Kokoro) played in
the headset; focus mode; places the agent characters walk to (D34).

## Carried over from review, not yet scheduled

- Stopping a run can hold the core loop for up to 3 s while the tool server exits.
- A force-killed app can leave a hidden game running until its next write fails; the game should exit when its parent's pipe closes.
- A running agent keeps the old session address if the game restarts mid-run.
- No cap on concurrent `/stream` clients; no body cap on `/mcp`.
- The hidden game window is a fixed size, so wide views letterbox; the session should match the view.
- Pop-out windows for non-game tabs: needs the core to know a view is out in another window, and a window-closed event.
- Packaging: a `.app` bundle, signing, and shipping `functor-app` in releases.

## Risks

| Risk | Where it is retired |
| --- | --- |
| Game-view latency through capture polling | S1 |
| `mis-agent` as a cross-repo dependency (toolchain, single MCP slot) | S2 |
| Model does not round-trip for the mirror | S3 |
| adb over Wi-Fi resets on headset reboot | Accepted; cable once per reboot for APK installs only |
| Unauthenticated debug server on the LAN | Link token (2.2); device debug server stays on loopback |
| Whisper model size and first-run download | 4.2 |

## Open, with the default the plan assumes

- Several projects in a repo → project switcher.
- Chat history → per project.
- Launch → `functor app`; bare `functor` unchanged.

## Spike findings

### S1 — window and game view (2026-11-11)

Branch `spike/app-s1-window-game-view` (not for merge). `examples/hello`,
release builds, 1280×720 unless stated; another workload was running, so read
absolute numbers as ±10%.

| Route | Where | fps | p50 ms | bytes/frame | game CPU | game fps (idle ≈ 50) |
| --- | --- | --- | --- | --- | --- | --- |
| `POST /capture` polling | wry window | 37–40 | 25 | 1.1 MB PNG | 25% | 40 |
| `POST /capture` polling, 2560×1440 | bench loop | 23 | 42 | 4.2 MB | 52% | 24 |
| `GET /stream` JPEG prototype | wry window | 47–49 | 10 | 137 KB | 44% | 48 |
| `GET /stream` JPEG prototype, 2560×1440 | bench loop | 33 | 44 | 488 KB | 110% | 44 |

- **Verdict: build `/stream` (task 1.4a).** Polling meets the target at 720p
  but encodes PNG on the render thread, which slows the game itself by about a
  quarter, fails at Retina size, and makes input wait behind a capture (43 ms
  round trip against about 20 ms with the stream).
- The prototype's remaining cost is the JPEG encoder; at 1440p it is the
  limit. Encoding at view size and a faster encoder are the expected fixes
  (not measured).
- A hidden session tops out near 50 fps on every route because hidden mode
  sleeps a fixed 16 ms per frame.
- wry 0.57 and tao 0.37 run alongside the hidden GLFW child without trouble;
  plain-http loopback pages load with no extra configuration.
- WKWebView's `fetch` rejects `multipart/x-mixed-replace`; an `<img>` shows it
  at full rate.
- Input forwarding through `POST /input` works for letters, digits, arrows,
  Space, Enter and Escape and absolute mouse. Missing: modifier, Tab and
  function keys, and relative mouse (task 1.4b).
- Not checked: how input feels with a person at the keyboard, Retina size in
  the window itself, scenes heavier than `hello`.

### S2 — agent and tools (2026-11-11)

Branch `spike/app-s2-agent-tools` (not for merge); recorded event logs are in
its `app/spikes/s2-agent/events/`.

- **Go on `mis-agent` as a git dependency.** It builds with this toolchain,
  adds 5 crates not already in the lockfile, has no version conflicts, and
  costs under 10 s cold.
- **A run can call a game tool end to end.** Claude and Codex both called a
  `game_state` tool on a loopback HTTP MCP server and answered with live
  values.
- **But the call is invisible to the UI today.** `mis-agent` hides every call
  to its `mis` server from the event stream, and its Codex parser has no case
  for MCP calls. Fix upstream (task 1.6a); one server slot is then enough.
- **rmcp 2.2 has the HTTP server transport**; mounting it with a bearer check
  is about 35 lines (task 1.6).
- **Events the UI must render:** streamed text deltas then the whole text
  (Claude; Codex sends whole text only), `tool_use` / `tool_result` with no
  linking id, file edits as an `Edit` tool call, `usage`, `finished{ok,
  summary}`. Tool results carry text only, so captures are shown from the MCP
  side, not the event stream.
- **Permissions:** folder-scoped mode on the project lets the agent edit and
  call the tools without prompting. That mode also loads the user's own Claude
  settings, hooks and MCP servers; the isolated mode does not. A refused call
  comes back as an ordinary tool result. Asking the user instead needs an
  `approve` tool on our server.
- Not checked: a real refusal and ask mode, raw Codex output for MCP calls,
  image tool results.

## Agent-run metrics

`npm run metrics:app` (task 1.19; `app/metrics/`) runs each scenario in
`scenarios.json` through `functor-app task --start-session` on a fresh git
copy of its example and judges the outcome mechanically (git status, a
regex, injected `expect`s under `functor test`, `functor build`). FakeAgent
by default; `FUNCTOR_APP_LIVE=1 … --live` for the real agent, capped at 600k
input tokens and 5 minutes a scenario. `--compare a.jsonl b.jsonl` prints the
deltas.

### Live run (2026-11-12, rev `c22a0b3d`, Claude `claude-fable-5-2`, the user's own settings)

| scenario | outcome | wall s | first event s | tool calls | by kind | input tokens | output tokens |
| --- | --- | --- | --- | --- | --- | --- | --- |
| state-question | pass | 9.7 | 4.5 | 5 | game 3, skills 2 | 143,314 | 313 |
| no-tools | pass | 1.8 | 0.4 | 0 | none | 33,052 | 4 |
| weak-jump | pass | 28.1 | 0.4 | 18 | files 3, game 10, shell 1, skills 4 | 412,348 | 2,507 |
| spawn-in-chasm | pass | 44.9 | 0.4 | 21 | files 4, game 8, shell 4, skills 5 | 545,776 | 3,437 |
| two-step | pass | 33.1 | 0.4 | 19 | files 5, game 7, shell 3, skills 4 | 597,556 | 3,033 |

"First event" is the harness's init line (its startup), not the model's
first output. Input includes cached tokens. One run each: no variance yet.

- **A turn costs about 33k input.** A one-word answer with no tools is 33k;
  every further API turn resends about as much, so turns, not output, set
  the bill.
- **Tool search is a turn per lookup.** The game tools are deferred: 2–5
  `ToolSearch` calls a scenario (14 of 63 calls; "skills" in the table), each
  a turn.
- **Asking for a tool that is not there costs turns.** `task` has no view
  route, so `present`/`show_view` are not offered, but the preamble asks for
  `present`; every edit scenario searched for it 1–2 times. The app offers
  both, so this inflates the edit rows here.
- **The user's settings shape the run.** Denied shell commands ("requires
  approval", unverifiable nested commands) cost turns, and `functor test`
  could not run; the user's own instructions surfaced in answers. Their token
  share needs an isolated mode to measure.
- **Edits verify by playing.** 7–10 game calls per edit (input, step,
  capture), which is the intended behaviour, and most of the cost.

Not reported by the pinned `mis-agent` (each needs an upstream change to its
`Usage` event): cached vs uncached input (Claude's `cache_read_input_tokens` /
`cache_creation_input_tokens`, Codex's `cached_input_tokens` are summed into
`input`), usage per API message, the number of API turns (Claude's
`num_turns`; agent messages and tool calls stand in), cost (`total_cost_usd`), API time (`duration_api_ms`), and
Codex's `reasoning_output_tokens`.

### Cheaper runs (2026-11-13, Claude `claude-fable-5-2`, the user's own settings)

Three changes, measured against a baseline taken the same day:

1. **The opening instructions name only tools the run has.** `present` /
   `show_view` only with the app's view route; "session `s1`" when the app's
   game is attached (the tool server attaches it first, so it is always `s1`),
   "no game is running" otherwise.
2. **The common game tools load up front.** Claude Code defers every MCP tool
   behind `ToolSearch` unless the server marks it: in `--http` mode the tool
   server sets `_meta["anthropic/alwaysLoad"]` on ten tools (state, step,
   resume, input, capture, app context, language guide, API reference, the two
   view tools), about 1.4k tokens a turn. The instructions name them.
3. **The language guide, not the skill.** The `functor-lang` skill is 133 KB
   (~33k tokens), resent on every later turn once loaded; the instructions now
   point to `language_guide` (contents, then one section). Also: edit with the
   edit tools, not shell scripts (a denied script cost a turn).

Baseline, rev `5e49c801` (main's code):

| scenario | outcome | wall s | tool calls | by kind | input tokens | output tokens |
| --- | --- | --- | --- | --- | --- | --- |
| state-question | pass | 6.6 | 4 | game 3, skills 1 | 143,655 | 231 |
| no-tools | pass | 2.2 | 0 | none | 33,063 | 4 |
| weak-jump | pass | 28.0 | 21 | files 4, game 12, skills 5 | 374,541 | 2,664 |
| spawn-in-chasm | pass | 19.4 | 11 | files 2, game 5, shell 1, skills 3 | 295,083 | 1,630 |
| two-step | FAIL: cut over 600k input | 32.8 | 21 | files 5, game 7, shell 4, skills 5 | 694,461 | 108 |

After, rev `9d33dcaa`:

| scenario | outcome | wall s | tool calls | by kind | input tokens | output tokens |
| --- | --- | --- | --- | --- | --- | --- |
| state-question | pass | 3.2 | 1 | game 1 | 74,555 | 58 |
| no-tools | pass | 1.8 | 0 | none | 35,783 | 4 |
| weak-jump | pass | 19.0 | 13 | files 4, game 9 | 213,701 | 1,760 |
| spawn-in-chasm | pass | 14.2 | 7 | files 4, game 2, skills 1 | 211,626 | 1,168 |
| two-step | pass | 17.1 | 12 | files 4, game 6, shell 2 | 255,272 | 1,522 |

`--compare`:

| scenario | outcome | wall ms | tool calls | input tokens | output tokens |
| --- | --- | --- | --- | --- | --- |
| state-question | pass → pass | 6635 → 3234 (-51%) | 4 → 1 (-75%) | 143655 → 74555 (-48%) | 231 → 58 (-75%) |
| no-tools | pass → pass | 2240 → 1764 (-21%) | 0 → 0 | 33063 → 35783 (+8%) | 4 → 4 (+0%) |
| weak-jump | pass → pass | 27972 → 18951 (-32%) | 21 → 13 (-38%) | 374541 → 213701 (-43%) | 2664 → 1760 (-34%) |
| spawn-in-chasm | pass → pass | 19434 → 14230 (-27%) | 11 → 7 (-36%) | 295083 → 211626 (-28%) | 1630 → 1168 (-28%) |
| two-step | fail → pass | 32801 → 17055 (-48%) | 21 → 12 (-43%) | 694461 → 255272 (-63%) | 108 → 1522 (+1309%) |

All five: 1.54M → 0.79M input (-49%). The fixed cost rose 2.7k (+8%): the
up-front schemas and the longer instructions. One run each; an intermediate
pass (changes 1–3 without `api_reference` up front or the no-game note) gave
1.02M, so run-to-run spread is wide.

- **Search turns: 14 → 1.** The one left looked up `rewind` and
  `reload_project`, which stay deferred.
- **No session lookup.** `list_sessions` + `app_context` before every first
  game call are gone (`s1` is named).
- **two-step no longer loads the skill**, which is most of its drop and why it
  now finishes under the cap.
- **Still waste, not removed here:** `functor test` is denied (no allow rule
  for it, D19), 1–2 turns per edit that writes tests. A test tool on the tool
  server, or an allow rule for `functor test`, would remove them.
- **An isolated-settings mode** (not here: it changes D19) would cut the
  user's own instructions, skill list and memory from each request: in a
  captured request (no tokens spent) they were 28k of 83k characters, about a
  third, so roughly 7–10k of each turn's ~35k. It would also drop the denials
  their settings cause.
- **Upstream (`mis-agent`, not changed):** its `--mcp-config` entry could set
  Claude Code's per-server `alwaysLoad`, but that loads every tool (~6k
  tokens a turn); the per-tool `_meta` mark the server sets is the finer
  control and needs no change there. Its `Usage` event still lacks the split
  listed above, so cached input cannot be told apart.

### Reviews (4.6a, 2026-11-13, rev `c4436f3c`, the review on `sonnet`)

When a run ends with a change for the user (ready for review, tracked), a
separate review run prepares the decision page's report (D31):

- **Read-only by the harness, not by asking.** Claude: `mis-agent`'s
  read-only mode (`dontAsk`: what no rule allows is denied; the only rule is
  the review's tool server; the user's settings are not loaded, as their
  rules could allow writes, so reviews are isolated from D19). Checked live:
  `Edit`, `Write` and a `sed -i` through `Bash` were each denied, the file
  unchanged. **Claude only:** the pinned `mis-agent` cannot force Codex's
  sandbox read-only (its config may choose another, or approve other MCP
  servers), so a Codex task's review runs on Claude and a setting naming
  Codex is refused. The tool server runs `--review`: the language and API
  tools and its own two, no game (launching one can rewrite a stale
  `assets.fun`) and never the user's (`--attach` is refused). The report is
  held until the review ends and the files are measured: a change since it
  started (an edit anyway, or the user's) voids it.
- **Tools:** `task_change` (the app's brief: request, the run's calls and
  last words, files, evidence so far, the agent's report, the diff up to
  48 KB) and `report` (summary, notes, evidence, highlights when the agent
  gave none), both loaded up front.
- **Setting per project** (`review.json`; the composer's "Review" panel): on
  by default; model `sonnet` (cheaper and faster than the task's); optional
  instructions. Off: the agent's own report is shown.
- **Precedence:** the review's report replaces the agent's and is labelled
  "Prepared by a separate review"; the agent's failed checks and the app's
  own path evidence stay. While it prepares, the page stands on what it
  has, with "Preparing the summary…"; Keep and Revert work at once and stop
  it. A failed or cut review leaves the page as it was and says why. A new
  run, Keep, Revert or a different change ends it.
- **Bounds:** one per change; cut at 300k input tokens or 180 s.
- **Style check:** sentences over 20 words, over 120 words, or more than one
  paragraph are flagged on the page and in `report`'s answer; never blocked.

The working runs, then the reviews of the three that changed a file:

| scenario | outcome | wall s | tool calls | by kind | input tokens | output tokens |
| --- | --- | --- | --- | --- | --- | --- |
| state-question | pass | 5.9 | 1 | game 1 | 76,393 | 58 |
| no-tools | pass | 7.5 | 0 | none | 37,709 | 4 |
| weak-jump | pass | 27.5 | 13 | files 4, game 9 | 213,217 | 1,686 |
| spawn-in-chasm | pass | 22.1 | 6 | files 3, game 3 | 171,132 | 1,126 |
| two-step | pass | 25.3 | 12 | files 4, game 5, shell 3 | 215,616 | 1,629 |

| review of | outcome | wall s | input tokens | output tokens | style flags |
| --- | --- | --- | --- | --- | --- |
| weak-jump | prepared | 25.7 | 111,990 | 974 | 0 |
| spawn-in-chasm | prepared | 18.3 | 112,336 | 939 | 0 |
| two-step | prepared | 9.5 | 82,944 | 838 | 0 |

- **A review costs about half its run:** 83–112k input (52% of the run's on
  average), 9.5–25.7 s, all well under the bounds. Its turns are cheaper
  (no user settings), but it still reads the brief and the files.
- **Every report was concrete:** 4–5 notes each, naming what nobody ran
  (`functor test`, the jump from the new spawn), and only checks that ran,
  credited to the agent. None launched a game.
- One run each, at `c4436f3c`. Since then the report waits for the
  review's end and the game tools are gone; neither changes what these
  reviews did. Not measured: a review cut by its bounds.
