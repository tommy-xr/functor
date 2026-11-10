// JsonDecode — JSON decoders as values: `Decode` for `Json.t` instead of
// bytes. A decoder reads a value out of a piece of JSON; compose small ones
// (numbers, text, fields, lists) into a whole shape and `parse` text with it.
// A failure is a `Json.error` with the path of fields and list indices that
// led there: `poses[2].x: expected a number, got a string`.
//
//   type point = { x: float, y: float }
//   let point = JsonDecode.map2((x, y): point => { x: x, y: y },
//     JsonDecode.field("x", JsonDecode.number), JsonDecode.field("y", JsonDecode.number))
//   let points = JsonDecode.parse(JsonDecode.field("points", JsonDecode.list(point)), text)

// Reads a value out of JSON, or fails.
type t<'a> = { read: (Json.t) => Result.t<'a, Json.error> }

// Helpers for writing decoders: an error here, and an error with `segment`
// added in front of its path.
let failure = (message: string): Json.error => { path: [], message: message }

let within = (segment: Binary.segment, e: Json.error): Json.error =>
  { e with path: List.append(e.path, [segment]) }

// What a piece of JSON is, for messages.
let kind = (json: Json.t): string =>
  match json with
  | Json.Null => "null"
  | Json.Bool(_) => "a bool"
  | Json.Number(_) => "a number"
  | Json.String(_) => "a string"
  | Json.Array(_) => "an array"
  | Json.Object(_) => "an object"

let mismatch = (wanted: string, json: Json.t): Result.t<'a, Json.error> =>
  Result.Error(failure(Text.join("", ["expected ", wanted, ", got ", kind(json)])))

// Always `value`, reading nothing.
let succeed = (value: 'a): t<'a> => { read: (json) => Result.Ok(value) }

// Always fails with `message`.
let fail = (message: string): t<'a> => { read: (json) => Result.Error(failure(message)) }

// The leaves: a JSON number, string or bool, and the JSON itself.
let number: t<float> =
  { read: (json) => match json with | Json.Number(n) => Result.Ok(n) | _ => mismatch("a number", json) }

let string: t<string> =
  { read: (json) => match json with | Json.String(s) => Result.Ok(s) | _ => mismatch("a string", json) }

let bool: t<bool> =
  { read: (json) => match json with | Json.Bool(b) => Result.Ok(b) | _ => mismatch("a bool", json) }

let value: t<Json.t> = { read: (json) => Result.Ok(json) }

// The decoded value, transformed.
let map = (f: ('a) => 'b, d: t<'a>): t<'b> =>
  { read: (json) => match d.read(json) with | Result.Ok(a) => Result.Ok(f(a)) | Result.Error(e) => Result.Error(e) }

// Decode with `d`, then with the decoder `f` chooses from its value — for a
// `version` or `type` field that shapes the rest. Both read the same JSON.
let andThen = (f: ('a) => t<'b>, d: t<'a>): t<'b> =>
  { read: (json) =>
      match d.read(json) with
      | Result.Ok(a) =>
        let next = f(a) in
        next.read(json)
      | Result.Error(e) => Result.Error(e) }

// A decoder built when first needed, so a decoder can use itself:
// `let tree = JsonDecode.field("children", JsonDecode.list(JsonDecode.lazy(() => tree)))`.
// Each level of such a tree nests the evaluation, so keep recursive data
// shallow (tens of levels at most).
let lazy = (make: () => t<'a>): t<'a> =>
  { read: (json) =>
      let d = make() in
      d.read(json) }

// Several decoders over the same JSON (usually `field`s of one object),
// combined; the first to fail is the error. Written flat rather than through
// `andThen`, so a wide object costs no more nesting than a narrow one.
let map2 = (f: ('a, 'b) => 'c, a: t<'a>, b: t<'b>): t<'c> =>
  { read: (json) =>
      match a.read(json) with
      | Result.Error(e) => Result.Error(e)
      | Result.Ok(x) => match b.read(json) with
      | Result.Error(e) => Result.Error(e)
      | Result.Ok(y) => Result.Ok(f(x, y)) }

let map3 = (f: ('a, 'b, 'c) => 'd, a: t<'a>, b: t<'b>, c: t<'c>): t<'d> =>
  { read: (json) =>
      match a.read(json) with
      | Result.Error(e) => Result.Error(e)
      | Result.Ok(x) => match b.read(json) with
      | Result.Error(e) => Result.Error(e)
      | Result.Ok(y) => match c.read(json) with
      | Result.Error(e) => Result.Error(e)
      | Result.Ok(z) => Result.Ok(f(x, y, z)) }

let map4 = (f: ('a, 'b, 'c, 'd) => 'e, a: t<'a>, b: t<'b>, c: t<'c>, d: t<'d>): t<'e> =>
  { read: (json) =>
      match a.read(json) with
      | Result.Error(e) => Result.Error(e)
      | Result.Ok(x) => match b.read(json) with
      | Result.Error(e) => Result.Error(e)
      | Result.Ok(y) => match c.read(json) with
      | Result.Error(e) => Result.Error(e)
      | Result.Ok(z) => match d.read(json) with
      | Result.Error(e) => Result.Error(e)
      | Result.Ok(w) => Result.Ok(f(x, y, z, w)) }

let map5 =
  (f: ('a, 'b, 'c, 'd, 'e) => 'f, a: t<'a>, b: t<'b>, c: t<'c>, d: t<'d>, e: t<'e>): t<'f> =>
  { read: (json) =>
      match a.read(json) with
      | Result.Error(err) => Result.Error(err)
      | Result.Ok(x) => match b.read(json) with
      | Result.Error(err) => Result.Error(err)
      | Result.Ok(y) => match c.read(json) with
      | Result.Error(err) => Result.Error(err)
      | Result.Ok(z) => match d.read(json) with
      | Result.Error(err) => Result.Error(err)
      | Result.Ok(w) => match e.read(json) with
      | Result.Error(err) => Result.Error(err)
      | Result.Ok(v) => Result.Ok(f(x, y, z, w, v)) }

// The object's field `name`, read with `d`; a failure inside names it.
let field = (name: string, d: t<'a>): t<'a> =>
  { read: (json) =>
      match json with
      | Json.Object(fields) =>
        (match Map.get(name, fields) with
         | Option.Some(inner) => d.read(inner)
         | Option.None => Result.Error(failure("missing")))
        |> Result.mapError((e) => within(Binary.Field(name), e))
      | _ => mismatch(Text.join("", ["an object with \"", name, "\""]), json) }

// `field`, but `Option.None` when the object has no such field or it is null.
let optionalField = (name: string, d: t<'a>): t<Option.t<'a>> =>
  { read: (json) =>
      match json with
      | Json.Object(fields) =>
        (match Map.get(name, fields) |> Option.defaultValue(Json.Null) with
         | Json.Null => Result.Ok(Option.None)
         | inner => d.read(inner) |> Result.map((a) => Option.Some(a)))
        |> Result.mapError((e) => within(Binary.Field(name), e))
      | _ => mismatch(Text.join("", ["an object with \"", name, "\""]), json) }

// Every item of an array, read with `d`; a failure names its index.
let list = (d: t<'a>): t<List<'a>> =>
  { read: (json) =>
      match json with
      | Json.Array(items) =>
        items
          |> List.tryMapAccum((i: float, item: Json.t) =>
            match d.read(item) with
            | Result.Ok(a) => Result.Ok((i + 1.0, a))
            | Result.Error(e) => Result.Error(within(Binary.Index(i), e)), 0.0)
          |> Result.map((done) => match done with | (count, values) => values)
      | _ => mismatch("an array", json) }

// The array's item at `i` (from 0), read with `d`.
let index = (i: float, d: t<'a>): t<'a> =>
  { read: (json) =>
      match json with
      | Json.Array(items) =>
        if i < 0.0 || i > 1000000000.0 || i != Math.floor(i)
        then Result.Error(failure("an index must be a whole number from 0"))
        else
          (match List.nth(i, items) with
           | Option.Some(inner) => d.read(inner)
           | Option.None => Result.Error(failure("missing")))
          |> Result.mapError((e) => within(Binary.Index(i), e))
      | _ => mismatch("an array", json) }

// Decode a piece of JSON.
let run = (d: t<'a>, json: Json.t): Result.t<'a, Json.error> => d.read(json)

// Decode JSON text; text that isn't JSON fails with an empty path.
let parse = (d: t<'a>, text: string): Result.t<'a, Json.error> =>
  Json.parse(text) |> Result.mapError(failure) |> Result.bind((json) => d.read(json))

// The text of an error: `poses[2].x: expected a number, got a string`.
let describe = (e: Json.error): string =>
  let path =
    e.path |> List.fold((text: string, segment: Binary.segment) =>
      match segment with
      | Binary.Field(name) => if text == "" then name else Text.join(".", [text, name])
      | Binary.Index(i) => Text.join("", [text, "[", Text.fromFloat(i), "]"]), "") in
  if path == "" then e.message else Text.join(": ", [path, e.message])
