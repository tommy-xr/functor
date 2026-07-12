# Time travel & hot reload

Two things happen while a Functor game runs that you don't have to ask for: it
**hot-reloads** as you edit, and it **records itself** so you can pause and scrub
back through what already happened. Together they turn "change something, restart,
watch again" into "change something, watch it change *in place*, then rewind to
check." This page is how to use them.

## Hot reload

Edit a running game and the program hot-swaps under it — the running scene picks up
your new code without restarting. The state you care about survives the swap.

- **The model is preserved; `init` does not re-run.** A bouncing ball keeps
  bouncing, mid-arc, under your new gravity. Whatever your game's state is — score,
  timers, positions — carries straight over. Because `init` is skipped on a reload,
  an edit to `init` itself only takes effect on a fresh restart.
- **Closures kept inside the model rebind by name.** If your model holds a function
  value, it adopts the edited body of the definition it came from, carrying its
  captured values with it. A definition you *renamed or deleted* can't be matched,
  so its closure keeps running the old body and prints a loud `[functor-lang]`
  warning.
- **A broken edit never costs you the running scene.** An unbalanced paren or a
  parse error is printed once; the last good program keeps rendering. Fix the
  source and it recovers on the next save.
- **Pending effects reset on reload.** Anything in flight — say an HTTP request
  whose response was going to fold back into your model — is dropped rather than
  left dangling.

**Where it works.** On the desktop runtime, hot reload watches your project's
`.fun` files and reloads the moment you save. In the browser — the
[sandbox](/sandbox.html) and the live scene on the home page — there's no file to
watch, so the editor pushes each edit straight into the running scene as you type.
Same swap, same rules, either way.

## Time travel

Every session records itself as it runs. Each rendered frame, the runtime snapshots
the whole game — the model *and*, if you use physics, the physics world — into a
rolling history. That recording is what lets you go back.

**In the browser, use the scrubber.** Every player on the site — the sandbox and the
live scene on the home page — carries a scrubber strip beneath it. With it you can:

- **Pause** the running scene at the current frame;
- **Scrub** the slider back and forth across every frame recorded so far — you're
  moving through the game's own recorded state, not a video;
- **Single-step** one frame at a time while paused;
- **Resume**, and the scene plays on from wherever you left the playhead.

Scrubbing is non-destructive: dragging back and forth just re-shows recorded frames.
The timeline only branches — the recorded future is dropped — once you resume from a
point in the past and let the scene play forward again.

**On the desktop runtime**, the same recorder drives a built-in scrubber overlay.
It's hidden by default; press the **`~`** (tilde) console key to toggle it, then use
the same timeline, pause, and step controls. The desktop debug server also exposes a
`POST /rewind` endpoint that jumps the running game to a recorded frame by number —
handy for driving a session from a script or an agent rather than by hand.

## How the two fit together

The one rule worth internalizing: **a live edit is a reload boundary, and it resets
the recorded history.** Hot reload can carry your *model* across an edit, but the
frames recorded *before* the edit can hold code from the old version — so the
runtime clears the recording at each reload and starts a fresh one from the edit
forward.

In practice that means you **scrub forward from your last edit, never back through
it.** Edit, let it run and record for a bit, then rewind to inspect — all within
that one unbroken stretch. The moment you change the source again, the timeline you
were scrubbing is gone and a new one begins. This is deliberate, but it surprises
people, so: rewinding shows you the *current* code's recent history, not a replay of
older versions.

## Why this is possible

Both features fall out of how a Functor game is written. Your game is pure
Model–View–Update: the model is an ordinary value, and every frame is a function of
it — `tick` advances the model, `draw` turns it into a picture. So "go back a frame"
is just "restore an earlier value and draw it," and "hot reload" is just "keep the
value, swap the functions." Nothing is hidden in opaque engine state. The same
purity makes the recording deterministic — replayed under a controlled effect
runner, a program produces exactly the frames it produced live — which is the
foundation the deeper time-travel tools build on.

## Still evolving

The scrubber is an active area of the runtime — pause, slider, and single-step are
what ships today, and richer controls (like previewing where the scene is *headed*)
are landing over time. The core recording is already whole-game, so those additions
extend what you see on the timeline without changing how you drive it.

Next: the **[getting started](/docs/getting-started/)** guide sets up the local
hot-reload loop, and the **[language reference](/docs/language/)** covers the pure
MVU functions the recording is built on.
