namespace Functor

open Fable.Core

/// Audio commands. One-shots (a gunshot, an explosion) are fire-and-forget
/// `effect`s, the dual of `Sub` — like `Effect.httpGet`, they ask the host
/// runtime to do something with no in-frame message. A continuous, spatial
/// `soundScape : model -> AudioScene` (the `Sub`-shaped half) is a later step.
module Audio =

    /// Play a sound once and let it finish (fire-and-forget). `sound` is an asset
    /// path the host loads and plays on its audio device (rodio on native, Web
    /// Audio on wasm). Returns an `effect`, so it composes like any command
    /// returned from `update` / `input` / `tick`.
    [<Emit("functor_runtime_common::Effect::play_audio(functor_runtime_common::audio::AudioCommand::play_one_shot($0.to_string()))")>]
    let play (sound: string) : effect<'msg> = nativeOnly
