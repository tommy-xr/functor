// lib.fun — module Lib: the one helper Asteroids still needs that the
// builtin registry doesn't cover today. length/and/or/any/reverse/append/
// flatten/floorAt were all replaced by builtins (List.*, the && / ||
// operators, Math.max); only removeFirst has no builtin equivalent.

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
