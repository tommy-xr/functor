// Stations: where each station stands in the lab, what it is built from, and
// the rig at it. World space is metres, Y up, the floor at y = 0; the hub is
// the origin and stations sit 4-5 m out from it.

type Box = { center: Input.point3, size: Input.point3 }

/// A station: the floor point the player stands on (`feet`, the floor-locked
/// rig's origin) and the way they face (`yaw`, radians about +Y; 0 = +Z).
type Station = { id: float, name: string, feet: Input.point3, yaw: float }

let grabTable: Station = { id: 1.0, name: "grab table", feet: Quat.v3(0.0, 0.0, 4.0), yaw: 0.0 }

// Facing -Z, so its rig-local frame is the world's, moved to its feet.
let inventory: Station = { id: 2.0, name: "inventory", feet: Quat.v3(0.0, 0.0, -4.0), yaw: Math.pi }

let all = [grabTable, inventory]

/// Whether the body slots are worn there.
let worn = (s: Station): bool => s.id == inventory.id

let find = (id: float): Station =>
  all |> List.find((s) => s.id == id) |> Option.defaultValue(grabTable)

/// The station after `id`, wrapping (the headset's cycle button).
let after = (id: float): float =>
  let later = all |> List.filter((s) => s.id > id) in
  match List.head(later) with
  | Option.Some(s) => s.id
  | Option.None => List.head(all) |> Option.map((s) => s.id) |> Option.defaultValue(id)

expect after(grabTable.id) == inventory.id && after(inventory.id) == grabTable.id

// --- the rig ---
// Tracked space is +X right, -Z forward; a station faces +Z turned by its
// yaw, so rig-local is the world turned a half turn more about +Y.

let turn = (s: Station) => Quat.yaw(s.yaw + Math.pi)
let toWorld = (s: Station, p: Input.point3): Input.point3 => Quat.add(s.feet, Quat.rotate(turn(s), p))
let toRig = (s: Station, p: Input.point3): Input.point3 => Quat.rotate(Quat.conj(turn(s)), Quat.sub(p, s.feet))
let dirToWorld = (s: Station, v: Input.point3): Input.point3 => Quat.rotate(turn(s), v)
let dirToRig = (s: Station, v: Input.point3): Input.point3 => Quat.rotate(Quat.conj(turn(s)), v)

/// The authored rig camera: level, at the feet, as `Xr.floorLocked` wants.
let rig = (s: Station) =>
  Camera3D.firstPerson(Quat.toVec3(s.feet), Angle.radians(s.yaw), 0rad, 90deg) |> Camera3D.clip(0.05, 60.0)

// The engine maps a tracked pose the same way.
expect (
  let s = { grabTable with feet: Quat.v3(1.0, 0.0, -2.0), yaw: 0.6 } in
  let p = Quat.v3(0.2, 1.3, -0.5) in
  let mapped = Camera3D.mapTrackedPose(rig(s), { position: p, orientation: Quat.identity }) in
  Hold.near3(toWorld(s, p), mapped.position)
    && Hold.near3(dirToWorld(s, Quat.v3(0.0, 0.0, -1.0)), mapped.forward)
    && Hold.near3(toRig(s, toWorld(s, p)), p)
)

// --- the grab table ---
// Ahead of the feet: a counter-height table with the props on it, a wall
// against its left end (the player's left is world +X here) to push them
// into, and a basket on the floor a throw away.

let tableTop = 0.9
let table: Box = { center: Quat.v3(-0.125, tableTop / 2.0, 4.6), size: Quat.v3(1.45, tableTop, 0.6) }
let wall: Box = { center: Quat.v3(0.65, 1.0, 4.6), size: Quat.v3(0.1, 2.0, 1.6) }

/// Where each prop rests on the table (rig-local, as `Hold` holds it): the
/// ball ahead, the stick between it and the wall pointing away with its near
/// end over the table's edge, and on the right a brick, turned, and a bottle
/// standing.
let spawn = (k: Props.Kind): Input.pose =>
  let on = (x: float, z: float, q: Input.quaternion) => Hold.at(Quat.v3(x, tableTop + (Props.extent(k)).y / 2.0, z), q) in
  match k with
  | Props.Ball => on(0.2, -0.55, Quat.identity)
  | Props.Stick => on(-0.3, -0.6, Quat.identity)
  | Props.Brick => on(0.6, -0.5, Quat.yaw(35.0 * Math.pi / 180.0))
  | Props.Bottle => on(0.62, -0.8, Quat.identity)

// The basket: an open box, its inside `basketInner` wide and `basketDepth` deep.
let basketAt = Quat.v3(-0.2, 0.0, 7.1)
let basketInner = 0.6
let basketDepth = 0.35
let basketWall = 0.03
let basket: List<Box> =
  let half = (basketInner + basketWall) / 2.0 in
  let side = (dx: float, dz: float, sx: float, sz: float): Box =>
    { center: Quat.add(basketAt, Quat.v3(dx, basketDepth / 2.0, dz)), size: Quat.v3(sx, basketDepth, sz) } in
  [
    { center: Quat.add(basketAt, Quat.v3(0.0, basketWall / 2.0, 0.0)), size: Quat.v3(basketInner, basketWall, basketInner) },
    side(half, 0.0, basketWall, basketInner + 2.0 * basketWall),
    side(0.0 - half, 0.0, basketWall, basketInner + 2.0 * basketWall),
    side(0.0, half, basketInner, basketWall),
    side(0.0, 0.0 - half, basketInner, basketWall),
  ]

/// Inside the basket, below its rim.
let inBasket = (p: Input.point3): bool =>
  Math.abs(p.x - basketAt.x) < basketInner / 2.0
    && Math.abs(p.z - basketAt.z) < basketInner / 2.0
    && p.y < basketAt.y + basketDepth

expect inBasket(Quat.add(basketAt, Quat.v3(0.1, 0.1, -0.1)))
expect not inBasket(Quat.add(basketAt, Quat.v3(0.4, 0.1, 0.0)))
expect not inBasket(toWorld(grabTable, (spawn(Props.Ball)).position))

// The scoreboard hangs behind the basket, facing the player.
let boardAt = Quat.add(basketAt, Quat.v3(0.0, 1.5, 0.6))

let floor: Box = { center: Quat.v3(0.0, -0.05, 0.0), size: Quat.v3(30.0, 0.1, 30.0) }

// --- the inventory ---
// The same table at the same place ahead of the feet, so the props spawn on
// it; no wall, no basket.

let bench: Box = { center: toWorld(inventory, toRig(grabTable, table.center)), size: table.size }

expect Hold.near3(bench.center, Quat.v3(0.125, tableTop / 2.0, -4.6))

/// The lab's fixed geometry, tagged for the `physics` hook.
let solids: List<(string, Box)> =
  [("floor", floor), ("table", table), ("wall", wall), ("bench", bench)]
    |> List.append(basket |> List.indexedMap((i, b) => ($"basket-{i}", b)))
