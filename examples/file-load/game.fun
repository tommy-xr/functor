// examples/file-load — rooms read on demand with `File.load`. `tick` asks
// for the current room's file until it lands: the read runs on a worker, so
// a big file never stalls a frame, and asking again while it is in flight is
// dropped (one read). The bytes arrive through `update`. Press N for the
// next room. Scrub back and the recorded bytes come back with the model;
// nothing is read again.

type Msg = | Loaded(room: float, r: Result.t<Bytes.t, File.error>)

let roomCount = 3.0
let roomPath = (n) => "rooms" |> Path.of |> Path.join($"{n}.txt")

// `rows` is the loaded room's lines; `room` is the one being asked for.
let init = { room: 1.0, shown: 0.0, rows: [], status: "loading" }

let tick = (m, dt, tts) =>
  if m.shown == m.room then m
  else (m, File.load(roomPath(m.room), (r) => Loaded(m.room, r)))

let input = (m, key, isDown) =>
  match key with
  | Key.N => if isDown then { m with room: Math.mod(m.room, roomCount) + 1.0 } else m
  | _ => m

let update = (m, msg) =>
  match msg with
  | Loaded(room, r) =>
    (match r with
     | Result.Ok(data) =>
       let text = Result.defaultValue("", Bytes.utf8(0.0, Bytes.length(data), data)) in
       { m with shown: room, rows: Text.split("\n", text), status: $"room {room}" }
     | Result.Error(e) =>
       (match e with
        | File.Missing(message) => { m with shown: room, rows: [], status: message }
        | File.Failed(message) => { m with shown: room, rows: [], status: message }
        | File.Unsupported(message) => { m with shown: room, rows: [], status: message }))

// One cube per `#`.
let walls = (rows) =>
  rows
  |> List.indexedMap((z, row) =>
       Text.chars(row)
       |> List.indexedMap((x, c) => if c == "#" then [Vec3.make(x, 0.5, z)] else [])
       |> List.flatten)
  |> List.flatten

let draw = (m, tts) =>
  Frame.createLit(
    Camera3D.lookAt(Vec3.make(5.0, 9.0, -6.0), Vec3.make(5.0, 0.0, 2.5)),
    Scene.group(walls(m.rows) |> List.map((at) => Scene.cube() |> Scene.translate(at))),
    [Light.ambient(Color.rgb(0.3, 0.3, 0.3)),
     Light.directional(Vec3.make(-0.4, -1.0, 0.3), Color.rgb(0.9, 0.85, 0.8), 1.0)])
