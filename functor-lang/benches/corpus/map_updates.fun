// BENCH: persistent Map updates -- inserts 1.5k ascending keys into an
// immutable map, then removes every key. Measures the tree search plus the
// root-to-leaf path copies of persistent updates.
//
// Convention: `main` is the timed unit of work. Also:
// `functor-lang run map_updates.fun`.
let main = () =>
  let keys = List.range(1500) in
  let populated =
    keys |> List.fold((map, key) => map |> Map.insert(key, key), Map.empty())
  in
  keys |> List.fold((map, key) => map |> Map.remove(key), populated)
