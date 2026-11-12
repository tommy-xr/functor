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

### D25 — the agent can drive the view

The layout is core state (D20), so the agent can change it through tools, the
same way it drives the game.

- **Tools.** Open or focus a view (a file at a line range, a task's changes
  at a file, the game, a task page); arrange it (to the side, split);
  highlight lines; put the game at a moment (pause, a frame, a dev camera).
  The agent reads the current layout through `app_context`.
- **Walkthroughs.** The agent's main use is showing its work: an ordered list
  of steps ("the change in `ladder.fun`", "the climb at the top rung, paused")
  shown in the chat as one card the user steps through, and can replay.
- **It never takes the wheel.** Agent-opened views go to a pane the user is
  not typing in; the focused tab and the text cursor are never moved; the
  agent cannot close the user's tabs; "restore my layout" undoes everything
  it arranged.
- **Automatic by default.** When a run finishes, the first step of its
  walkthrough is applied without a click; a setting turns that off. To be
  judged in use.

## Decisions, fifth round — the headset

These extend D5 (the headset owns the game state), D8 (status toast), D9 (the
device link) and D21 (the Headset placement).

### D26 — pairing needs no cable

- The Functor app on the headset finds desktops on the Wi-Fi and shows a code.
  The desktop shows the same code with "a headset wants to pair"; the user
  clicks Allow. Nothing is typed in the headset.
- After pairing the headset reconnects by itself (D9).
- The cable still works and does more in one step: it installs the headset
  app, pairs, and enables updates.
- Limit: the headset app must be installed before it can pair. A first install
  is a sideload over the cable until there is a store listing. Updates after
  pairing travel over the link; the headset asks the user to confirm.

### D27 — connection is a small set of states, each with one fix

Not paired · asking to pair · paired but away · connected (Wi-Fi or cable) ·
needs an update · cannot reach. The title-bar pill shows the state's word
everywhere; the Devices pane shows the detail (link quality, battery, the
build running) and the one action that fixes the state.

### D28 — the in-headset panel is drawn by the runtime

An overlay the runtime draws over any game, opened by the menu button; the
game's code does not know about it. Sections: Now (the task in front of the
user), Tasks (waiting on you, in progress, ready to review), Build, and a way
through to the game's own menu. With the panel closed, one small light on the
wrist carries the status: working, needs you, reloaded, not connected.

Agent-built tuning UI (D7) stays game-authored; the task panel does not.

### D29 — a router decides what speech becomes

- One small, fast model call per utterance splits it into parts and labels
  each: steer the running task, or a new task.
- The decision is always shown, with a one-tap override in each direction.
- Steering a running task means stopping the run and resuming it with the new
  message, since the agent CLIs take no input mid-run.
- A new task queues behind the running one (one run per project at a time).

### D30 — queued tasks start by themselves

Bias to action. When a queued task is about to start, it is given its
original request plus everything the user has said since, in order, so a
later change of mind is honoured — including dropping the task.

### D31 — a presentation when the user must decide

Whenever a task needs a decision — a change ready to keep or revert, or a
question from the agent — the user gets a prepared presentation: everything
needed to say yes or to give feedback. It is not produced for changes that
need no decision.

Four parts, any of which may be absent:

1. **Highlights** — the change and, where it matters, its architecture, in a
   few points. Each links to its code spot for drilling in.
2. **Before and after captures** — the same camera and moment on both
   versions.
3. **A lab** — when the change needs to be felt: the moment the user gave
   feedback, replayed in the new version; or, when there is no such moment, a
   small test scene built for the change.
4. **The real before and after** — switching the running game between the two
   versions (D32).

- **One format, several renderings:** the walkthrough card on the desktop
  (D25), pages in the headset panel, and later a code-city view (D33).
- **A separate presentation agent** prepares it, with read access to the code
  and tools to read the change, capture frames from either version, and
  assemble the result. Which agent, model and skill it uses is configurable
  per project, with a built-in default.
- **The tools are ours.** Building a presentation — and the visualisations in
  it — goes through MCP tools the tool server exposes, so any agent can do
  it. A small Functor helper for authoring presentation scenes is a likely
  shape for the visual parts.

### D32 — before and after by branching state

Each version keeps its own state, so the old code only ever sees state the
old code produced. Switching swaps the code and the branch together; coming
back restores the other branch exactly. "Before" starts from the last state
the old code produced, or from the lab's recorded moment. Built on the
recorder's existing rewind and branch; needs a spike to confirm on device.

### D33 — code city as a later rendering

A Functor-specific code-city view is a third rendering of the same
presentation: the buildings a change touched light up and the user steps
between them. In VR it is far simpler than on the desktop — the changed
modules and their neighbours at table scale.

## Decisions, sixth round — seeing the agents

### D34 — agents are characters you can read at a glance

Each running agent is drawn as a small character, so the state of the work is
visible without reading a transcript. The same four signals appear on the
desktop and in the headset; only the staging differs.

**What a character shows**

- **Who it is.** One character per task. Claude and Codex differ in silhouette
  and colour, not only in a label. A subagent is a smaller figure tied to the
  agent that started it.
- **What it is doing: a thought bubble.** An icon for the kind of call plus one
  short line ("reading `weapons.fun`", "running tests"). The kinds are a small
  fixed set: thinking, files, shell, network, game (our own game tools),
  agents, skills, asking you. A shell command is sorted by what it touches, so
  `cat` is files and `curl` is network. Calls running together show a count. A
  refusal or error turns the bubble red, and the mark stays until the user's
  next message.
- **How much room it has left: a bar.** How full the context window is, in the
  manner of a health bar: amber at 70%, red at 90%.
- **Its stance.** Thinking, working, waiting for you, done, failed. "Waiting
  for you" must be the easiest to spot, from across the window or the room.

**Desktop.** The character is a compact card: avatar, bubble, bar. Cards sit in
the Tasks pane and the running task's card rides in the chat dock, so it is in
view whichever layout is open. Selecting a card opens that task. No animation
runs while nothing is happening.

**Headset.** The same characters stand in the room at table scale, beside the
panel (D28) and never inside the game's scene: the runtime draws them as an
overlay, as it does the panel. The bubble and bar face the user. Looking at a
character and pressing opens its task in the panel; holding to talk while
looking at it steers that task (D29). The wrist light is the same stance signal
when the room view is closed.

**One scene, two stagings.** The characters are a Functor scene fed agent
events as messages, so the desktop card strip and the headset room are two
cameras on one model, and the engine is exercised on something real. The first
version is flat cards; places the characters walk to (a bench for files, a
terminal, the game) come only once the event feed is proven.

**What it needs.** The app already has every tool call per task and which
engine runs it. Still missing from the event stream: which agent a subagent's
calls belong to, and how full the context is (only running token totals arrive
today).

The vocabulary — call kinds, the context thresholds, clock marks for long
calls, a sticky error mark — follows
[lot-map](https://github.com/alexeyw/lot-map), which draws one agent as a map
of places. Here there are several agents at once, so the agents are the
characters and the kinds are what appears over their heads.

### D35 — the decision page answers at a glance

Refines D31. Easy decisions are approved from the first page without opening
anything; hard ones are drilled into. So the first page of a presentation
carries the whole case in small, and the four parts are depth behind it.

**The first page shows, in this order**

- **Size of the change.** Lines added and removed, the number of files, and
  which kind they are (game logic, assets, engine). A small change is easier
  to approve, or to approve on evidence alone.
- **Before and after.** The one image pair or short animation that shows the
  change best, in place — not behind a tab.
- **A one-paragraph summary** in Simplified Technical English (ASD-STE100):
  short sentences, one meaning per word, active voice, no idiom.
- **Evidence.** What was run and the result: checks, tests, captures compared.
- **Things to note.** What was not verified, what was assumed, side effects,
  anything the user would want to know before saying yes.
- **The decision.** Keep, Revert, or say what is off — always on this page.

**Depth is tabs 1 → 2 → 3 → 4.** The four parts of D31 are tabs in order of
effort: Highlights, Captures, Lab, Real before and after. Each item on the
first page opens the tab that backs it — the size opens Highlights at the
code, the image opens Captures, a note opens the part it is about — so the
user can go deeper from the thing they doubt, or walk the tabs in order.

**The presentation says how deep to go.** It marks the change as one to
approve at a glance or one to look into, with the reason ("12 lines in one
file, tests pass" or "changes how the grip feels: try the lab"), and opens on
the suggested tab only when the user asks to look.

The same page is the task's card on the desktop and the first panel page in
the headset.

### D36 — in the headset, a presentation can be shown in the world

Some changes are judged with the eyes or the hands, not on a panel. For
those, an item of the presentation may also be shown at the place or on the
object it is about.

- **Two versions side by side.** The old and the new — two models, two
  weapons, two ladders — stand next to each other in front of the user, each
  labelled, each usable. The user compares by looking and by picking each up.
  This is the lab (D31) without a screen.
- **A ghost of the old.** Where two objects cannot stand apart (a grip pose, a
  holster position), the old version is drawn translucent on the new one.
- **Notes pinned to the thing.** A highlight or a thing to note sits at the
  object it concerns; a measured line shows a distance that changed.
- **The decision may be an act.** Taking the version you prefer keeps it. The
  panel's Keep and Revert always work as well.

Rules:

- It is a property of an item, not a mode. The panel item and its place in
  the world highlight each other, and the panel alone is always enough.
- Only for items about a place or an object. A rule change or a refactor
  stays on the panel, with the size, the evidence and the summary.
- The runtime draws it over the game (D28); the game's own scene and state
  are untouched. The old version is the real old code running (D32), never an
  approximation.
- Few at a time: no more than a handful of marks in view.
- The presentation agent chooses per item, through the same tools (D31).

Pinned notes and measured lines need only a point or object to attach to and
come first; side-by-side versions and ghosts wait on the branching spike (S5).

### D38 — the extrapolation is before-and-after evidence

The runtime can already draw where the game goes next under the current code:
the extrapolated path from a moment, with the recorded inputs. Run from the
same moment under the old code and under the new, that is a before and after
in one picture, with no second game kept alive.

- **In the transport bar.** An Extrapolate control beside pause and step shows
  the predicted path in the game view. When an edit lands while paused — the
  user's or the agent's — the path is shown for a moment by itself, so the
  effect of the edit is seen before anything is replayed.
- **In the chat.** When a run changes the code, the change card carries the
  two paths from the moment the user was looking at: the old one faint, the
  new one bright ("fell short" beside "clears the gap"). It is the default
  before-and-after on the decision page (D35) whenever the change moves
  something, and it opens the Captures tab.
- **Same moment, same inputs, both codes.** The picture is honest only if the
  two paths differ by the code alone; it says which moment and which inputs
  it used.
- **Limits.** It shows motion and position, not feel, sound or anything a
  path cannot draw; those still need captures, the lab or the real before and
  after (D31, D32). A change that moves nothing gets no path.

In the headset the two paths are drawn in the world (D36).

## Open

- In-headset agent presence (the original's open issues) — after M4.
- Whether commit / branch / PR appears in the developer menu — likely yes, as
  part of the build-info panel in M5.
