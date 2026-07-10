// BENCH: self-recursion -- a 50k fold whose step calls `sumTo`, which recurses
// to a fixed depth of 30 via a bool-literal match base case (Functor Lang has no loops;
// iteration that is not a List.* builtin is recursion). ~1.5M recursive calls
// total. Measures call/return + environment overhead of Functor Lang recursion.
//
// Depth per call is kept shallow: the interpreter caps evaluation nesting
// (MAX_EVAL_DEPTH), so unbounded user recursion is intentionally not a thing --
// deep iteration belongs in List.fold. Here recursion is exercised at volume
// (many shallow recursions) rather than one deep one.
//
// Convention: `main` is the timed unit of work. Also: `functor-lang run recursion.fun`.
let sumTo = (n, acc) =>
  match n < 1.0 with
  | true => acc
  | false => sumTo(n - 1.0, acc + n)
let main = () =>
  List.fold((acc, x) => acc + sumTo(30.0, 0.0), 0.0, List.range(50000))
