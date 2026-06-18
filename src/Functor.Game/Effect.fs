namespace Functor
open Fable.Core

[<Erase; Emit("functor_runtime_common::Effect<$0>")>] type effect<'msg> = | Noop


module Effect = 

    [<Emit("functor_runtime_common::Effect::none()")>]
    let none (): effect<_> = nativeOnly

    [<Emit("functor_runtime_common::Effect::wrapped($0)")>]
    let wrapped (a: 'a) :  effect<'a> = nativeOnly

    [<Emit("functor_runtime_common::Effect::map($0, $1)")>]
    let map (fn: 'a -> 'b)  (eff: effect<'a>) : effect<'b> = nativeOnly

    // Networking: fire an HTTP request, Elm-style. `tagger` (the Elm `expect`)
    // maps the eventual result to a message; the runtime applies it when the
    // response lands and delivers it through `update`. No in-frame message, and
    // no subscription needed. The host runtime performs the actual I/O.

    /// HTTP GET `url`, delivering the result to `tagger` as a message.
    [<Emit("functor_runtime_common::Effect::http(functor_runtime_common::net::next_token(), functor_runtime_common::net::HttpMethod::Get, $0.to_string(), vec![], vec![], $1)")>]
    let httpGet (url: string) (tagger: Net.HttpResponse -> 'msg) : effect<'msg> = nativeOnly

    /// HTTP POST `url` with a UTF-8 `body`, delivering the result to `tagger`.
    [<Emit("functor_runtime_common::Effect::http(functor_runtime_common::net::next_token(), functor_runtime_common::net::HttpMethod::Post, $0.to_string(), vec![], $1.to_string().into_bytes(), $2)")>]
    let httpPost (url: string) (body: string) (tagger: Net.HttpResponse -> 'msg) : effect<'msg> = nativeOnly

    
    // TODO: These should live elsewhere because they aren't user space

    [<Emit("functor_runtime_common::Effect::run($0)")>]
    let run (eff: effect<'a>) : 'a array = nativeOnly