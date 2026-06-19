namespace Functor

open Fable.Core

/// A continuous, looping voice in a soundscape (a wind bed, a fountain you can
/// walk around). The `Sub`-shaped half of audio: `soundScape : model ->
/// AudioScene` is reconciled by `key` each frame, so a live voice keeps playing
/// (panning/attenuating as the camera moves) instead of restarting.
[<Erase; Emit("functor_runtime_common::audio::AudioSource")>]
type AudioSource = | Noop

/// The full set of voices the game wants playing this frame.
[<Erase; Emit("functor_runtime_common::audio::AudioScene")>]
type AudioScene = | Noop

/// Audio commands. One-shots (a gunshot, an explosion) are fire-and-forget
/// `effect`s, the dual of `Sub` — like `Effect.httpGet`, they ask the host
/// runtime to do something with no in-frame message. The continuous, spatial
/// `soundScape : model -> AudioScene` (the `Sub`-shaped half) is built from
/// `AudioScene` / `AudioSource` below and reconciled by the runtime each frame.
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

    [<Emit("functor_runtime_common::Effect::play_audio(functor_runtime_common::audio::AudioCommand::play_one_shot_at($0.to_string(), $1, $2, $3))")>]
    let private playAtRaw (sound: string, x: float32, y: float32, z: float32) : effect<'msg> = nativeOnly

    /// Play a sound once at a world-space `position`, panned and attenuated
    /// relative to the camera (the listener). Fire-and-forget, like `play`.
    let playAt (sound: string) (position: Functor.Math.Vector3) : effect<'msg> =
        playAtRaw (sound, position.x, position.y, position.z)

    // Executor-only (not user space): drain the tokens of sounds that finished
    // since last frame, and take the completion message a token registered.
    [<Emit("functor_runtime_common::audio::drain_finished_array()")>]
    let drainFinished () : uint64 array = nativeOnly

    [<Emit("functor_runtime_common::audio::take_completion($0)")>]
    let takeCompletion (token: uint64) : Option<'msg> = nativeOnly

    // Executor-only: serialize the scene `soundScape` returned, for the host to
    // reconcile against its live voices (the `audio_scene_json` runtime export).
    [<Emit("functor_runtime_common::audio::scene_to_json(&$0).into()")>]
    let sceneToJson (scene: AudioScene) : string = nativeOnly

module AudioSource =

    /// A non-spatial bed (wind, music), keyed for cross-frame identity.
    [<Emit("functor_runtime_common::audio::AudioSource::ambient($0.to_string(), $1.to_string())")>]
    let ambient (key: string) (sound: string) : AudioSource = nativeOnly

    [<Emit("functor_runtime_common::audio::AudioSource::at($0.to_string(), $1.to_string(), $2, $3, $4)")>]
    let private atRaw (key: string, sound: string, x: float32, y: float32, z: float32) : AudioSource = nativeOnly

    /// A positioned emitter at a world-space point (panned/attenuated relative to
    /// the camera listener).
    let at (key: string) (sound: string) (position: Functor.Math.Vector3) : AudioSource =
        atRaw (key, sound, position.x, position.y, position.z)

    /// Set the linear gain (1.0 = full).
    [<Emit("$1.with_gain($0)")>]
    let gain (g: float32) (source: AudioSource) : AudioSource = nativeOnly

module AudioScene =

    /// Build the scene from the voices that should be playing this frame.
    [<Emit("functor_runtime_common::audio::AudioScene::from_sources($0)")>]
    let create (sources: AudioSource[]) : AudioScene = nativeOnly

    /// The empty scene (silence) — the default `soundScape`.
    [<Emit("functor_runtime_common::audio::AudioScene::default()")>]
    let empty () : AudioScene = nativeOnly
