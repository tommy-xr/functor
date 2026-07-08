// Positions and velocities — record types, record literals, field access,
// arithmetic, and unary minus (the roadmap's `move` example).

type Position = { x: float, y: float }
type Velocity = { dx: float, dy: float }

let origin = { x: 0.0, y: 0.0 }

let move = (p: Position, v: Velocity, dt: float): Position =>
  { x: p.x + v.dx * dt, y: p.y + v.dy * dt }

let delta = (a: Position, b: Position): Position =>
  { x: a.x - b.x, y: a.y - b.y }

let mirror = (p: Position): Position => { x: -p.x, y: p.y }

let isOrigin = (p: Position): bool => p.x == 0.0

// Record update syntax + an expression-level `let … in`.
let nudge = (p: Position): Position => { p with x: p.x + 1.0 }

let main = () =>
  let moved = move(origin, { dx: 3.0, dy: 4.0 }, 0.5) in
  nudge(moved)
