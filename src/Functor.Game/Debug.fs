namespace Functor
open Fable.Core

module Debug =

    // Log a line that is visible on every target. Prefer this over `printfn`
    // when you want output in the browser console: on wasm, `printfn` compiles
    // to Rust's `println!` -> stdout, which is not connected to anything in the
    // browser, so those lines are silently dropped. `Debug.log` routes to
    // `console.log` on wasm and stdout on native.
    [<Emit("functor_runtime_common::log($0)")>]
    let log (message: string) : unit = nativeOnly
