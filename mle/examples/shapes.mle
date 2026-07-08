// Shapes as a variant type — constructor declarations and calls, and every
// B5 pattern kind: constructors with bindings, a nullary constructor, `_`
// wildcards (top-level and inside a constructor), bool-literal arms (the
// language's first conditional), and a catch-all variable.

type Shape =
  | Circle(radius: float)
  | Rect(w: float, h: float)
  | Point

let pi = 3.14159

let area = (s: Shape): float =>
  match s with
  | Circle(r) => pi * r * r
  | Rect(w, h) => w * h
  | Point => 0.0

let isRound = (s: Shape): bool =>
  match s with
  | Circle(_) => true
  | _ => false

// The first conditional: match on a bool.
let sizeOf = (s: Shape): string =>
  match area(s) > 10.0 with
  | true => "big"
  | false => "small"

// Arm bodies are full expressions, so this nested match is greedy: it is
// only unambiguous because it sits in the LAST arm — anywhere else it would
// need parentheses (see the parser's grammar notes).
let describe = (s: Shape): string =>
  match s with
  | Point => "a point"
  | other =>
      match isRound(other) with
      | true => Text.concat("a round ", Text.concat(sizeOf(other), " shape"))
      | false => Text.concat("a boxy ", Text.concat(sizeOf(other), " shape"))

let shapes = [Circle(2.0), Rect(3.0, 4.0), Point]

let main = () =>
  shapes |> List.map(describe) |> Text.toBullets
