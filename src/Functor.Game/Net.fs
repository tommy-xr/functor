namespace Functor

open Fable.Core

// Inbound networking results, surfaced to the pure game through `Sub` decoders
// (the outbound side is `Effect.httpGet` / `httpPost`). See docs/multiplayer.md:
// effects carry plain data out; subscriptions decode results back in, correlated
// by the `token` the request chose.
module Net =

    /// A completed HTTP request delivered to the game by a `Sub.httpResponses`
    /// decoder. This is a thin handle over the runtime's `HttpResult`; read its
    /// fields through the accessors below.
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
    // that arrived since the last frame. The executor routes these to the game's
    // `Sub.httpResponses` decoders.
    [<Emit("functor_runtime_common::net::drain_http_results()")>]
    let drainHttpResults () : HttpResponse array = nativeOnly
