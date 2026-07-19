// server.fun — typed-message ping server: every `Protocol.Ping(n)` that
// arrives as a decoded `Net.Data` value is answered with `Protocol.Pong(n+1)`
// via `Effect.sendMsg`. No string wire anywhere — the shared Protocol ADT is
// the protocol.

type Model = { pings: float }

type Msg =
  | Incoming(cid: float, wire: Protocol.Wire)
  | Ignore

let toMsg = (ev: Net.NetEvent): Msg =>
  match ev with
  | Net.Data(cid, w) => Incoming(cid, w)
  | _ => Ignore

let init = { pings: 0.0 }

let update = (m: Model, msg: Msg) =>
  match msg with
  | Incoming(cid, w) =>
      (match w with
       | Protocol.Ping(n) =>
           ({ pings: m.pings + 1.0 }, Effect.sendMsg(cid, Protocol.Pong(n + 1.0)))
       | _ => m)
  | Ignore => m

let subscriptions = (m: Model) => Sub.listen("127.0.0.1:9101", toMsg)

let tick = (m: Model, dt: float, tts: float) => m

let draw = (m: Model, tts: float) =>
  Frame.create(
    Camera.lookAt(Vec3.make(0.0, 2.0, -4.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.cube())
