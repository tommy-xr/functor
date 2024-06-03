namespace Functor.Math

open Fable.Core
[<Erase; Emit("functor_runtime_common::Angle")>] type Angle = | Noop

module Angle = 
    [<Emit("functor_runtime_common::Angle::degrees($0)")>]
    let pi: Angle = nativeOnly;

    [<Emit("functor_runtime_common::Angle::degrees($0)")>]
    let degrees (deg: float): Angle = nativeOnly

    [<Emit("functor_runtime_common::Angle::radians($0)")>]
    let radians (deg: float): Angle = nativeOnly