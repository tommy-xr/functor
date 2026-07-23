// A successful value or an error. Helpers are subject-last so they compose
// with `|>`.

type t<'value, 'error> =
  | Ok(value: 'value)
  | Error(error: 'error)

// Transform a successful value; leave an error unchanged.
let map = (fn: ('value) => 'mapped, result: t<'value, 'error>): t<'mapped, 'error> =>
  match result with
  | Ok(value) => Ok(fn(value))
  | Error(error) => Error(error)

// Transform an error; leave a successful value unchanged.
let mapError = (fn: ('error) => 'mapped, result: t<'value, 'error>): t<'value, 'mapped> =>
  match result with
  | Ok(value) => Ok(value)
  | Error(error) => Error(fn(error))

// Continue with a result-producing computation after success.
let bind =
  (fn: ('value) => t<'mapped, 'error>, result: t<'value, 'error>): t<'mapped, 'error> =>
    match result with
    | Ok(value) => fn(value)
    | Error(error) => Error(error)

// Return the successful value, or an eager fallback for an error.
let defaultValue = (fallback: 'value, result: t<'value, 'error>): 'value =>
  match result with
  | Ok(value) => value
  | Error(_) => fallback

// Return the successful value, or compute a fallback from the error.
let defaultWith = (fallback: ('error) => 'value, result: t<'value, 'error>): 'value =>
  match result with
  | Ok(value) => value
  | Error(error) => fallback(error)

// Whether the result is successful.
let isOk = (result: t<'value, 'error>): bool =>
  match result with
  | Ok(_) => true
  | Error(_) => false

// Whether the result contains an error.
let isError = (result: t<'value, 'error>): bool =>
  match result with
  | Ok(_) => false
  | Error(_) => true

// Convert `Ok(value)` to `Option.Some(value)` and an error to `Option.None`.
let toOption = (result: t<'value, 'error>): Option.t<'value> =>
  match result with
  | Ok(value) => Option.Some(value)
  | Error(_) => Option.None
