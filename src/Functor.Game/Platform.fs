namespace Platform
open Fable.Core

[<Erase; Emit("functor_runtime_common::OpaqueState")>] type OpaqueState = | Noop

module OpaqueState =

    [<Emit("functor_runtime_common::OpaqueState::new($0)")>]
    let to_opaque_type<'T> (obj: 'T): OpaqueState = nativeOnly

    [<Emit("functor_runtime_common::OpaqueState::coerce($0)")>]
    let unsafe_coerce<'T>(state: OpaqueState): 'T = nativeOnly
