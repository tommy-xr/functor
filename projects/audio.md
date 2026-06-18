# Audio

A project plan for audio in Functor. Goal: **game-ready sound** — fire-and-forget
effects (a gunshot, an explosion) *and* a continuous, spatialized ambient
**soundscape** (city hum nearby, a fountain you can walk around), on **native**
(rodio) and **wasm** (Web Audio), positioned relative to the camera.

## The core idea: audio is Effect + Sub, not a new concept

Functor already has the two shapes audio needs, and they map exactly:

| Game need | Existing dual | Audio API |
| --- | --- | --- |
| One-shot SFX (gunshot, explosion, UI click) | **`Effect`** — "do this once" | `Audio.play` / `Audio.playAt` (a command) |
| Continuous / ambient bed (wind, a fountain, engine loop) | **`Sub`** — "while the model looks like this, keep this alive" | `soundScape : model -> AudioScene` |

`Sub.fs` already spells out the pattern the soundscape needs:

> Resource-backed subscriptions … DO need identity — a live socket must be
> matched across recomputations so it isn't torn down and reopened every frame.

A looping spatial voice is exactly such a resource: each frame we recompute
`soundScape model`, **diff it against the live voices by key**, and spawn the
new, stop the gone, and update the position/gain of the ones that continue. So
the soundscape is the audio instance of the **keyed resource registry** the
backlog already anticipates (`docs/todo.md` → Effects & subscriptions), and
`Audio.play` is just another `Effect` draining through the existing
`EffectQueue`.

This keeps audio inside the MVU model rather than bolted on: a game is a pure
function of `model`, now for *what you hear* as well as *what you see*.

## Design principles (carried from the rest of Functor)

1. **Functional-core, imperative shell.** `AudioScene` is pure, serializable
   data. The *diff* (old scene vs. new scene → spawn/stop/update commands) is a
   pure function in `functor-runtime-common`, unit-testable with no device. Only
   the actual mixing/output (rodio, Web Audio) lives in the thin platform shell,
   behind an `AudioBackend` trait.
2. **LLM-native / headless.** A `NullAudioBackend` records *what would play*
   (voice spawns, positions, gains) without touching a sound device — the audio
   analogue of the text-only render path. Games run and audio is fully
   introspectable with no speakers, and the debug server exposes the live mix at
   `/audio`. This also makes audio **testable**: assert on the `AudioScene` a
   model produces, and on the command list the diff emits.
3. **Listener = the render camera.** You hear from where you see. The runtime
   derives the audio *listener* (position + orientation) from the same `Camera`
   the frame already carries, so games never specify it twice. Coordinates stay
   **Y-up, right-handed** (forward = look direction, up = `[0,1,0]`); the shell
   maps that to rodio's ear positions and Web Audio's listener vectors.
4. **Thin bindings.** The F# `Audio` types are `[<Erase; Emit>]` shims over Rust
   `functor_runtime_common::audio` types, like `Graphics`/`Light` today.
5. **Don't regress the dev loop.** Audio output runs on its own thread (rodio's
   output stream; the browser's audio thread). The per-frame reconciler only
   sends commands — never blocks the frame — and the live-voice registry lives in
   the **shell**, so it survives a hot reload for free (the dylib reloads; the
   audio keeps playing, and the next `soundScape` re-diffs against it).

## Architecture

**Types (pure data, in `functor-runtime-common::audio`, mirrored in F#).**

```
AudioScene { sources: Vec<AudioSource> }     // returned by soundScape

AudioSource {
    key:      String,            // identity for cross-frame reconciliation
    sound:    String,            // asset path (loaded via AssetCache)
    position: Option<Vec3>,      // world space; None = non-spatial (2D/music bed)
    gain:     f32,               // linear 0..1 (pre-attenuation)
    pitch:    f32,               // playback rate (1.0 = normal); later: doppler
    looping:  bool,              // ambient beds loop; most are true here
    rolloff:  Rolloff,           // {min_distance, max_distance, model} attenuation
}

Listener { position: Vec3, forward: Vec3, up: Vec3 }   // derived from the Camera
```

One-shots reuse a subset: `Audio.play(sound)` and `Audio.playAt(sound, pos)`
build an `Effect` carrying a transient `AudioSource` (no key — fire and forget).

**The per-frame loop (in the executor, after the model settles).**

1. Drain the effect queue as today; **audio one-shot effects** call
   `backend.play_oneshot(source, &listener)`.
2. Compute `listener` from the frame's camera; `backend.set_listener(listener)`.
3. Compute `desired = soundScape(model)`; `diff(live, desired)` → a list of
   `Spawn { key, source } | Update { key, source } | Stop { key }`; apply each
   via the backend. `diff` is pure and tested in `common`.

**`AudioBackend` trait (the only platform seam).**

```
trait AudioBackend {
    fn play_oneshot(&mut self, src: &AudioSource, listener: &Listener);
    fn spawn(&mut self, key: &str, src: &AudioSource, listener: &Listener);
    fn update(&mut self, key: &str, src: &AudioSource);   // position/gain/pitch
    fn stop(&mut self, key: &str);                        // (later: fade out)
    fn set_listener(&mut self, listener: &Listener);
}
```

- **Native (`functor-runtime-desktop`): rodio.** An `OutputStream` + a `Sink`
  per non-spatial voice and a `SpatialSink` per positioned voice (rodio takes
  emitter + left/right-ear positions, which we compute from the listener). rodio
  decodes wav/ogg via its `Source` decoders.
- **Web (`functor-runtime-web`): Web Audio.** One `AudioContext`; a `PannerNode`
  (+ `GainNode`) per positioned voice fed by an `AudioBufferSourceNode`, and the
  context's `AudioListener` for position/orientation. `decodeAudioData` decodes
  the fetched bytes.
- **Headless (`NullAudioBackend`, in common):** logs/records commands; used by
  the text runtime, tests, and the debug server.

**Assets.** Audio files load through the existing `AssetCache` with a new audio
pipeline (async, like textures/models): native decodes to PCM via rodio; wasm
decodes via `AudioContext.decodeAudioData`. Start with **wav** (trivial) and
**ogg** (small, royalty-free); both rodio and Web Audio handle them.

**Spatialization.** Distance attenuation (linear/inverse rolloff between
`min_distance` and `max_distance`) + stereo panning from the listener frame.
rodio's `SpatialSink` and Web Audio's `PannerNode` do the DSP; we only supply
world positions + the listener. Keep an equal-power/stereo model for parity;
Web Audio HRTF is an optional per-platform upgrade later.

**Bounded polyphony.** Like `MAX_LIGHTS`, cap simultaneous voices (e.g. 32) with
a simple **stealing** policy (drop the quietest/most-distant) so a runaway game
can't exhaust the mixer. The cap lives in the shared reconciler.

### Layout (new pieces)

| Path | What |
| --- | --- |
| `src/Functor.Game/Audio.fs` (+ `.fsi`) | F# API: `AudioScene`, `AudioSource`, `Audio.play/playAt`, smart constructors |
| `runtime/functor-runtime-common/src/audio/` | shared types, the `diff` reconciler, `AudioBackend` trait, `NullAudioBackend`, spatial math, voice cap |
| `runtime/functor-runtime-desktop/` | rodio backend |
| `runtime/functor-runtime-web/` | Web Audio backend |
| `src/Functor.Game/Game.fs` | `Game.soundScape` field + `GameBuilder.soundScape` |

## API sketch (F#)

```fsharp
// One-shots — commands, drain through the EffectQueue:
Audio.play "click.wav"                       // non-spatial
Audio.playAt "explosion.ogg" position        // positioned (uses the camera listener)

// Continuous soundscape — a pure function of the model, reconciled each frame:
let soundScape (model: Model) : AudioScene =
    AudioScene.create [|
        // A non-positioned ambient bed (2D), always on:
        AudioSource.ambient "wind-loop.ogg" |> AudioSource.gain 0.4f

        // A positioned, looping emitter the player can walk around:
        AudioSource.at "fountain" "water-loop.ogg" model.fountainPos
        |> AudioSource.gain 0.8f
        |> AudioSource.rolloff 1.0f 12.0f
    |]

game
|> GameBuilder.draw3d draw3d
|> GameBuilder.soundScape soundScape   // listener comes from draw3d's camera
```

`key` ("fountain") is the reconciliation identity; drop a source from the
returned scene and its voice stops, add one and it starts, move its `position`
and the live voice pans/attenuates — no per-frame restart.

## Roadmap (small, ordered PRs)

1. ✅ **One-shot effect, native** (#120). `Audio.play` effect + rodio backend +
   the audio asset pipeline. A spacebar press fires a gunshot.
2. ✅ **One-shot, wasm** (#123). Web Audio backend for the same path
   (`AudioContext`, `decodeAudioData`, `AudioBufferSourceNode`), at parity.
   - ✅ **Completion messages** (#122, not in the original plan). `Audio.playThen
     sound onFinished` delivers a message when a one-shot ends — the audio twin
     of `Effect.httpGet`'s tagger. Native-only for now (the host reports the
     finish back over a channel; wasm plays fire-and-forget).
3. 🔜 **Spatial one-shots** (#124, in review). `Audio.playAt`, the `Listener`
   derived from the camera, distance attenuation + panning (rodio `SpatialSink`
   / Web Audio `PannerNode`). An explosion to your left sounds to your left.
4. ⏭ **`soundScape` + the reconciler** (next). The pure
   `soundScape : model -> AudioScene`, `GameBuilder.soundScape`, the keyed
   `diff`, and per-frame spawn/stop/update of looping spatial voices, native +
   wasm. Verify: a positioned fountain loop that pans and fades as the WASD
   camera moves around it; survives a hot reload.
5. **Polish.** Per-voice fade-in/out on spawn/stop (no clicks), master
   volume/mute, a non-spatial **music** channel, the voice cap + stealing, and
   the debug-server `/audio` introspection endpoint.
6. *(Later / deferred)* pitch + doppler from voice velocity; directional cones;
   simple occlusion (raycast → low-pass); ducking (lower the bed under SFX);
   reverb/buses. Web Audio HRTF as an optional high-fidelity path.

### Progress / as-built notes (PRs 1–3)

The one-shot half (PRs 1–3) shipped with a **simpler seam than the `AudioBackend`
trait sketched above**: a one-shot is an `Effect` that pushes an `AudioCommand`
onto a process-global outbound queue in the game dylib, which the host drains
each frame through an `audio_drain_commands_json` runtime export and plays on its
device (rodio native / Web Audio wasm) — mirroring the `net` outbound-command
pattern, not a trait object. The queue lives on the dylib side and the device in
the shell, so **hot reload is free** (sounds in flight aren't disturbed). The
`Listener` type landed as planned (derived from the frame camera's
eye/target/up, Y-up RH).

Still **not built**, and expected to arrive with the soundscape work (PR 4) where
the keyed registry and pure `diff` make them natural: the `AudioBackend` trait +
`NullAudioBackend`, the headless/`/audio` introspection path, and the
`AudioScene`/command-list golden tests. The voice cap + stealing stays in PR 5.

## Open questions

- **Formats.** Ship **wav + ogg** first (both backends handle them; ogg keeps
  assets small). mp3 is patent-clear now but adds a rodio feature + size; defer.
- **Listener source.** Decided: the render camera. VR is still a single listener
  (the HMD), so this holds; split-screen would need multiple listeners — out of
  scope.
- **One-shot handles.** One-shots are fire-and-forget (no stop). A long
  one-shot you want to cancel (a charging hum) is better modeled as a keyed
  `soundScape` source. Revisit only if a concrete need appears.
- **Voice budget + stealing policy.** Start at 32 voices, steal quietest; tune
  later. Should the cap be per-channel (SFX vs. music vs. ambient)?
- **Determinism / "golden" for audio.** We can't byte-compare sound, but we
  *can* golden the **`AudioScene`** a model produces and the **command list** the
  diff emits (text, diffable) via the `NullAudioBackend` — the audio counterpart
  to the `/scene` dump and the render goldens. Worth doing in step 4/5.
- **Attenuation model + units.** Linear vs. inverse-square rolloff, and what
  "1.0 gain at `min_distance`" means in world units — pick defaults that feel
  right in the demo scenes (the lighting sample's scale is a good reference).
- **rodio on the dev loop.** Confirm rodio's `OutputStream` lifetime plays
  nicely with hot-reload (the stream must outlive reloads — it lives in the
  shell, not the dylib, so this should be free, but verify early).

## Demo target

Extend the existing lighting/primitives sample (or a small new `audio` sample):
a looping **fountain** emitter at a fixed position (walk around it with WASD and
hear it pan/attenuate), a non-spatial **wind** bed, and a **spacebar → explosion**
one-shot at the camera's focus point. This exercises all four core PRs and gives
the golden-style `AudioScene`/command tests something concrete to assert on.
