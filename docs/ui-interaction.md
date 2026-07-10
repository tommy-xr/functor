# Interactive UI (Elm-style)

Status: **SHIPPED** (U1–U5, 2026-07-10). `ui = (model) => View` is
interactive: `Ui.button` / `Ui.slider` / `Ui.textInput` produce messages
folded through `update` — Elm/React-parity controlled widgets — drivable
headlessly through the debug server (`POST /input {"type":"ui_event",…}`).
This doc is the design record and the map of the machinery; the reference
example is **`examples/ui`** (the widget showcase; `examples/counter` is the
button-only hello world). It absorbed `docs/todo.md` time-travel items T2
(pointer/click plumbing) and T4 (interactive `View`); the UI system is
independent of the scrubber.

## The shape

The Elm plumbing already exists and is reused unchanged: msgs are plain
`functor_lang::Value`s (usually ADT variants), everything funnels through
`update` via `absorb`/`drain_effects`, and there are two precedented ways a
declarative description carries a message — a **verbatim msg value**
(`Sub.every(dur, Pulse)`) and a **tagger** applied to a host-synthesized
payload (`Sub.connect`, `Effect.now`, physics events). UI widgets follow the
same split:

```
Ui.button("Reset", ResetClicked)                            // verbatim msg
Ui.slider(0.0, 10.0, model.speed, (v) => SpeedChanged(v))   // tagger
Ui.textInput(model.name, (s) => NameChanged(s))             // tagger
```

`ui(model)` stays pure and is re-evaluated every frame; widget values come
from the model; interactions become msgs; `update` stores them back. From the
game's perspective this is fully Elm-controlled.

## Design decisions

- **egui owns layout, hit-testing, and widget behavior.** An engine-side
  layout pass over `View` would duplicate egui's layout and mismatch it.
  Instead the shells feed egui real pointer input (`RawInput` — the T2 work,
  which also replaces the scrubber's hand-rolled `PointerState` bridge) and
  interactive nodes render as real egui widgets. Sliders/text fields get
  egui's dragging, focus, and IME for free.

- **Handlers live in a per-frame slot table, not in the tree.** `View` is
  serializable (it crosses the wasm boundary and stays LLM-inspectable);
  `Value` msgs/closures are not. During `ui(model)` evaluation each
  interactive constructor pushes its handler (`Msg(Value)` or `Tagger(Value)`)
  onto a per-frame `Vec` and stamps the node with its index — `slot: u32`,
  construction order. The shell's render recursion does nothing with slots
  except echo one back when egui reports an interaction. The table is rebuilt
  every frame from `ui(model)`, so hot-reload is safe by construction (same
  reason `subscriptions` taggers survive reloads) — no cross-frame pending
  map, no rebinding.

- **Events are delivered like input events.** The shell calls a new
  `GameProducer::ui_event(UiEvent)` where
  `UiEvent { slot, kind: Clicked | SliderChanged(f64) | TextChanged(String) }`
  (serializable). The producer looks up the slot, applies the tagger if any,
  and folds through `update` at the next frame's pre-tick step via a
  `FrameCtx::deliver_ui_event` modeled on `deliver_net_event` — one frame of
  latency, same as a buffered key event and same as Elm's event→msg→view
  loop. `UiEvent` also becomes a `RecordedInput` variant so time-travel
  replay includes UI interactions.

- **Widget identity is positional (the slot); explicit keys are a fallback,
  not the default.** Buttons are stateless. Stateful widgets (text focus,
  drag) need "same widget as last frame", and construction-order slots answer
  that positionally — correct whenever the tree shape is stable, which is
  every HUD/panel case, and it's also exactly how egui derives its own ids.
  The one failure mode is a *reordering dynamic list* (insert a field above
  the one being edited): state attaches to the wrong field for a frame, the
  reconciliation below self-heals with a cursor reset. If a real example hits
  that, add an optional explicit key (the React `key` / Elm `Html.Keyed`
  analog) for those widgets only.

- **Text inputs are uncontrolled-with-push-on-change, not naively
  controlled.** A fully controlled field drops keystrokes in an
  immediate-mode loop: the model echo is one frame behind the buffer. The
  shell keeps, per text widget: `live_buffer` (the string egui edits) and
  `last_emitted` (the last value sent up as a msg). Each frame, with the
  incoming model `value`:
  1. first sighting → `live_buffer = value`;
  2. `value == last_emitted` → our own edit echoing back → leave the buffer
     alone (comparing against the *buffer* instead would clobber every
     keystroke — the comparison target is load-bearing);
  3. otherwise → a programmatic change (Reset button, game logic) → overwrite
     the buffer, cursor to end.
  When egui reports an edit: `last_emitted = live_buffer`, emit
  `TextChanged`. Known accepted wart (React has the same one): a transform in
  `update` (uppercase, clamp) makes the echo differ → case 3 → cursor jumps
  to end — the transform wins, which is the right behavior. Buffer entries
  are dropped when their widget doesn't appear in the frame's view. Sliders
  get the cheap version: the live drag value wins for display while egui
  reports dragging; the model wins otherwise.

- **Routing policy.** Pointer: when the cursor is free, events feed egui;
  when egui doesn't want the pointer, a click recaptures for free-look (as
  today). Keyboard: when egui wants keyboard (a text field is focused), key
  events are suppressed from the game's `input` hook. A raw `mouseButton`
  game hook (click-to-shoot into the 3D world) is out of scope — a natural
  follow-up on the same plumbing.

- **Headless testability (LLM-native).** Because `UiEvent` is serializable
  and routed through one producer method, the debug server gains
  `POST /input {"type":"uiEvent", ...}` and the UI tree (slots + labels
  included) is dumpable — an agent can read the tree, click "Reset" by slot,
  and assert on `/state` without pixels. Unit tests drive the same seam with
  `FakeEffects`: evaluate `ui(model)`, inject an event, assert `update` ran.

- **Typechecking caveat, accepted.** Like `Sub.t`/`Effect.t`, the `Ui.t`
  interface erases `'msg`, so widget-msg ↔ `update` agreement is a runtime
  check. Consistent with every existing seam; typed msg contracts are a
  language-level project and don't gate this.

## Roadmap

- [x] **U1 — pointer plumbing (= todo T2)** (#288). The scrubber's
      hand-rolled `PointerState` → egui-event synthesis extracted into the
      shared, unit-tested `PointerBridge` the game-UI pass reuses.
- [x] **U2 — the seam** (#291). `UiEvent`, `GameProducer::ui_event`, the slot
      table, `FrameCtx::deliver_ui_event`, the `RecordedInput` variant,
      debug-server injection — all headlessly tested before any widget
      rendered. Replay rebuilds the handler table once per fine step, before
      the step's inputs (the live last-render contract).
- [x] **U3 — `Ui.button` (= todo T4) + `examples/counter`** (#293). The
      interactive `TextOverlay` pass (pointer via the U1 bridge, both
      shells), click → msg. Review catches now baked in: a press LATCH so
      sub-frame clicks aren't level-sampled away; scrubber pointer priority
      over an overlapping game widget; Empty-view frames still flush bridge
      events so egui can't hold a stuck press.
- [x] **U4 — `Ui.slider` (#294) + `Ui.textInput` (#297)** plus `Ui.row` and
      the remaining anchors. Both reconciliation algorithms above shipped
      as specified (per-slot buffers keyed by slot, dropped when the slot
      leaves the view); the keyboard focus gate suppresses the game's
      `input` hook — including releases whose press was swallowed — and the
      pinned clock hides the pointer from the pass entirely. The egui
      `TextEdit` integration is unit-tested headlessly (egui runs without
      GL; input hit-tests against the previous frame's rects, so click
      tests need a warmup frame).
- [x] **U5 — `examples/ui`.** The widget showcase: one panel exercising
      every interactive widget plus the display vocabulary, each wired to
      model state echoed back as text — the full UI → msg → model → view
      loop, no 3D scene or scrubber involvement.
