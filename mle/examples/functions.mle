// Higher-order functions, inline lambdas, and generic type annotations.

type Player = { name: string, scores: List<float> }

let add = (a: float, b: float): float => a + b

let average = (a, b) => (a + b) / 2.0

let total = (p: Player): float => p.scores |> List.fold(add, 0.0)

let bestTotal = (players: List<Player>): float =>
  players |> List.map((p) => total(p)) |> List.maximum

let isWinner = (p, cutoff) => cutoff < total(p)

let debug = false

let main = () =>
  bestTotal([
    { name: "ada", scores: [12.0, 30.0] },
    { name: "grace", scores: [8.0, 4.0] },
  ])
