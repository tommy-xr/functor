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

    /// Play a sound once and deliver `onFinished` as a message when it ends — the
    /// audio twin of `Effect.httpGet`'s tagger. The host reports completion back
    /// after the sound finishes.
    [<Emit("functor_runtime_common::Effect::play_audio_then(functor_runtime_common::audio::next_token(), $0.to_string(), $1)")>]
    let playThen (sound: string) (onFinished: 'msg) : effect<'msg> = nativeOnly

    // Executor-only (not user space): drain the tokens of sounds that finished
    // since last frame, and take the completion message a token registered.
    [<Emit("functor_runtime_common::audio::drain_finished_array()")>]
    let drainFinished () : uint64 array = nativeOnly

    [<Emit("functor_runtime_common::audio::take_completion($0)")>]
    let takeCompletion (token: uint64) : Option<'msg> = nativeOnly
