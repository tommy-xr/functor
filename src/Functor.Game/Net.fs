namespace Functor

open Fable.Core

// The result of an HTTP request. The request is fired with `Effect.httpGet` /
// `httpPost`, which carry a `tagger : HttpResponse -> 'msg` (the Elm `expect`);
// the runtime applies the tagger when the response lands and delivers it as a
// message through `update`. See docs/multiplayer.md.
module Net =

    /// A completed HTTP request handed to your request's tagger. This is a thin
    /// handle over the runtime's `HttpResult`; read its fields through the
    /// accessors below.
    [<Erase; Emit("functor_runtime_common::net::HttpResult")>]
    type HttpResponse =
        /// The correlation token from the originating request effect.
        [<Emit("($0.token as i32)")>]
        abstract token: int
        /// HTTP status code (e.g. 200, 404); 0 when the request failed before a
        /// response (DNS/connection error — see `ok`).
        [<Emit("($0.status as i32)")>]
        abstract status: int
        /// True when a response arrived (any status); false on transport error.
        [<Emit("$0.is_ok()")>]
        abstract ok: bool
        /// The response body decoded as UTF-8 text.
        [<Emit("$0.body_text().into()")>]
        abstract body: string
        /// The transport error message, or "" on success.
        [<Emit("$0.error_text().into()")>]
        abstract error: string

    // Executor-only (not user space): drain the async inbox as the HTTP results
    // that arrived since the last frame.
    [<Emit("functor_runtime_common::net::drain_http_results()")>]
    let drainHttpResults () : HttpResponse array = nativeOnly

    // Executor-only: take the tagger registered for this result's request and
    // apply it, yielding the game's message. `None` if no tagger is registered
    // (unknown token, or one dropped by a hot reload while the request was in
    // flight). The Msg type is inferred from the call site.
    [<Emit("functor_runtime_common::net::take_pending($0)")>]
    let takePending (result: HttpResponse) : Option<'msg> = nativeOnly
