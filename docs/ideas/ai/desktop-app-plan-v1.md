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
  HTTP as well as stdio (task 1.6) so they fit that one-server slot.
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
| 1.4 | Game view: show the session's frames (per S1), forward input on focus, "Running in this window" picker, "Open in window". | Playwright against a `--fixed-time` session: the canvas shows a non-blank frame that matches a `/capture`. |
| 1.5 | Transport: reuse `runtime/functor-runtime-web/scrubber.js` and `timeline-model.js` against the native session's `/time`, `/rewind`, `/input-log`. No redrawn copy. | Existing `timeline-model` tests still pass; Playwright: pause, step, rewind change `/state.frame`. |
| 1.6 ∥ | `functor mcp --http <port>`: the same tool router over rmcp's HTTP transport with a bearer token. Add an `app_context` tool the core fills in (open view, selected file, active session). | Existing MCP tests on both transports. |
| 1.7 | Agent runner: a task = one conversation; run through `mis-agent` with the session attached via 1.6; stream events to the UI; Stop; persist tasks and transcripts per project under `~/.functor/app/`. A `FakeAgent` that replays recorded events drives the tests. | Unit tests with `FakeAgent`; one opt-in live test. |
| 1.8 | Chat UI: composer (D15) with context chips, steps, cards for captures and changes with "open in canvas", result pill. | Playwright with `FakeAgent`. |
| 1.9 | Changes: per-task diff of the working tree, diff view in the canvas, Keep / Revert. | Test on a temp git repo. |
| 1.9b | Code view with the game (D16): the Code and Diff tabs can keep the game visible, side by side or as a draggable mini view, or hidden; the choice is remembered. The game element is the same one as 1.4, moved, not a second stream. | Playwright: each mode shows a live frame beside the code; one stream open. |
| 1.10 | Tasks view (D12): grouped list, task page with what the agent was given, steps, change, before/after captures. | Playwright with `FakeAgent`. |
| 1.11 | Window and launcher: wry window around the served page; `functor app` finds and starts `functor-app`; macOS only. | `functor app --headless --print` in CI; window checked by hand once. |
| 1.12 | Docs page for the app, and `pr-visuals` captures of the three views. | — |

## M2 — devices and the Wi-Fi link

| # | Task | Verify |
| --- | --- | --- |
| 2.1 ∥ | `functor devices` (text and `--json`): adb wrapper listing headsets with link, battery and build. | Tests against a fake `adb` on `PATH`. |
| 2.2 | Device link, desktop side: WebSocket listener with token auth, request/response tunnelling of debug-runtime calls, a local port that proxies to the device through the link, mDNS advert. A loopback **device stub** binary speaks the device side for tests. | Tests: stub connects, `curl` to the proxied port reaches the stub; bad token refused. |
| 2.3 | Device link, Quest side: read the pairing file, find the desktop by mDNS, dial with the token, serve tunnelled requests into the existing debug handler, reconnect with backoff. | Stub-level tests in common code; on-device check per the `vr-device-loop` skill. |
| 2.4 | Pairing: on cable attach, write token and desktop address to the headset over adb and enable adb over Wi-Fi. Devices pane shows link, cable, battery, build, Forget. | Fake `adb` tests; headset when attached. State in the PR when none was. |
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
| S3 | Spike before 4.5: can `/state.model` round-trip into a second session for `examples/vr-lab` and two other examples? List what breaks (closures, host handles). | Findings appended here. |
| 4.5 | Mirror view (D5): desktop draws from the headset's state with head view and switchable dev cameras (D18, via the debug camera); fall back to the headset's own capture where S3 says the model does not round-trip. | Stub feeds recorded state; frame matches a golden. |

## M5 and later

Developer menu and build info; agent-authored tuning UI (D7), which needs UI
render targets wired in VR and the VR gamma fix first; TTS (Kokoro) played in
the headset; focus mode; agents visible in the world.

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
