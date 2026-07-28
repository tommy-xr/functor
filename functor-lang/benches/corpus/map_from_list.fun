// BENCH: deterministic Map bulk construction + lookup -- builds a 20k-entry
// map from reversed input, then performs 20k binary-search reads. Exercises
// canonical sorting, tuple/list materialization, Option construction, and the
// lookup hot path together.
//
// Convention: `main` is the timed unit of work. Also:
// `functor-lang run map_from_list.fun`.
let main = () =>
  let entries =
    List.range(20000)
      |> List.reverse
      |> List.map((key) => (key, key * 2.0))
  in
  let map = Map.fromList(entries) in
  List.range(20000)
    |> List.fold(
      (sum, key) =>
        match map |> Map.get(key) with
        | Option.Some(value) => sum + value
        | Option.None => sum,
      0.0
    )
