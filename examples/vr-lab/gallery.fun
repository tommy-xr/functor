// Gallery: the grip gallery, an entry of its own (functor.json's `gallery`
// role; `npm run grips:vr-lab` drives it). Each held item and site
// (`Grip.specs`) placed by its intent and fitted, the right hand's capsules
// on it: white free, green touching, red inside. The clock picks the item
// (`tts` 0, 1, …); the script moves the camera. Its model is every item's
// metrics, fitted on the first step (a hook: the game never pays for it),
// so a spec edit needs a relaunch, not a hot reload; and the tables the game
// looks fits up in (`Grip.tableOf`, `Grip.graspCells`), for `--write`. After
// the specs it shows a few of those fits (`views`): a support hand slid and
// rolled on its fore-end, a ball held two other ways.

/// One item measured at its placement: its preset and fit (`Grip.compare`),
/// the hand's root in its frame (its placement), the intent's own checks
/// (`Grip.checksOf`), the pose the game takes (`Grip.adopted`), and a gun
/// grip's placement in its layout frame (`Grip.placementOf`).
/// `extra`: one of the `views`, not a spec.
type Row = { name: string, intent: string, compared: Grip.Compared, hand: Input.pose, checks: Grip.Checks, adopted: Hand.FingerPose, placement: Option.t<Input.pose>, extra: bool }

/// A table's cells, by its site's name, and each cell's digits' gaps.
type Table = { name: string, cells: List<Grip.Cell>, gaps: List<List<float>> }

let tableRow = (name: string, cells: List<Grip.Cell>, hands: (List<Grip.Cell>) => List<Input.pose>): Table =>
  let gaps = Grip.itemNamed(name) |> Option.map((it) => List.zip(cells, hands(cells)) |> List.map((hc) => let (h, c) = hc in Grip.cellGaps(it, h, c))) |> Option.defaultValue([]) in
  { name: name, cells: cells, gaps: gaps }

/// `fingerprint`: the fit's inputs (`Grip.fingerprint`), for `--write`.
type Model = { rows: List<Row>, tables: List<Table>, grasps: List<Table>, fingerprint: float }

let init: Model = { rows: [], tables: [], grasps: [], fingerprint: 0.0 }

let rowOf = (it: Grip.Item): Row =>
  let c = Grip.compare(Hold.Right, it) in
  { name: it.name, intent: Grip.intentName(it.spec.intent), compared: c, hand: it.hand, checks: Grip.checksOf(Hold.Right, it, c.fitted.pose), adopted: Grip.adopted(it, c.fitted), placement: Grip.placementOf(it),
    extra: not (Grip.items |> List.any((i) => i.name == it.name)) }

/// A spec's item shown with the hand elsewhere on it: named, and where (a
/// fit away: placed in `tick`, so loading pays nothing).
type View = { item: Grip.Item, label: string, hand: (Grip.Item) => Input.pose }

let viewOf = (name: string, label: string, hand: (Grip.Item) => Input.pose): List<View> =>
  Grip.itemNamed(name) |> Option.map((it) => [{ item: it, label: $"{name} ({label})", hand: hand }]) |> Option.defaultValue([])
// A support cell's hand: slid `s`, rolled `r`, its palm pushed back on.
let slid = (s: float, r: float): ((Grip.Item) => Input.pose) => (it: Grip.Item) =>
  let h = Grip.slidOn(it, s, r, it.hand) in Grip.pushedBy((Grip.cellAt(it, true, h)).push, h)
// A free prop's placement `i`.
let grasped = (i: float): ((Grip.Item) => Input.pose) => (it: Grip.Item) =>
  Grip.graspSets |> List.find((g) => g.name == it.name) |> Option.bind((g) => List.nth(i, g.nudges) |> Option.map((n) =>
    let h = Grip.nudgedHand(it, n) in Grip.pushedBy((Grip.cellAt(it, g.push, h)).push, h))) |> Option.defaultValue(it.hand)

let views: List<View> = List.flatten([
  viewOf("rifle foregrip", "slid back 10 cm", slid(0.0 - 0.1, 0.0)),
  viewOf("rifle foregrip", "slid on 10 cm, rolled 23 deg", slid(0.1, 0.4)),
  viewOf("shotgun fore-end", "slid back 6 cm, rolled 34 deg", slid(0.0 - 0.06, 0.6)),
  viewOf("shotgun fore-end", "slid on 6 cm", slid(0.06, 0.0)),
  viewOf("ball", "2 cm toward the fingers", grasped(1.0)),
  viewOf("ball", "1.5 cm along the knuckles", grasped(3.0)),
])

// Every item shown, the views' hands where their specs' are (each row has
// its own).
let shown = Grip.items |> List.append(views |> List.map((v) => { v.item with name: v.label }))

let tick = (m: Model, dt, tts) =>
  if List.isEmpty(m.rows) then
    { rows: Grip.items |> List.append(views |> List.map((v) => { v.item with name: v.label, hand: v.hand(v.item) })) |> List.map(rowOf),
      tables: Grip.sliders |> List.map((sl) => tableRow(sl.name, Grip.tableOf(sl), (cs) => Grip.tableHands(sl, cs))),
      grasps: Grip.graspSets |> List.map((g) => tableRow(g.name, Grip.graspCells(g), (cs) => Grip.graspHands(g, cs))),
      fingerprint: Grip.fingerprint }
  else m

// The hand's root in the world: a metre up, facing -Z.
let root = Hold.p(0.0, 1.0, 0.0)

let mapped = (p: Input.pose): Camera3D.mappedPose =>
  let w = Hold.compose(root, p) in
  { position: w.position, forward: Quat.rotate(w.orientation, Quat.v3(0.0, 0.0, 1.0)), up: Quat.rotate(w.orientation, Quat.v3(0.0, 1.0, 0.0)) }

let shapeOf = (f: Grip.Form): Scene.t =>
  match f with
  | Grip.Box(h) => Scene.cube() |> Scene.scaleXYZ(2.0 * h.x, 2.0 * h.y, 2.0 * h.z)
  | Grip.Sphere(r) => Scene.sphere() |> Scene.scale(r)
  | Grip.Cylinder(r, h) => Scene.cylinder() |> Scene.scaleXYZ(2.0 * r, 2.0 * h, 2.0 * r)
  | Grip.Capsule(r, h) =>
    Scene.group([Scene.cylinder() |> Scene.scaleXYZ(2.0 * r, 2.0 * h, 2.0 * r),
      Scene.sphere() |> Scene.scale(r) |> Scene.translate(Vec3.make(0.0, h, 0.0)), Scene.sphere() |> Scene.scale(r) |> Scene.translate(Vec3.make(0.0, 0.0 - h, 0.0))])

let dot = (p: Input.point3, c: Color.t): Scene.t => Scene.sphere() |> Scene.scale(0.003) |> Scene.emissive(c) |> Scene.atPose(mapped(Hold.at(p, Quat.identity)))

/// An item's scene: its solids (see-through), the hand's capsules coloured
/// by their gaps, and the intent's target (yellow).
let itemScene = (r: Row, it: Grip.Item): Scene.t =>
  let solids = Grip.held(Hold.Right, it) in
  let posed = Hand.local(Hold.Right, r.adopted) in
  let paint = (c: Hand.Capsule) =>
    let (a, b) = Hand.ends(posed, c) in
    let g = Grip.gapOf(solids, a, b, c.radius) in
    if g < 0.0 then Color.rgb(0.9, 0.15, 0.15) else if g <= Grip.touchGap then Color.rgb(0.2, 0.85, 0.3) else Color.rgb(0.92, 0.92, 0.92) in
  Scene.group([
    Scene.group(solids |> List.map((s) => shapeOf(s.form) |> Scene.lit(Color.rgb(0.45, 0.5, 0.6)) |> Scene.atPose(mapped(s.at)))) |> Scene.opacity(0.4),
    Scene.group(Hand.capsules |> List.map((c) => Hand.partScene(paint(c), Hand.partOf(posed, [c])) |> Scene.rotateY(180deg) |> Scene.translate(Quat.toVec3(root.position)))),
    Scene.group(Grip.targetsOf(Hold.Right, it) |> List.map((p) => dot(p, Color.rgb(1.0, 0.85, 0.1)))),
  ])

let draw = (m: Model, tts) =>
  let i = Math.floor(tts) in
  let shown = List.zip(shown, m.rows) |> List.nth(i) |> Option.map((ri) => let (r, it) = ri in Scene.memo($"grip-{i}", i, (_) => itemScene(r, { it with hand: r.hand }))) |> Option.defaultValue(Scene.group([])) in
  Frame.createLit(Camera3D.lookAt(Vec3.make(0.0, 1.05, 0.0 - 0.35), Vec3.make(0.0, 1.0, 0.0)),
    shown,
    [Light.ambient(Color.rgb(0.35, 0.35, 0.38)), Light.directional(Vec3.make(0.3, -1.0, -0.4), Color.rgb(1.0, 1.0, 1.0), 0.8)])
