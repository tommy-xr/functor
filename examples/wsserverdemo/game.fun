// wsserverdemo — the Functor Lang port of examples/wsserverdemo (docs/functor-lang.md E).
// A minimal WebSocket SERVER (native-only): `Sub.listen` binds an address;
// each accepted client surfaces through the tagger carrying its own
// connection id, which we use to reply with `Effect.send`. A closure tagger
// (`toMsg`) decodes Net.NetEvent into game-specific messages.
//
//   functor -d examples/wsserverdemo run native

type Model = { clients: float, status: string, lastMsg: string }

type Msg =
  | ClientConnected(id: float)
  | Received(id: float, text: string)
  | ClientLeft(id: float)
  | ConnError(text: string)

// The tagger: NetEvent -> a game-specific Msg (a closure, not a ctor).
let toMsg = (ev: Net.NetEvent): Msg =>
  match ev with
  | Net.Connected(id) => ClientConnected(id)
  | Net.Message(id, text) => Received(id, text)
  | Net.Disconnected(id) => ClientLeft(id)
  | Net.Error(id, message) => ConnError(message)

let init = { clients: 0.0, status: "listening", lastMsg: "" }

let update = (m: Model, msg: Msg) =>
  match msg with
  | ClientConnected(id) =>
      ({ m with clients: m.clients + 1.0, status: "client-connected" },
       Effect.send(id, "welcome"))
  | Received(id, text) =>
      ({ m with status: "echoed", lastMsg: text }, Effect.send(id, text))
  | ClientLeft(id) => { m with clients: m.clients - 1.0, status: "client-left" }
  | ConnError(e) => { m with status: Text.concat("error: ", e) }

// Declaring the listener in `subscriptions` keeps the server bound.
let subscriptions = (m: Model) => Sub.listen("127.0.0.1:9001", toMsg)

let tick = (m: Model, dt: float, tts: float) => m

let draw = (m: Model, tts: float) =>
  Frame.create(
    Camera.firstPerson(
      Vec3.make(0.0, 0.0, -5.0),
      Angle.radians(0.0), Angle.radians(0.0), Angle.degrees(60.0)),
    Scene.cube() |> Scene.emissive(Color.rgb(0.3, 0.6, 0.4)))
