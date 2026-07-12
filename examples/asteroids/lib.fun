// lib.fun — module Lib: the one helper Asteroids still needs beyond the
// builtin registry. Everything else (length/append/flatten/any, &&/||/not,
// Math.max) is now a builtin — see the exercise's findings log.

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
  List.reverse(out)
