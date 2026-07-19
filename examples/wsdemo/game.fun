// wsdemo — the Functor Lang port of examples/wsdemo (docs/functor-lang.md Track E).
// A minimal WebSocket client: `Sub.connect` keeps a connection open; when it
// opens, the runtime hands us a connection id via `Net.Connected`, which we
// store and use with `Effect.send`. Inbound messages flow to the same tagger.
// Driven headlessly in tests (events injected without a real socket).
//
//   functor -d examples/wsdemo run native

type Conn =
  | NoConn
  | Conn(id: float)

type Model = { conn: Conn, status: string, lastMsg: string }

type Msg =
  | Ws(ev: Net.NetEvent)
  | Send(text: string)

let init = { conn: NoConn, status: "connecting", lastMsg: "" }

let update = (m: Model, msg: Msg) =>
  match msg with
  // Parenthesize the nested match so it doesn't consume the `Send` arm.
  | Ws(ev) =>
    (match ev with
     | Net.Connected(id) =>
         ({ m with conn: Conn(id), status: "connected" }, Effect.send(id, "hello"))
     | Net.Message(id, text) => { m with lastMsg: text, status: "got-message" }
     // This demo speaks plain text; a typed payload (Effect.sendMsg) is not
     // part of its protocol.
     | Net.Data(id, _) => { m with lastMsg: "<typed message>", status: "got-message" }
     | Net.Disconnected(id) => { m with conn: NoConn, status: "disconnected" }
     | Net.Error(id, e) => { m with status: Text.concat("error: ", e) })
  | Send(text) =>
    (match m.conn with
     | Conn(id) => (m, Effect.send(id, text))
     | NoConn => m)

// Declaring the connection in `subscriptions` is what keeps it open.
let subscriptions = (m: Model) => Sub.connect("ws://127.0.0.1:9001/echo", Ws)

let tick = (m: Model, dt: float, tts: float) => m

let draw = (m: Model, tts: float) =>
  Frame.create(
    Camera.firstPerson(
      Vec3.make(0.0, 0.0, -5.0),
      Angle.radians(0.0), Angle.radians(0.0), Angle.degrees(60.0)),
    Scene.cube() |> Scene.emissive(Color.rgb(0.6, 0.4, 0.9)))
