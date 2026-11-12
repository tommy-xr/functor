# Desktop app — revisions v1

AI-assisted revisions to [`../desktop-app.md`](../desktop-app.md), which stays the
original statement of intent. Where the two differ, this file records the change
and why.

Status: design settled for a first build; see `desktop-app-plan-v1.md`.

## Decisions

| # | Topic | Original | Revised | Why |
| --- | --- | --- | --- | --- |
| D1 | UI stack | Rust + gpui | Rust core + web UI in a native shell (wry) | MIS theming is CSS; reuses the site IDE's editor, scrubber and player; the same UI opens from the Quest browser. gpui would mean rebuilding chat, diff and editor widgets and bridging the OpenGL game view. |
| D2 | First milestone | (none) | Desktop chat + game view | Verifiable end to end without a headset. |
| D3 | Agent runner | "reuse some MakeItSo modules" | Standalone app; depend on the `mis-agent` crate (git dependency) and spawn `claude` / `codex` directly | One project, a few conversations. Fleet orchestration stays in MakeItSo. |
| D4 | Quest ↔ desktop transport | WebRTC | WebSocket opened by the device to the desktop core (`adb reverse` on USB, LAN address on Wi-Fi) | Push-to-talk is short clips, not a live call. WebRTC is revisited only for live eye-streaming. |

## Shape

```
              ┌────────────── functor app core (Rust, headless) ──────────────┐
web UI  ◄─ws─►│ projects · game sessions · agent (mis-agent) · devices · PRs  │◄─ws─► Quest runtime
(wry window   │ voice (STT/TTS) · feedback capsules                           │
 or browser)  └───────────────┬───────────────────────────────┬───────────────┘
                              │ debug runtime (HTTP)          │ functor mcp (stdio)
                              ▼                               ▼
                        game session(s)                 claude / codex
```

- **Headless core first.** Everything the UI shows is a JSON API on the core, and
  the core has a text mode (`functor app --headless`, `--print`). Each pane is
  checkable from the CLI before any UI exists.
- **The UI is a view.** State lives in the core; the page renders it and sends
  intents. A reload of the page loses nothing.
- **The agent sees the game through `functor mcp`**, already shipped: model,
  capture, pause/step, input, hot reload. The app adds context about where the
  user is (open view, selected file, active session) as an injected prompt
  preamble or skill.
- **Separate crate and binary** (`app/`), launched by `functor app` (and by bare
  `functor` in a project directory). The CLI build does not grow a webview
  dependency.

## Revisions by section

### User journey

Unchanged in intent. Layout: see D10–D12.

### Theming

Lift the MIS CSS tokens (dark and light sets) into a shared stylesheet rather
than restating them. The marketing site keeps its synthwave look; the app is a
tool and follows MIS.

### Working area

Views in delivery order: game, code / diff, tasks. Code city stays out of scope
(see the original's "Out of scope").

### Developer menu

Depends on work that has not landed: UI render targets in VR, and the VR gamma
fix. Scheduled after the voice loop.

### Use case 1 — editing in the world

Added definitions:

- **Feedback capsule** — the unit sent to the agent for one spoken remark. Plain
  serialisable data: audio clip, transcript, a timeline window around the
  recording (frames before and after), a stereo capture, head and controller
  poses, and the object the user pointed at. Built from the existing recorder
  and debug runtime, so a capsule can be produced and tested from the desktop.
- **Audio path** — the Quest records and ships raw audio; speech-to-text runs on
  the desktop (reading of the original's line 68). Quest mic capture is new
  work: no `RECORD_AUDIO` permission or capture path exists today.
- **Models** — Whisper large-v3-turbo (quantised) for STT, Kokoro for TTS,
  downloaded on first use into `~/.functor/models`.

### Use case 2 — devices

Built CLI-first (`functor devices` — list, state, Wi-Fi adb, forward/reverse),
with the pane on top.

### Use case 3 — PR artifacts

Port `npm run pr` into the Rust CLI (`functor pr list | pull`) so other
repositories get it without this checkout, plus a reusable GitHub Action that
publishes the artifacts. The pane shows download and hot-reload progress.

## Milestones

Each is a stack of small PRs; each leaves the app working.

| M | Delivers | Verified by |
| --- | --- | --- |
| M1 | Core + shell: open a project, run a game session, chat with an agent that has the session attached, game view, diff view | Headless core tests; scripted UI run (Playwright against the served page) |
| M2 | Devices: cable pairing, Wi-Fi device link, tunnelled debug server, run on headset | CLI against a fake `adb` and a loopback device stub; headset when attached |
| M3 | PR artifacts: list, pull, hot reload; reusable Action | CLI against recorded `gh` output |
| M4 | VR voice loop: push-to-talk, STT, feedback capsule, agent edit, in-headset status; mirrored game view from headset state | Desktop-synthesised capsules; headset for the mic path |
| M5 | Developer menu and agent-authored tuning UI | Captures; headset |
| Later | TTS replies, agents visible in the world, code city | — |

## Decisions, second round

- **D5 — who owns the game state.**
  - *Local:* the core runs the game as a hidden native session and the canvas
    streams its frames and forwards input. "Open in window" gives a real
    window.
  - *Headset connected:* the headset owns the state. It sends its state (model,
    head and controller poses) over the device link, and the desktop draws a
    mirror from it — `draw` is a pure function of the model, so the desktop
    needs no simulation of its own. The mirror offers the head view or a free
    spectator camera.
  - Risk to settle early in M4: a model holding closures does not survive the
    trip as plain data. Fallback for such games is streaming the headset's own
    eye capture.
- **D6 — macOS first.**
- **D7 — tuning values round-trip through the agent.** The agent writes ordinary
  `ui` code; tuned values live in the model; "apply" sends them back to the
  agent, which edits the constants. No new language construct in v1.
- **D8 — TTS follows the first voice loop.** Until then the headset shows a
  status toast: heard → working → reloaded.
- **D9 — the device link is Wi-Fi first, paired once over the cable.**
  - Plugging the headset in pairs it with no prompts: the app writes a token and
    the desktop's address to the headset over adb, and turns on adb over Wi-Fi.
  - From then on the headset dials the desktop over Wi-Fi with that token,
    finding it by mDNS, so a changed IP address needs no action.
  - Everything the dev loop needs travels on that one link: source and asset
    pushes, state, captures, audio. A cable that drops therefore interrupts
    nothing; the cable is only a fallback when Wi-Fi is unreachable.
  - The headset's debug server stays on loopback. The core exposes it locally
    by tunnelling requests through the link, so existing tools (`functor mcp`,
    the SDK, `curl`) work unchanged.
  - Known limit: adb over Wi-Fi resets when the headset reboots. Installing a
    new APK after a reboot needs the cable once; the app re-enables it the next
    time it sees the cable. Hot reload and voice do not depend on adb.

## Decisions, third round (from the mockups)

Three layouts were mocked: a workbench, a chat-first layout and a game-first
layout. The workbench was chosen.

- **D10 — layout: workbench.** Title bar (logo, project, branch and commit,
  session and headset status) → rail → side pane → canvas → chat docked right.
- **D11 — the rail is Explorer, Tasks, Devices.** "Play" is not a destination:
  where the game runs is a picker above the game view.
- **D12 — a task is a conversation.** Tasks lists every conversation with the
  agent, grouped as ready for review, in progress and waiting on you; a task
  ends when its change is kept or dropped.
- **D13 — the transport is the web player's.** Same glyphs, colours and
  behaviour as the web scrubber (pause/step, input and reload marks, debug
  camera, extrapolation), reused rather than redrawn.
- **D14 — pull-request builds are "install on headset"**, with one line saying
  what installing does and a setup link when a repo publishes no builds.
- **D15 — composer** follows the current chat-app pattern: one rounded
  container, attach and model picker on the left, mic and send on the right.

- **D16 — the game stays visible while reading code.** *(Superseded by D20
  and D21.)* The Code and Diff tabs
  offer the game side by side, as a draggable mini view, in its own window, or
  hidden, chosen from a button strip in the tab bar.

- **D17 — the game view says little.** One small context banner ("Click to
  play" / "Esc to release" / "Mirrored from headset"); timeline marks explain
  themselves in tooltips, not a legend. Headset state lives only in the title
  bar pill.
- **D18 — dev cameras on the mirror.** Because the desktop draws the mirror
  from state, it can draw it from any viewpoint: head view, or cameras parked
  in the world and switched between while playing in the headset.

- **D19 — agent runs load the user's own agent settings** (their Claude
  settings, hooks and tool servers), as in the terminal. An isolated mode is a
  later option.

Taken from the other two layouts:

- Captures, diffs and voice capsules appear as cards in the chat, each with
  "open in canvas".
- The composer shows what the agent can see as removable context chips.
- Connection events ("headset reconnected over Wi-Fi") appear as lines in the
  chat.
- A working agent has a Stop button; a capsule can be replayed on the desktop.
- Later: a focus mode where the game fills the window and the composer floats.

Proposed, awaiting confirmation:

- A repo with several projects gets a project switcher at the top of the
  explorer; the app opens on the project nearest the start directory. (See D23.)
- Chat history is kept per project.
- The app starts with `functor app`; bare `functor` keeps printing help.

## Decisions, fourth round — canvas, game view, projects

These replace D16 and the fixed Game / Code / Diff tabs of D10's canvas. The
title bar, rail, side pane and chat dock are unchanged.

### D20 — the canvas is panes of tabs

- **A view is a tab.** View types: the game, a file, a task's changes, a task
  page, and later a visualizer. New view types need no layout work.
- **A pane holds tabs.** Each pane's tab bar ends with four actions: open a
  view here, split right, split down, pop out into its own window.
- **Where a view opens.** In the focused pane by default. "To the side" opens
  it in the neighbouring pane, creating one with a split right if there is
  none. A view that is already open is focused, not duplicated.
- **Closing.** Closing a pane's last tab removes the pane. An empty canvas
  offers three actions: start the game, open a file, start a task.
- **From the chat.** "View diff" and "Open in canvas" open or focus the
  matching tab; they never replace what the user is looking at in another
  pane.
- **Layout is state.** The core holds it per project, so it survives a
  reload and a restart. There is a "reset layout" action.
- **Shortcuts (proposed):** split right, split down, close tab, focus the
  game, focus pane *n*, and "open to the side" on a file in the explorer.

### D21 — one game view, four placements

There is one game view per project. Choosing a placement moves it; it is
never shown twice.

| Placement | Where the game runs | What the desktop shows | Input |
| --- | --- | --- | --- |
| Pane | Hidden local session | A Game tab in a pane | Click to play, Esc to release |
| Mini | Hidden local session | A small floating view over the canvas; drag to move, resize from a corner | Same, while focused |
| Own window | A normal game window | A one-line note in place of the view | In that window |
| Headset | On the headset | The mirror, as a Game tab or mini (D5, D18) | In the headset; the desktop drives only its cameras |

- **The control.** One placement control on the game view (in the tab bar
  when it is a tab, in the title strip when it is mini), next to Start game /
  Stop game. Headset is offered only while a headset is connected.
- **Pane ↔ Mini is instant.** Same session, same stream; the view moves.
  "Dock" returns a mini view to the pane it came from.
- **Own window and Headset change where the game runs.** Today that restarts
  the session, so the game starts from its initial state. Carrying the state
  across is a later improvement; the model being plain data makes it possible.
- **One banner** (D17): "Click to play", "Esc to release", or "Mirrored from
  headset".

Flows:

1. **At the desk.** Start game → the Game tab opens in the focused pane. Open
   a file to the side → code and game side by side. Ask the agent for a
   change → the game reloads in place → a Changes tab is offered from the
   chat.
2. **Reading code.** Switch the game to Mini → the pane is free for code and
   the game keeps running in the corner. Dock it to bring it back.
3. **Second monitor.** Pop the game out to its own window, or pop out any
   other tab (a file, the changes) and keep the game in the app.
4. **Into VR.** Connect a headset → choose Headset → the game runs on the
   device and the desktop view becomes the mirror, with the head view or a
   parked dev camera. Spoken feedback arrives in the chat as capsules, the
   agent works, and the headset reloads. Choosing Pane afterwards brings the
   game back to the desktop.

### D22 — one place to start and stop the game

"Start game" is the game view's empty state; "Stop game" sits beside the
placement control. The explorer's session card shows status only. "Stop" in
the chat always means the agent.

### D23 — the app opens any folder

A project is a folder, not a place in a repository: a subfolder of a repo, a
loose folder, or the root of a new repo all work. The app keeps a list of
recent projects. A repo with several projects still gets the project
switcher.

### D24 — new project

A modal, opened from the project switcher:

- **Name and Folder** (a path with Browse). Any folder; created if missing.
- **Track changes with git** — offered only when the folder is not already
  tracked. Reviewing and reverting the agent's changes depends on it.
- **Start from** — a gallery that scrolls, with search and tag filters.
  Starters (blank, 3D, first person, 2D, multiplayer, VR) and complete
  example games. A template is an ordinary project folder plus a small
  manifest (name, description, tags, preview), so adding one needs no app
  change. Templates are local for now.
- **Describe the game (optional)** — becomes the project's first task, built
  from the chosen template. This covers genres with no template.
- **Create and start.** The game always starts; there is no option not to.

## Open

- In-headset agent presence (the original's open issues) — after M4.
- Whether commit / branch / PR appears in the developer menu — likely yes, as
  part of the build-info panel in M5.
