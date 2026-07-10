# Interactive UI (Elm-style)

Status: **design — nothing shipped.** Today `ui = (model) => View` renders a
text-only HUD (`Ui.text`/`column`/`panel`) that egui paints with every area
`.interactable(false)`; mouse buttons never reach game logic at all (the
desktop shell consumes them for cursor recapture and the scrubber). This doc
is the design for making UI *interactive* — buttons, sliders, text inputs that
produce messages folded through `update` — targeting **parity with React and
Elm**. It absorbs `docs/todo.md` time-travel items T2 (pointer/click plumbing)
and T4 (interactive `View`), but the UI system itself is independent of the
scrubber — the scrubber shows up only as U1's verification target, being the
one interactive egui widget that exists today. The culmination is
`examples/ui`, a widget showcase.

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

- [ ] **U1 — pointer plumbing (= todo T2).** Mouse buttons + free-cursor
      position into egui as `RawInput` on both shells; unify the scrubber's
      `PointerState` bridge. No game-facing change. *Verify:* the scrubber
      becomes a normally-interactive egui widget.
- [ ] **U2 — the seam.** `UiEvent`, `GameProducer::ui_event`, the slot table,
      `FrameCtx::deliver_ui_event`, the `RecordedInput` variant, debug-server
      injection. *Verify:* headless unit tests (inject event → `update` ran),
      before any widget renders.
- [ ] **U3 — `Ui.button` (= todo T4) + counter example.** Prelude ctor,
      interactable egui rendering, click → msg. *Verify:* debug-server e2e
      clicks the button and reads the count; functor-lang skill updated.
- [ ] **U4 — `Ui.slider` + `Ui.textInput`.** Tagger-carrying, with the
      reconciliation algorithm above and the keyboard-focus gate; plus the
      missing basics (`Ui.row`, remaining anchors). *Verify:* typing-fast and
      programmatic-reset cases as producer tests; validate the egui
      `TextEdit` buffer integration first — it's the riskiest piece.
- [ ] **U5 — `examples/ui`.** A widget showcase: one panel exercising every
      interactive widget (buttons, slider, text input) plus the display
      vocabulary (`row`/`column`/anchors), each wired to model state echoed
      back as text — the full UI → msg → model → view loop, no 3D scene or
      scrubber involvement required. GIF/PNG via pr-visuals.
