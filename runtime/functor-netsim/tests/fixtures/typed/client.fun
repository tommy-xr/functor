// client.fun — typed-message ping client: on connect it opens with
// `Protocol.Ping(1)`; every `Protocol.Pong(n)` that arrives as a decoded
// `Net.Data` value is recorded and answered with `Protocol.Ping(n+1)`, so the
// exchange escalates — `lastPong`/`rounds` growing proves repeated typed
// round-trips in BOTH directions.

type Model = { lastPong: float, rounds: float }

type Msg =
  | Joined(id: float)
  | Incoming(id: float, wire: Protocol.Wire)
  | Ignore

let toMsg = (ev: Net.NetEvent): Msg =>
  match ev with
  | Net.Connected(id) => Joined(id)
  | Net.Data(id, w) => Incoming(id, w)
  | _ => Ignore

let init = { lastPong: 0.0, rounds: 0.0 }

let update = (m: Model, msg: Msg) =>
  match msg with
  | Joined(id) => (m, Effect.sendMsg(id, Protocol.Ping(1.0)))
  | Incoming(id, w) =>
      (match w with
       | Protocol.Pong(n) =>
           ({ lastPong: n, rounds: m.rounds + 1.0 },
            Effect.sendMsg(id, Protocol.Ping(n + 1.0)))
       | _ => m)
  | Ignore => m

let subscriptions = (m: Model) => Sub.connect("ws://127.0.0.1:9101/typed", toMsg)

let tick = (m: Model, dt: float, tts: float) => m

let draw = (m: Model, tts: float) =>
  Frame.create(
    Camera.lookAt(Vec3.make(0.0, 2.0, -4.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.cube())
