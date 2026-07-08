// Format player scores as a bulleted report — pipelines, qualified names,
// strings, and comparisons.

let threshold = 10

let isHigh = (score: float): bool => score > threshold

let describe = (score) => Text.concat("score: ", Text.fromFloat(score))

// Pipelines bind loosest: the division happens before the `|>`.
let normalized = (score) => score / 100.0 |> Math.clamp01

let report = (scores) =>
  scores
    |> List.filter(isHigh)
    |> List.map(describe)
    |> Text.toBullets

let main = () => report([12.0, 3.5, 40.0])
