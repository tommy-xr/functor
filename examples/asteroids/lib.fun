// lib.fun — module Lib: tiny helpers Asteroids needs that the builtin
// registry doesn't cover today (no List.length/append/flatten/any, and no
// boolean &&/||/! operators — see the exercise's findings log).

let length = (xs) =>
  xs |> List.fold((acc, x) => acc + 1.0, 0.0)

let and = (a, b) =>
  match a with
  | true => b
  | false => false

let or = (a, b) =>
  match a with
  | true => true
  | false => b

// True when any element satisfies pred. Subject-LAST so it pipes:
// `rocks |> Lib.any(pred)`.
let any = (pred, xs) =>
  xs |> List.fold((acc, x) => (match acc with | true => true | false => pred(x)), false)

// NOTE: a naive recursive append hits the interpreter's evaluation-depth
// cap (~200 frames) at list lengths well under 100, so these are built on
// the iterative List.fold instead (findings log: no builtin
// List.reverse/append/flatten).
let reverse = (xs) =>
  xs |> List.fold((acc, x) => [x, ..acc], [])

// a ++ b: prepend a's elements onto b, walking a back to front.
let append = (a, b) =>
  reverse(a) |> List.fold((acc, x) => [x, ..acc], b)

let flatten = (xss) =>
  reverse(xss) |> List.fold((acc, xs) => append(xs, acc), [])

// Remove the FIRST element satisfying pred (if any), preserving order.
let removeFirst = (pred, xs) =>
  let folded = xs |> List.fold((acc, x) =>
    let (out, removed) = acc in
    (match removed with
     | true => ([x, ..out], true)
     | false => (match pred(x) with
                 | true => (out, true)
                 | false => ([x, ..out], false))),
    ([], false)) in
  let (out, _) = folded in
  reverse(out)

// Clamp v to at least lo (no Math.max builtin).
let floorAt = (lo, v) =>
  match v < lo with
  | true => lo
  | false => v
