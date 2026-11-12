## Desktop app

### User Experience

#### User journey

1. User starts the functor app (via finder, or `functor` in the CLI)
2. The functor desktop app opens in the folder (or, last open project folder)
3. The primary interface is a chat-style interface (like ChatGPT, Claude, or VSCode's new agent mode)
  - A primary 'canvas' working area for each view
  - A left pane (solution explorer, tasks, devices, play)
  - A secondary left pane (the actual explorer window, description of active tasks, etc)

#### Theming / styling

Theming and styling should be similar to the existing MIS interface.

#### Working area / Chat Interface

The working area will show some visual representation, ie:
1. A rendered view of the game world
2. Code (or a diff)
3. A 'code city' style 3d visualizer
4. Tasks - the context we gave to the agent (ie, a reply and tasks) and 

In all cases - there will be a chat UI available. You can always engage an agent from any view.

#### Developer Menu
 
- When in vr, connected - we'll have a 'developer menu' invoked by the menu key. 
- There will be an option to open the 'real' in-game menu, as if the user pressed menu in game
  - This will have tuning parameters
  - Details about the hash, commit, and build that is being run

#### Use cases

1. Dynamic editing while in the world

While in VR, user should be able to press a button (thumbstick) to initiate voice chat - they can give a command. This audio stream gets sent from the Quest -> Desktop app,

They should be able to give feedback on objects or particular aspects in the game world. The agent should be able to introspect the full  state (we should keep a sliding window of before/after the recording so that the agent can analyze the full report).

The agent, if unsure about a tuning parameter, can provide a custom functor UI (either diedetic or invoked by the 'menu') that allows for tuning parameters - like how we had for shock2quest.

2. Manage device connectivity

Show connection state for quest devcie, and allow to switch to wifi mode

3. For the connected repo

When functor is run in a repo, and it submits pull request artifacts, we should be able to list and pull those.
  - When pulling them, we can show feedback that we are downloading and then hot reloading
  - We can create a helper github action that adds PR artifacts so that other repos (ie, all-must-fall or 3d-scanner-quest) could easily test prs w/o changing

#### Technology Stack

- Rust + gpui
- Whisper local model
- TTS model - we did an experiment with MakeItSo and found a good model
- WebRTC for communication with VR device

#### Agentic integration

Similar to MakeItSo, we will delegate agentic operations to a CLI coding agent (claude, codex). We can hopefully reuse some of the modules for that.

We may want to inject certain skills or hooks (ie, context about where  the user is in the functor interface)

Audio should be not be processed (converted to text or back) on the quest -> that should be sent to the agent

## Open Issues

- It'd be very useful to show in-quest feedback of what's happening - could we render an agent inline?
  - ie, the agent could have a positional representation in the world, and also show code diffs live on top
  - potentially visualize mutliple agents working on a code base
  - feedback on when a task is 'started' vs 'done'
- Could we have fable/opus generate a 3d model to represent itself?

- Is there feedback in the app for what commit / branch / pr we're working on? That might be useful info for a 'menu' item


## Out of scope

Future work:
- Have an in-vr visualization of the codebase (ie, 'code city' in vr)
