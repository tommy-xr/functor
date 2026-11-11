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

- **D16 — the game stays visible while reading code.** The Code and Diff tabs
  offer the game side by side, as a draggable mini view, or hidden.

- **D17 — the game view says little.** One small context banner ("Click to
  play" / "Esc to release" / "Mirrored from headset"); timeline marks explain
  themselves in tooltips, not a legend. Headset state lives only in the title
  bar pill.
- **D18 — dev cameras on the mirror.** Because the desktop draws the mirror
  from state, it can draw it from any viewpoint: head view, or cameras parked
  in the world and switched between while playing in the headset.

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
  explorer; the app opens on the project nearest the start directory.
- Chat history is kept per project.
- The app starts with `functor app`; bare `functor` keeps printing help.

## Open

- In-headset agent presence (the original's open issues) — after M4.
- Whether commit / branch / PR appears in the developer menu — likely yes, as
  part of the build-info panel in M5.
