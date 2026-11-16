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

*Revised 2026-11-13 (1.4b): while the game has input, every key it knows
goes to it — Esc, Tab and the function keys too — and ⌘ is the app's: ⌘
chords never reach the game, and ⌘Esc releases ("⌘Esc to release"). A game
that captures the mouse gets the pointer locked on Click to play. Focus
leaving the view, the window or the tab, or an app dialog opening, releases
everything held.*

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

*Revised 2026-11-12 by D37: no empty state to start from; the game opens
paused on its first frame.*

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

### D37 — the game opens paused on its first frame

*2026-11-12. Replaces D22's "Start game" empty state.*

Opening a project — at app start, from a folder or a recent project, by
switching projects, or after creating one — starts its game, hidden and
paused on frame 0: after `init`, before any tick moves time. The game view
shows that frame; the transport bar shows it paused at the start. Play runs
it from there. The game view has no Start prompt.

- **One start/stop control** stays beside the placement control (D22):
  **Stop game** ends the process and frees what it holds; when stopped,
  **Start game** brings it back paused on frame 0. Play and pause are the
  transport bar's.
- **Own window** is not started on open, and runs at once when started: that
  window has no transport bar.
- **A live game in a pane keeps its tab**, so opening a project brings its
  Game tab back if it was closed.
- **A game that fails to load** shows its error where the frame would be;
  saving a project file starts it again.
- **Cost:** one hidden game process per open project, from the moment it
  opens. Switching projects stops the previous one; closing the app stops it.
- **The agent** is told the game starts paused; its tools step or resume it.
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

### D39 — how much of your own setup an agent run loads is a choice

Extends D19, where every run loads the user's own agent settings. That is
right for some work and waste for other work, so it becomes a setting with
three levels:

- **Yours and the project's** — the user's global instructions, skills and
  memory, plus the project's own. The agent works the way the user works.
- **The project's only** — what is checked into the project (its instructions
  and skills), nothing personal. The same result for anyone who opens it.
- **Isolated** — neither: only what the app gives the run (its opening
  instructions and tools). The cheapest and the most repeatable.

Where it is set:

- **Per project, with a default** of "yours and the project's", as today.
- **Per task**, from the composer, for one request.
- **Per kind of run.** The presentation agent (D31) has its own level, and
  defaults lower than the working agent: it reads and reports, and does not
  need the user's habits.

What it changes is what the run reads, never what it may do: permissions stay
scoped to the project folder at every level. The task records the level it
ran with, and the metrics record it too, so the cost of each level is known
rather than guessed (one turn is about 35k input tokens today, of which the
user's own setup is roughly a quarter).

### D40 — one binary

The app and the CLI become one executable, `functor`. Today they are two, and
the app runs whatever `functor` it finds; when that one is older, the game
and the agent's tools fail to start with errors about arguments.

- **`functor` with no arguments opens the app, in-process.** The game
  sessions and the agent's tool server are the same executable started again
  (`run native …`, `mcp …`), so the app and its CLI cannot differ.
- **The app is a library the CLI links,** behind a build feature: a CLI-only
  build stays possible for CI and for platforms without the window.
- **Releases carry the app** because they already carry `functor`.
- **On macOS the game child says it is a background process** when it starts,
  so it takes no Dock tile of its own and "Quit" quits the app, also inside
  the `.app` bundle, which then holds one executable.
- **Elsewhere,** where there is no window yet, `functor` with no arguments
  keeps printing help, or serves the page to a browser.

Until then the app checks the `functor` it found before using it and, when it
is too old, says which one it is and how to fix it, in one place — not as
launch errors in the game view and the chat.

### D41 — the conversation is readable in the headset

*2026-11-14. Extends D28.*

A spoken request is followed and answered without taking the headset off. The
panel shows the current task's conversation as a page of its own.

- **What it shows:** what the user said, with spoken messages marked as
  spoken; the router's decision line; the agent's replies and what it is
  doing now; the result; and any question the agent asked, shown first.
- **Text only, newest at the bottom, scrollable.** It follows new text unless
  the user scrolled up. Long agent messages are cut to their first lines with
  "more".
- **It carries no code:** no file contents, diffs or tool inputs, only the
  short step labels the desktop chat already shows.
- **The user answers by voice** (push-to-talk) from the same page; its stages
  show inline, above the conversation.
- **With the panel closed,** the status toast also shows the agent's first
  reply and the final result, so the loop is visible without opening anything.

### D42 — the agent can switch the project when asked

Extends D25. Asked in the headset "switch to another sample", the agent
could only say it could not: its tools drove one project's game. Switching
the project is taking the wheel, so it is allowed only when the user asks.

- **Tools.** `list_projects` names the open folder's projects (the open one
  marked) and recent ones in other folders. `open_project` takes a project in
  the open folder and no other path. Another folder opens only when no task
  runs, and the asking one does, so the agent tells the user to open it from
  the project switcher; nor can it open an arbitrary folder.
- **The picker's switch.** The app switches as the project picker does: the
  old game stops and the new one opens paused (D37). A game on a headset
  stays on it: the new project's game runs there too, for this switch and a
  manual one, while that project stays open; its saved layout keeps its own
  placement, whatever else of it is saved meanwhile.
- **The gate, without judging intent.** Applied only for a running task whose
  project is the one open, within 10 minutes of the user's last message to
  it (a request that waited its turn longer is not taken as asked), once per
  run, with "Let the agent arrange views" on (one setting for
  what the agent may change on screen), and never for a review.
- **The task stays.** A task belongs to its project (D12). The tool answers
  that the app now shows the other project and that the next request works
  there; the run ends with a sentence. Before asking, its tool server forgets
  the game it was attached to and refuses that address for the run, which may
  now serve the other project's game; only a refused switch lifts that.
- **The chat follows the app.** It shows a new task in the new project and a
  line: "Switched to *project* — asked by you in *task*", with Undo (back to
  that task) and a dismiss. Switching again, or the next request (a new
  task, a follow-up or a steer), clears it.
- **The headset hears it.** The spoken request ends with "Switched to
  *project*", shown and said aloud (D47), as an Answered status: the
  headset needs no new one.

### D43 — every request shows where it is

A principle, from using the headset for real: after the user says or types
something, there is never a silent wait. One small progress line shows which
step the request has reached, and it is the same line on the desktop and in
the headset.

The steps, in order:

1. **Received** — the words or the clip arrived. Shown at once.
2. **Transcribed** — for speech: the text, as heard. It stays long enough to
   read, also when the next step follows at once.
3. **Processing** — where it went (steers a task, a new task, queued), and
   the agent is starting.
4. **The agent's status** — thinking, or working with what it is doing now
   (D34), then its reply.
5. **The outcome** — answered, changed and reloaded, needs you, or failed
   with the reason in a few words.

Rules:

- **One element that moves through the steps,** not a stack of messages; each
  step shows how long it took once it is done.
- **A step that takes long says so** rather than disappearing: after a couple
  of seconds the current step shows it is still going.
- **A late step brings the line back** if it had faded, and a step is never
  skipped on screen because the next arrived quickly.
- **The same state feeds every surface:** the chat, the headset toast and the
  headset panel read one progress record held by the app, so they cannot
  disagree.
- **A failure names its step** ("Could not transcribe", "The headset is
  away"), so the user knows what to try again.

### D44 — the workshop: a zoomed-out place to work from

A direction, not yet a plan. Today the headset shows the game with our
interface laid over it. The workshop turns that round: the workspace is where
the user is, and the game is one object in it.

**Two scales.**

- **Workshop (zoomed out).** Passthrough, seated at the real desk. The game is
  a live miniature on the desk, which can be grabbed, moved and scaled. Around
  it: the tasks, the conversation, the agents and a map of the code. Status is
  in the room; nothing has to be opened to see it.
- **Inside (1:1).** The user is in the game, as today. Pointing at a spot in
  the miniature and pressing goes in at that spot; one gesture comes back out.
  The game's state is the same on both sides.

**The miniature.** Pause and scrub it with a dial; point at something in it
and say "this"; put the old and the new version side by side to decide
(D36); a game with no 1:1 inside (a 2D platformer) is simply viewed this way.

**The agents (D34, in the room).** A figure per agent, standing at what it
is working on: at a file when it reads or edits, at the miniature when it
drives the game, at the desk's edge with a hand up when it needs the user. A
tool call is a visible link from the figure to its target (a file, the
miniature, the network). A subagent splits off as a smaller figure. Context
is something the figure carries, and it fills.

**Several agents, in a hierarchy.** An orchestrator hands work to subagents:
it stands by the tasks, passes a piece of work along a visible link, and the
subagent goes to its target and comes back with the result. Who works for
whom is readable at a glance (tethers, or a faint tree on the floor), two
levels shown and deeper ones folded into a count. The tasks panel shows the
same tree.

**Forks that converge.** Parallel subagents each work in their own copy of
the code, so their changes stay apart. The room shows it: each fork is a layer
over the city in its agent's colour, showing only what that fork changed,
with its own miniature running that version. When a subagent finishes, its
layer settles into the base. Where two forks
changed the same block, a clean merge interleaves and a conflict holds the
two versions apart for the orchestrator, or the user, to settle. The user can
keep one fork, several, or send one back, and sees the combined result as one
more miniature before the final Keep — two forks that each work but not
together show up there.

**Drafts are drawn by the desktop.** A fork's miniature need not be a game
running in the headset. The desktop already runs each fork's game hidden; it
renders the miniature from where the user is looking and sends the picture,
which the headset shows as a draft — semi-transparent, at a low rate. Only
the one the user reaches for becomes a live game in the headset. So the
headset's cost stays that of one game however many forks there are.

**What watching them is for — all three.** Progress in real time (position,
link, bubble, context). Steering: an agent can be addressed — look or point
at a figure and talk to that one; talking to the orchestrator redirects the
whole effort, talking to a subagent only its piece; stop is on the figure,
and stopping an orchestrator stops what is under it. And delight: the
figures have some character, never at the cost of reading them.

**When an agent needs the user.** It comes to the user and brings the thing:
it walks to the desk's edge with a hand up, and a sound comes from where it
stands. A question is a card it holds out; a change to decide is the two
miniatures set down in front of the user; an approval shows what it would
run. The user answers in place, by voice to that figure or with a pinch.
Several that need the user wait in order at the desk's edge, and the tasks
panel shows the same order. Sent away, it waits visibly and does not nag: one chime, then quiet.
Everything waiting on the user is also one list, in order — who, what they
need, how long they have waited — at the top of the tasks panel and on the
palm-up glance; choosing a row brings that agent and its thing forward.

**The code map is a code city** (D33, D46): folders are
districts, files are blocks, and every definition is a building, on the desk
beside the miniature. It is the place the agents are seen working, several at
once in the same codebase:

- **Agents stand in the city.** A worker is at the building it is reading or
  editing, with its link going into it; where two agents touch the same file
  the block shows it.
- **What is changing shows as construction.** Uncommitted work paints the
  city: changed buildings by how much was
  added and removed, new ones rising, deleted ones as empty lots. Each fork
  is such a layer, so forks are seen diverging and converging on one city
  rather than as copies of it.
- **It is where a change is explained.** A tour of the city — a few stops
  with the reason for each — is the walkthrough of a finished change (D25,
  D31), and pointing at a building opens its change.
- **It scales both ways.** A small game is one district of a few blocks and
  is drawn simply; a large project is a real city, and zooming in is going
  down a level.
- **The same model draws flat** as a skyline in the desktop pane.

**Seated and standing, hands busy or free.** The workshop must work at a
desk and while walking about — the aim is to keep working on a game while
doing chores, headset on.

- **At the desk:** the full layout, anchored to the desk.
- **About the house:** a compact form that comes along — the miniature small
  and near a hand or parked on a surface, the request-progress line and one
  agent status in the edge of view, everything else folded away. Passthrough
  always; nothing may cover where the user is walking or what the hands are
  doing.
- **Hands, not controllers.** Hand tracking drives it: a pinch-and-hold to
  talk, a pinch to pick, a grab to move the miniature, a palm-up glance for
  status (the wrist light's successor). Going in is a pinch on a spot pulled
  back while a ring fills; coming out is both palms pushed away; "later" is a
  brush aside; reset is both fists together. Controllers stay supported for
  playing the game itself.
- **Or a wake phrase.** When hands are wet or full, a phrase of the user's
  choosing ("Computer, …") starts a request. It is off until turned on,
  because it means the microphone listens all the time: a mark in view shows
  when it does, one gesture or word mutes it, and what it hears goes nowhere
  but the paired desktop, which keeps nothing that did not begin with the
  phrase.
- **Voice first, and sound back.** With eyes and hands elsewhere, a request
  is spoken and its progress is heard as well as shown: a short sound per
  step, the outcome spoken briefly ("Changed. Ready to look at."), a question
  from the agent read out and answered by voice.

**The workshop is itself a Functor project — a scene about a scene.** It is
a `.fun` program like any game: its model is the workspace (where things
stand, what is open), and it draws the miniature by containing the running
game's frame, as the mirror does (4.5). So:

- **The user can change their workspace by asking.** "Put the code city on
  my left", "make the conversation bigger", "give the agents hats" is a task
  like any other: an agent edits the workshop's own `.fun`, it hot-reloads
  with its state kept, and the change is kept or reverted the usual way.
- **It ships as a default the user's copy is forked from,** so updates to the
  default converge with the user's changes rather than replacing them.
- **The data model is an external contract.** What a workspace may read
  (the state: tasks and their tree, agents with stance, activity, context and
  parent, request progress, what waits on the user, forks, the code map,
  devices, the running games) and what it may ask for (the actions: talk to
  this agent, keep, revert, stop, move a request, go in) is one versioned
  schema that belongs to the app, not to any one interface. The desktop page,
  the headset panel and the workshop scene are three clients of it; types for
  each are generated from it (TypeScript, and a typed Functor Lang module).
  The rules live behind the contract: confirmation for Keep, Revert and Stop,
  who may be steered, and what an agent may run are enforced where an action
  lands, so no workspace, however edited, can remove them. A workspace can be
  developed and tested against recorded state with no app running.
- **It cannot lock the user out.** A workshop that fails to load, or loads
  and misbehaves, falls back to the built-in default, with "reset my
  workspace" always reachable by voice and by a gesture the scene does not
  own.
- **The same scene draws on the desktop,** where it is the agents' view in a
  pane (D34).

**Themes.** Because the workshop is a scene over a contract, it can look like
something else entirely while meaning the same: a plain office (the manager
at the front desk, workers at theirs, the code city as an architect's model,
what waits for the user in an in-tray), a starship's bridge and engineering
(the game on the main screen, forks on side screens, workers at stations by
what they do, the code city as the ship's systems display, "needs you" as an
alert, going in through a door to the simulation). A theme is a scene and a
mapping of roles; the data, the actions and the rules do not change, and the
gestures, the progress line and where "waiting for you" appears stay the
same so habit carries over. A theme has a passthrough form (furniture and
figures in the real room) and may have an immersive one, for when the user is
still. The user gets one by asking for it.

What the engine must gain: a scene that holds another game (and answers for
things tied to the player's view — sky, fog, post-effects — and the world's
bounds), text and panels drawn in the world, hands as input, and the time to
run two programs in one headset frame.

**Order to try it in:** the miniature with teleport in and out; the tasks and
the conversation beside it; the agents; the code city; two miniatures for a
decision.

### D45 — a project manager is the point of contact

A direction, with D44. One agent, the project manager, is who the user talks
to by default — in the chat and in the headset. It does not do the work; it
takes requests, hands them out, and reports back.

- **It replaces the router's guess with a colleague.** What the router decides
  silently (D29: steer, new task, queue) the manager decides and says: "That
  is for the jump task — I have told it", "New piece of work; it starts after
  the current one." The decision is still shown and still movable with one
  tap.
- **It hands out the work** to worker agents (D44: the hierarchy, the forks),
  keeps the queue (D30), and knows who is doing what.
- **It gathers what needs the user.** Workers' questions and finished changes
  come through it: it brings one summary rather than five interruptions, and
  the pending list is its list.
- **It reports.** "Two of three are done; the third wants a decision." The
  voice the user hears back is the manager's.
- **Workers can still be addressed directly.** Looking at one and talking
  steers that worker; the manager is told.

What it must be to work: quick and cheap. It answers within a second or two
("Got it") before any work starts, so it runs lean — a small model, its own
few tools (start, steer, queue, ask, summarise, switch project), none of the
user's own setup, and a short memory of the effort rather than the code. An
always-on agent that costs what a worker costs per sentence would not be
worth having.

To try first on the desktop: the chat's default recipient becomes the
manager, over the existing router, queue and tasks (S8).

### D46 — the code city, for Functor Lang

The code map (D33, D44) is a city of **definitions**, built for `.fun` only.
The principles, the architecture and the plan are in
[`docs/code-city.md`](../../code-city.md).

**Geometry carries slow facts; light and motion carry live ones.** The city
does not reshuffle: a mode, a frame or an agent never moves a building.

- **Footprint** is the size of the definition's code.
- **Height** is history as strata: one slab per change that touched the
  definition, coloured by the task or agent that made it. A fork's
  uncommitted work floats above the stack until kept.
- **Inside a file:** functions are buildings; types are low, wide foundations
  with a segment per field or variant; constants are kiosks; inline modules
  are terraces.
- **The loop is the landmark.** The game loop's entry points are towers at
  the district's gate; the model is a central plaza.
- **Execution light** comes from the runtime's coverage: what ran on this
  frame is lit, what can run but did not stays dark, and scrubbing replays
  the city. Exact while paused today (per expression); full speed needs a
  cheaper signal (S10).
- **Traffic is minimal:** no drawn roads. A dot travels from caller to callee
  when the call happens — one per pair per frame, between files from afar and
  between definitions up close. Static references are a later, optional
  overlay.
- **Files may draw themselves.** A file or inline module may define an
  optional, pure `diagram = (model, tts) => Scene.t` (working name
  `ide_render`), drawn above its plate in the plate's own unit box: a jump
  arc, a state machine with the current state lit, a minimap. The game loop
  never calls it; it is bounded in time and size, and a fault shows as a
  mark on the plate. An agent may write one to explain a change.

**Built in layers:** an analyzer on the language crate emits a versioned,
source-neutral snapshot; history, execution, agents and forks are separate
layers keyed by definition id; one model lays it out and turns it into marks;
the workshop scene and a flat skyline in a desktop pane draw the same marks.
A tour is the existing walkthrough (`present`, D25/D31), not a second format.

**To reconsider:**

- *Slab colour by agent rather than by task.* Tasks outnumber the six or so
  hues that can be told apart; an agent's hue with the selected task outlined
  stays readable.
- *Types do not grow with history.* A much-edited type would become a tower
  and read as a function; show its changes as bands on the foundation's edge.
- *What one slab is.* A kept task (or an outside commit) that changed the
  definition's meaning, so formatting and moves add nothing.

**What the engine needs to draw it well** (the list and the reasons are in
the document): text as a scene primitive, per-instance attribute buffers, a
box ray query; a district of up to about 150 buildings draws with today's
prelude.

### D47 — the agent answers aloud

*2026-11-14. Extends D8, D29, D41.*

A request the user spoke is answered in speech, so the loop closes without
reading.

- **When:** as a spoken request reaches its outcome — answered, changed and
  reloaded, or failed. Not at progress points for now: the toast and the panel
  carry those.
- **On by default for a spoken request, with a setting to mute**
  (`FUNCTOR_APP_SPEAK=off` until the settings page has it). A typed
  request is silent unless speech is turned on for it. Nothing listens: this
  is a reply to something the user said, so it does not fall under "off until
  turned on".
- **What is said:** one or two sentences written for the ear — no paths, no
  code, no markdown. The working agent writes it: a spoken request's prompt
  asks it to end its last message with a `Spoken:` line. That costs about 30
  output tokens and no time. When the line is missing, the reply's first
  sentences are used, cleaned the same way. A separate summariser run was the
  alternative: another process start and model call, 3–6 s later, to restate
  what the working agent already knows. The presentation agent (D31) may
  write the line later for changes that get a presentation.
- **The app adds what it knows and the agent does not:** "It's ready for you
  to look at" when the change waits for review; that it failed, and why in a
  few words.
- **Synthesis is local, on the Mac:** no audio and no text leaves the
  machine. First the macOS system voice: nothing to download, 15 ms to the
  first audio once warm (330 ms cold) through the system's API; the first
  build runs the `say` tool instead, 0.6–1.6 s a reply. Later, as a choice, Kokoro for a
  better voice: Apache-2.0, an 88 MB model and 26 MB of voices, 260 ms to the
  first sentence. Piper is faster (55 ms) but its phonemiser is GPL and its
  voices' licences vary. The Quest has a system speech service, but it is not
  a documented engine for apps and its voice is not ours to choose.
- **Where it plays:** in the headset when one is linked — PCM on a link
  channel of its own, through the headset's audio output — otherwise on the
  desktop. The first build plays on the desktop only.
- **Barge-in:** holding push-to-talk stops the reply at once. A new reply
  replaces one still playing.

### D48 — a paused game says so, and who paused it

*2026-11-14. Extends D28, D34, D37.*

An agent's `step` pins the game clock, and the animation stopped with nothing
saying why. The cause is recorded where the pause is asked for, never guessed
by a view.

- **The request says who asks.** The agent's game tools, the transport bar,
  and a game that opens paused each name themselves; the runtime keeps the
  cause while the clock holds and forgets it when the clock runs. The user's
  pause takes an agent's over; an agent stepping a game someone else paused
  leaves it theirs. A pause nobody names (the game window's own timeline, a
  seek) shows no cause.
- **One run per project,** so the pause is put down to the project's run in
  flight; the runtime knows an agent asked, not which.
- **In the headset,** a small "PAUSED" marker stays for as long as the clock
  holds, with "Agent is checking its change" when the agent holds it. It gives
  its place to the pairing card, the open panel and the voice toast, and is
  left out of a captured frame.
- **The agent's status** says "checking the change, game paused" while its run
  is in flight and it holds the game: on its card, in the chat, and in the
  headset panel.
- **The transport bar** says "Paused by the agent", "Paused at start" or
  "Paused" beside its buttons.
- **The run's end resumes what the agent paused.** A pause the runtime puts
  down to the user or to the start stays; with no cause reported (an older
  runtime) it goes by what the app asked for, as before.

## Open

- In-headset agent presence (the original's open issues) — after M4.
- Whether commit / branch / PR appears in the developer menu — likely yes, as
  part of the build-info panel in M5.
