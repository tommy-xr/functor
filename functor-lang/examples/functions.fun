// Higher-order functions, inline lambdas, and generic type annotations.

type Player = { name: string, scores: List<float> }

let add = (a: float, b: float): float => a + b

let average = (a, b) => (a + b) / 2.0

let total = (p: Player): float => p.scores |> List.fold(add, 0.0)

// `List.maximum` is partial, so this answers `Option.t<float>` — an empty
// roster is `Option.None` rather than an error. (It stays UNANNOTATED
// because this golden is checked as a lone module, without the bundled
// `Option` stdlib that makes `Option.t` a resolvable type name.)
let bestTotal = (players: List<Player>) =>
  players |> List.map((p) => total(p)) |> List.maximum

let isWinner = (p, cutoff) => cutoff < total(p)

let debug = false

let main = () =>
  bestTotal([
    { name: "ada", scores: [12.0, 30.0] },
    { name: "grace", scores: [8.0, 4.0] },
  ])
