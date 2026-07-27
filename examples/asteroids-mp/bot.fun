// bot.fun — a tiny scripted pilot standing in for a remote peer.
//
// The bot owns no world state: each tick the client asks it for an Intent
// (the same plain record a keyboard produces) and folds it in as a
// `Protocol.Steer` through `Server.recv`, exactly as a remote player's
// packets will arrive once the netsim transport lands.
//
// The policy is deliberately tiny: point the nose at the nearest rock,
// thrust while it is far, hold fire while roughly aimed — the shared
// cooldown in `Server.step` paces the actual shots.

let intent = (ship: Protocol.Ship, rocks: List<Protocol.Rock>): Protocol.Intent =>
  let nearest =
    rocks
      |> List.sortBy((r) => Protocol.dist2(ship.x, ship.y, r.x, r.y))
      |> List.head in
  match nearest with
  | Option.None => Server.coast
  | Option.Some(r) =>
    let dx = Protocol.wrapDelta(r.x - ship.x, Protocol.halfW) in
    let dy = Protocol.wrapDelta(r.y - ship.y, Protocol.halfH) in
    // The nose at angle a points (-sin a, cos a), so the aim angle is
    // atan2(-dx, dy); steer by the wrapped angle difference.
    let want = Math.atan2(0.0 - dx, dy) in
    let diff = Math.mod(want - ship.angle + Math.pi, Math.pi * 2.0) - Math.pi in
    { turn: Math.sign(diff),
      thrust: dx * dx + dy * dy > 36.0,
      fire: Math.abs(diff) < 0.4 }

expect (
  // A rock dead ahead: no turn needed, fire held, close enough to coast.
  let rock = { x: 0.0, y: 6.0, vx: 0.0, vy: 0.0, size: 3.0 } in
  let i = intent(Server.newShip(1.0), [rock]) in
  i.fire && i.turn == 0.0 && not i.thrust
)
