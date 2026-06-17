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

    // Networking: fire an HTTP request. Produces no message in-frame; the result
    // arrives later through the async inbox and is decoded by `Sub.httpResponses`,
    // matched on `token`. The command itself is plain data (it survives hot reload
    // like any queued effect); the host runtime performs the actual I/O.

    /// HTTP GET `url`. `token` correlates the eventual response.
    [<Emit("functor_runtime_common::Effect::http_request($0 as u64, functor_runtime_common::net::HttpMethod::Get, $1.to_string(), vec![], vec![])")>]
    let httpGet (token: int) (url: string) : effect<'msg> = nativeOnly

    /// HTTP POST `url` with a UTF-8 `body`. `token` correlates the response.
    [<Emit("functor_runtime_common::Effect::http_request($0 as u64, functor_runtime_common::net::HttpMethod::Post, $1.to_string(), vec![], $2.to_string().into_bytes())")>]
    let httpPost (token: int) (url: string) (body: string) : effect<'msg> = nativeOnly

    
    // TODO: These should live elsewhere because they aren't user space

    [<Emit("functor_runtime_common::Effect::run($0)")>]
    let run (eff: effect<'a>) : 'a array = nativeOnly