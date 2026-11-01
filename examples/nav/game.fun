// examples/nav — agents route around walls with `Nav.path`. The graph is a
// top-level value: one node per square, links between open neighbours. The
// door is model state, so it goes in each query: D toggles it, and every
// agent asks again with a new `seq`; a route for an older `seq` is ignored
// when it lands. Searches run on a worker, and a scrub hands back the
// recorded routes.

type Msg = | Routed(i: float, r: Nav.result)

// Metres per second.
let speed = 3.0

// `#` wall, `D` the door, `.` floor. Square (x, z) is node z * width + x.
let rows = [
  "......#.....",
  "......#.....",
  "..##..#..#..",
  "......D..#..",
  "..#...#.....",
  "..#...#..##.",
  "......#.....",
  "............",
]
let width = 12.0
let height = 8.0
let tiles = rows |> List.map(Text.chars) |> List.flatten
let tile = (x, z) => List.nth(z * width + x, tiles) |> Option.defaultValue("#")
let isOpen = (x, z) => x >= 0.0 && z >= 0.0 && x < width && z < height && tile(x, z) != "#"
let square = (i) => Vec3.make(Math.mod(i, width), 0.0, Math.floor(i / width))

// Every link through the door square is gated by door 0.
let linksFrom = (i) =>
  let x = Math.mod(i, width) in
  let z = Math.floor(i / width) in
  [Vec3.make(1.0, 0.0, 0.0), Vec3.make(-1.0, 0.0, 0.0), Vec3.make(0.0, 0.0, 1.0), Vec3.make(0.0, 0.0, -1.0)]
  |> List.filter((d) => isOpen(x, z) && isOpen(x + Vec3.x(d), z + Vec3.z(d)))
  |> List.map((d) =>
       let (nx, nz) = (x + Vec3.x(d), z + Vec3.z(d)) in
       let gated = tile(x, z) == "D" || tile(nx, nz) == "D" in
       { from: i, to: nz * width + nx, cost: 1.0, bits: 1.0,
         door: if gated then Option.Some(0.0) else Option.None })

let nav = Nav.graph({
  nodes: List.range(width * height) |> List.map(square),
  links: List.range(width * height) |> List.map(linksFrom) |> List.flatten,
})

let goals = [Vec3.make(0.0, 0.0, 0.0), Vec3.make(11.0, 0.0, 7.0), Vec3.make(11.0, 0.0, 0.0),
             Vec3.make(0.0, 0.0, 7.0), Vec3.make(8.0, 0.0, 3.0), Vec3.make(3.0, 0.0, 4.0)]
let goal = (n) => List.nth(Math.mod(n, List.length(goals)), goals) |> Option.defaultValue(Vec3.make(0.0, 0.0, 0.0))

// `seq` is what the agent wants a route for; `routed` is the `seq` of the
// route it follows.
let agent = (n) => { pos: goal(n), goal: n + 1.0, route: [], seq: 1.0, routed: 0.0 }
let init = { doorOpen: true, agents: [agent(0.0), agent(2.0), agent(4.0)] }

let ask = (m, i, a) =>
  Nav.path(nav, $"agent{i}", {
    from: a.pos, to: goal(a.goal), bits: 1.0,
    closedDoors: if m.doorOpen then [] else [0.0],
    links: [], nodes: [], now: 0.0, seq: a.seq,
  }, (r) => Routed(i, r))

// Walk to the next waypoint; at the goal, head for the next one.
let step = (dt, a) =>
  match List.head(a.route) with
  | Option.Some(next) =>
    let d = Vec3.distance(next, a.pos) in
    if d < 0.05 then { a with route: List.drop(1.0, a.route) }
    else { a with pos: a.pos |> Vec3.lerp(next, Math.min(1.0, speed * dt / d)) }
  | Option.None =>
    if a.routed == a.seq && Vec3.distance(goal(a.goal), a.pos) < 0.05
    then { a with goal: a.goal + 1.0, seq: a.seq + 1.0 }
    else a

let tick = (m, dt, tts) =>
  let agents = m.agents |> List.map((a) => step(dt, a)) in
  let asks = agents |> List.indexedMap((i, a) => if a.routed < a.seq then [ask(m, i, a)] else []) in
  ({ m with agents: agents }, Effect.batch(List.flatten(asks)))

let input = (m, key, isDown) =>
  match key with
  | Key.D =>
    if isDown then { doorOpen: not m.doorOpen, agents: m.agents |> List.map((a) => { a with seq: a.seq + 1.0 }) }
    else m
  | _ => m

let update = (m, msg) =>
  match msg with
  | Routed(i, r) =>
    { m with agents: m.agents |> List.indexedMap((j, a) =>
        if j == i && r.query.seq == a.seq then { a with route: r.waypoints, routed: a.seq } else a) }

let colors = [Color.rgb(0.9, 0.3, 0.2), Color.rgb(0.2, 0.7, 0.9), Color.rgb(0.95, 0.8, 0.2)]
let color = (i) => List.nth(i, colors) |> Option.defaultValue(Color.rgb(1.0, 1.0, 1.0))

let walls =
  List.range(width * height)
  |> List.filter((i) => tile(Math.mod(i, width), Math.floor(i / width)) == "#")
  |> List.map((i) => Scene.cube() |> Scene.lit(Color.rgb(0.35, 0.37, 0.42)) |> Scene.translate(square(i) |> Vec3.add(Vec3.make(0.0, 0.5, 0.0))))

let door = (m) =>
  let at = List.range(width * height) |> List.filter((i) => tile(Math.mod(i, width), Math.floor(i / width)) == "D") in
  if m.doorOpen then []
  else at |> List.map((i) => Scene.cube() |> Scene.scaleXYZ(0.2, 1.0, 1.0) |> Scene.lit(Color.rgb(0.7, 0.45, 0.2))
                                |> Scene.translate(square(i) |> Vec3.add(Vec3.make(0.0, 0.5, 0.0))))

let agentScene = (i, a) =>
  Scene.group(
    [Scene.sphere() |> Scene.scale(0.6) |> Scene.lit(color(i)) |> Scene.translate(a.pos |> Vec3.add(Vec3.make(0.0, 0.3, 0.0)))]
    |> List.append(a.route |> List.map((p) =>
         Scene.cube() |> Scene.scale(0.12) |> Scene.emissive(color(i)) |> Scene.translate(p |> Vec3.add(Vec3.make(0.0, 0.06, 0.0))))))

let draw = (m, tts) =>
  Frame.createLit(
    Camera3D.lookAt(Vec3.make(5.5, 12.0, -4.0), Vec3.make(5.5, 0.0, 3.5)),
    Scene.group(
      [Scene.plane() |> Scene.scale(14.0) |> Scene.lit(Color.rgb(0.2, 0.22, 0.25)) |> Scene.translate(Vec3.make(5.5, 0.0, 3.5))]
      |> List.append(walls)
      |> List.append(door(m))
      |> List.append(m.agents |> List.indexedMap(agentScene))),
    [Light.ambient(Color.rgb(0.3, 0.3, 0.32)),
     Light.directional(Vec3.make(-0.4, -1.0, 0.3), Color.rgb(0.9, 0.85, 0.8), 1.0)])
