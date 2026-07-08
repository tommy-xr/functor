// Tuples: `(a, b)` literals (two or more elements — `(e)` is grouping),
// `(float, string)` product annotations, tuple patterns in `match`, and the
// destructuring let (`let (a, b) = e in …`, sugar for a single-arm match).
// Multiple returns without naming a record — B6's `(model, effects)` shape.

type Msg =
  | Moved(dx: float, dy: float)

// A function returning two things at once.
let minMax = (a: float, b: float): (float, float) =>
  match a < b with
  | true => (a, b)
  | false => (b, a)

let span = (a, b) =>
  let (lo, hi) = minMax(a, b) in
  hi - lo

// Tuple patterns match by exact arity; sub-patterns are names or `_`.
let describe = (pair: (float, string)): string =>
  match pair with
  | (n, label) => Text.concat(label, Text.fromFloat(n))

// Tuples nest in data structures and messages like any value.
let step = (msg: Msg): (float, float) =>
  match msg with
  | Moved(dx, dy) => (dx * 2.0, dy * 2.0)

let main = () =>
  let (vx, vy) = step(Moved(1.5, -0.5)) in
  (span(9.0, 3.0), describe((vx, "vx: ")), (vx, vy) == (3.0, -1.0))
