// Connector — join the halves of a `Solid.split` part: dowel pins, a key,
// and laying halves flat to print. The `Fit` gap all goes into the hole or
// slot; pins and keys stay nominal.
//
//   let plane = { normal: Vec3.make(0.0, 0.0, 1.0), offset: 100.0 }
//   let pin = { diameter: 5.0, length: 16.0 }
//   let halves = Connector.dowelled(plane, Fit.slip, pin, [{ x: -15.0, y: 0.0 }, { x: 15.0, y: 0.0 }], tower)
//   let top = match halves with | (upper, _) => Connector.layFlat(Vec3.make(0.0, 0.0, -1.0), upper)
//
// Spots on a plane are `{ x, y }` points (as for `Sketch.polygon`) in the
// plane's frame: Z along the normal,
// the origin where the normal through the world origin meets the plane. For
// a +Z normal, x and y are world X and Y; for -Z, x is mirrored.

type dowel = { diameter: float, length: float }
type key = { width: float, length: float, height: float }

let lift = (z: float, s) => Solid.translate(Vec3.make(0.0, 0.0, z), s)

// Angles of a direction: tilt from +Z, then heading about Z from +X.
let tilt = (v: Vec3.t) => Math.atan2(Math.sqrt(Vec3.x(v) * Vec3.x(v) + Vec3.y(v) * Vec3.y(v)), Vec3.z(v))
let heading = (v: Vec3.t) => Math.atan2(Vec3.y(v), Vec3.x(v))

// A pin standing on the bed (z = 0 to length), the way it prints roundest.
// Ends chamfered 0.5 mm so a pin with a squashed first layer still starts in.
let pin = (d: dowel) =>
  let r = d.diameter / 2.0 in
  let c = Math.min(0.5, Math.min(r, d.length / 2.0) / 2.0) in
  Solid.union([
    Solid.cone(r - c, r, c) |> lift(c / 2.0),
    Solid.cylinder(r, d.length - 2.0 * c) |> lift(d.length / 2.0),
    Solid.cone(r, r - c, c) |> lift(d.length - c / 2.0),
  ])

// The hole for a pin, centered on z = 0 to cut across a split: Ø + 2·gap,
// and length / 2 + gap deep on each side so the pin never holds the cut open
// (a negative gap tightens the sides, never the depth).
let hole = (fit: Fit.t, d: dowel) =>
  Solid.cylinder(d.diameter / 2.0 + fit.gap, d.length + 2.0 * Math.max(fit.gap, 0.0))

// A key's tab, to fuse onto the half below a plane: `height` proud of the
// cut face, reaching 1 mm into the half.
let tab = (k: key) =>
  Solid.box(k.width, k.length, k.height + 1.0) |> lift((k.height - 1.0) / 2.0)

// The tab's slot, to cut from the half above: grown by the gap on every
// side, and 1 mm below the face so the cut leaves no skin.
let slot = (fit: Fit.t, k: key) =>
  let depth = k.height + Math.max(fit.gap, 0.0) in
  Solid.box(k.width + 2.0 * fit.gap, k.length + 2.0 * fit.gap, depth + 1.0) |> lift((depth - 1.0) / 2.0)

// `s`, built at the origin with Z up, moved onto `plane` at `at`: Z turns
// to the plane's normal by the shortest rotation.
let onPlane = (plane: Solid.plane, at: Input.point2, s) =>
  let n = plane.normal in
  if Vec3.length(n) == 0.0 then Binary.invalid("Connector: a plane's normal can't be zero")
  else
    s
      |> Solid.translate(Vec3.make(at.x, at.y, 0.0))
      |> Solid.rotateZ(Angle.radians(0.0 - heading(n)))
      |> Solid.rotateY(Angle.radians(tilt(n)))
      |> Solid.rotateZ(Angle.radians(heading(n)))
      |> Solid.translate(Vec3.scale(plane.offset / Vec3.length(n), Vec3.normalize(n)))

// `Solid.split` with a dowel hole across the cut at each spot, so both
// halves get matching holes. Print one `pin(d)` per spot.
let dowelled = (plane: Solid.plane, fit: Fit.t, d: dowel, spots: List<Input.point2>, s) =>
  let holes = Solid.group(List.map((at: Input.point2) => onPlane(plane, at, hole(fit, d)), spots)) in
  Solid.split(plane, s |> Solid.subtract(holes))

// `Solid.split` with a key at `at`: the tab on the half below, its slot in
// the half above. The face under the tab must be solid.
let keyed = (plane: Solid.plane, fit: Fit.t, k: key, at: Input.point2, s) =>
  let (above, below) = Solid.split(plane, s) in
  (above |> Solid.subtract(onPlane(plane, at, slot(fit, k))), Solid.union([below, onPlane(plane, at, tab(k))]))

// Turn `s` so its side facing `face` (an outward direction) lies on the bed:
// `face` turns to -Z, the lowest point to z = 0, centered on X and Y. A half
// above a plane lies on its cut face with `face` = the normal reversed; the
// half below, with `face` = the normal.
let layFlat = (face: Vec3.t, s) =>
  if Vec3.length(face) == 0.0 then Binary.invalid("Connector.layFlat: the face direction can't be zero")
  else
    let turned =
      s |> Solid.rotateZ(Angle.radians(0.0 - heading(face))) |> Solid.rotateY(Angle.radians(Math.pi - tilt(face))) in
    let b = Solid.bounds(turned) in
    turned
      |> Solid.translate(Vec3.make(
        0.0 - (Vec3.x(b.min) + Vec3.x(b.max)) / 2.0,
        0.0 - (Vec3.y(b.min) + Vec3.y(b.max)) / 2.0,
        0.0 - Vec3.z(b.min)))
