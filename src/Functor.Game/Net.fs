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

    // ----- Persistent connections (WebSocket/TCP/UDP) -----

    /// An opaque handle to a live connection, minted by the runtime and handed to
    /// you via `NetEvent.Connected` (or a server's per-client events). Hold it,
    /// compare it, key a `Map` by it, and pass it to `Effect.send` / `Effect.close`
    /// -- but you can't fabricate one, so a send always names a real connection.
    type ConnectionId = internal ConnectionId of int64

    /// Extract the raw id. `internal`, so only the framework (same assembly) can —
    /// games never see the underlying value.
    let internal rawId (ConnectionId id) = id

    /// Events delivered to a connection's `Sub.connect` decoder.
    type NetEvent =
        | Connected of ConnectionId
        // Single tuple fields: Fable's Rust backend miscompiles a match on a union
        // case with multiple separate fields (see Input.fs).
        | Message of (ConnectionId * string)
        | Disconnected of ConnectionId
        | Error of (ConnectionId * string)

    /// A thin handle over the runtime's `KeyedEvent`; the executor reads these to
    /// rebuild a typed `NetEvent` and route it by `key`.
    [<Erase; Emit("functor_runtime_common::net::KeyedEvent")>]
    type private ConnInbound =
        [<Emit("$0.key_str().into()")>]
        abstract key: string
        /// 0=Connected, 1=Message, 2=Disconnected, 3=Error.
        [<Emit("$0.kind()")>]
        abstract kind: int
        [<Emit("$0.conn()")>]
        abstract conn: int64
        /// Message payload (UTF-8) / error text / "".
        [<Emit("$0.text().into()")>]
        abstract text: string

    [<Emit("functor_runtime_common::net::take_conn_events()")>]
    let private takeConnInbound () : ConnInbound array = nativeOnly

    /// Executor-only: drain inbound connection events as `(key, NetEvent)` pairs.
    let internal drainConnEvents () : (string * NetEvent) array =
        takeConnInbound ()
        |> Array.map (fun e ->
            let id = ConnectionId e.conn
            let event =
                match e.kind with
                | 0 -> Connected id
                | 1 -> Message(id, e.text)
                | 2 -> Disconnected id
                | _ -> Error(id, e.text)
            (e.key, event))

    // Executor-only: reconciliation commands (open a declared connection / tear
    // down one no longer declared). Key == the endpoint url.
    [<Emit("functor_runtime_common::net::push_conn_command(functor_runtime_common::net::ConnCommand::Connect { key: $0.to_string(), url: $0.to_string() })")>]
    let internal pushConnect (key: string) : unit = nativeOnly

    [<Emit("functor_runtime_common::net::push_conn_command(functor_runtime_common::net::ConnCommand::Listen { key: $0.to_string(), addr: $0.to_string() })")>]
    let internal pushListen (key: string) : unit = nativeOnly

    [<Emit("functor_runtime_common::net::push_conn_command(functor_runtime_common::net::ConnCommand::CloseKey { key: $0.to_string() })")>]
    let internal pushCloseKey (key: string) : unit = nativeOnly
