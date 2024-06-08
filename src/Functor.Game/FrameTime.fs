namespace Time

open Fable.Core
[<Erase; Emit("functor_runtime_common::FrameTime")>] 
type FrameTime =
    abstract dts: float
    abstract tts: float