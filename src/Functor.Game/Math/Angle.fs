namespace Math

open Fable.Core
[<Erase; Emit("functor_runtime_common::math::Angle")>] type Angle = | Noop

module Angle =

    [<Emit("functor_runtime_common::math::Angle::from_degrees($0)")>]
    let degrees (angle: float32): Angle = nativeOnly

    [<Emit("functor_runtime_common::math::Angle::from_radians($0)")>]
    let radians (angle: float32): Angle = nativeOnly