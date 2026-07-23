// An optional value. Helpers are subject-last so they compose with `|>`.

type t<'value> =
  | Some(value: 'value)
  | None

// Transform a present value; leave `None` unchanged.
let map = (fn: ('value) => 'mapped, option: t<'value>): t<'mapped> =>
  match option with
  | Some(value) => Some(fn(value))
  | None => None

// Continue with an optional computation when a value is present.
let bind = (fn: ('value) => t<'mapped>, option: t<'value>): t<'mapped> =>
  match option with
  | Some(value) => fn(value)
  | None => None

// Return the contained value, or an eager fallback for `None`.
let defaultValue = (fallback: 'value, option: t<'value>): 'value =>
  match option with
  | Some(value) => value
  | None => fallback

// Return the contained value, computing the fallback only for `None`.
let defaultWith = (fallback: () => 'value, option: t<'value>): 'value =>
  match option with
  | Some(value) => value
  | None => fallback()

// Whether the option contains a value.
let isSome = (option: t<'value>): bool =>
  match option with
  | Some(_) => true
  | None => false

// Whether the option is `None`.
let isNone = (option: t<'value>): bool =>
  match option with
  | Some(_) => false
  | None => true

// Keep a present value only when it satisfies the predicate.
let filter = (predicate: ('value) => bool, option: t<'value>): t<'value> =>
  match option with
  | Some(value) =>
      if predicate(value) then Some(value) else None
  | None => None

// Convert `Some(value)` to `[value]` and `None` to `[]`.
let toList = (option: t<'value>): List<'value> =>
  match option with
  | Some(value) => [value]
  | None => []
