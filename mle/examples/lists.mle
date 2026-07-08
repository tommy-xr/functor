// List patterns and cons — `[a, b]` / `[]` / `[head, ..rest]` in match, and
// `[x, ..xs]` prepend in expressions. Refutable, so a list match needs a
// catch-all (`_`, a name, or `[..rest]`).

let sum = (xs: List<float>): float =>
  match xs with
  | [] => 0.0
  | [head, ..rest] => head + sum(rest)

let firstOr = (xs: List<float>, fallback: float): float =>
  match xs with
  | [x, ..rest] => x
  | [] => fallback

let pair = (xs: List<float>): (float, float) =>
  match xs with
  | [a, b] => (a, b)
  | _ => (0.0, 0.0)

let main = () =>
  let xs = [1.0, 2.0, 3.0] in
  (sum(xs), firstOr(xs, 0.0), pair([4.0, 5.0]), [0.0, ..xs])
