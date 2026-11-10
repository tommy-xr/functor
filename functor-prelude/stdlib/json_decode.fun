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
type t<'a> = { decode: (Json.t) => Result.t<'a, Json.error> }

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
let succeed = (value: 'a): t<'a> => { decode: (json) => Result.Ok(value) }

// Always fails with `message`.
let fail = (message: string): t<'a> => { decode: (json) => Result.Error(failure(message)) }

// The leaves: a JSON number, string or bool, and the JSON itself.
let number: t<float> =
  { decode: (json) => match json with | Json.Number(n) => Result.Ok(n) | _ => mismatch("a number", json) }

let string: t<string> =
  { decode: (json) => match json with | Json.String(s) => Result.Ok(s) | _ => mismatch("a string", json) }

let bool: t<bool> =
  { decode: (json) => match json with | Json.Bool(b) => Result.Ok(b) | _ => mismatch("a bool", json) }

let value: t<Json.t> = { decode: (json) => Result.Ok(json) }

// The decoded value, transformed.
let map = (f: ('a) => 'b, d: t<'a>): t<'b> => { decode: (json) => d.decode(json) |> Result.map(f) }

// Decode with `d`, then with the decoder `f` chooses from its value — for a
// `version` or `type` field that shapes the rest. Both read the same JSON.
let andThen = (f: ('a) => t<'b>, d: t<'a>): t<'b> =>
  { decode: (json) =>
      d.decode(json)
        |> Result.bind((a) =>
          let next = f(a) in
          next.decode(json)) }

// Several decoders over the same JSON (usually `field`s of one object),
// combined.
let map2 = (f: ('a, 'b) => 'c, a: t<'a>, b: t<'b>): t<'c> =>
  a |> andThen((x) => b |> map((y) => f(x, y)))

let map3 = (f: ('a, 'b, 'c) => 'd, a: t<'a>, b: t<'b>, c: t<'c>): t<'d> =>
  a |> andThen((x) => map2((y, z) => f(x, y, z), b, c))

let map4 = (f: ('a, 'b, 'c, 'd) => 'e, a: t<'a>, b: t<'b>, c: t<'c>, d: t<'d>): t<'e> =>
  a |> andThen((x) => map3((y, z, w) => f(x, y, z, w), b, c, d))

let map5 =
  (f: ('a, 'b, 'c, 'd, 'e) => 'f, a: t<'a>, b: t<'b>, c: t<'c>, d: t<'d>, e: t<'e>): t<'f> =>
  a |> andThen((x) => map4((y, z, w, v) => f(x, y, z, w, v), b, c, d, e))

// The object's field `name`, read with `d`; a failure inside names it.
let field = (name: string, d: t<'a>): t<'a> =>
  { decode: (json) =>
      match json with
      | Json.Object(fields) =>
        (match Map.get(name, fields) with
         | Option.Some(inner) => d.decode(inner)
         | Option.None => Result.Error(failure("missing")))
        |> Result.mapError((e) => within(Binary.Field(name), e))
      | _ => mismatch(Text.join("", ["an object with \"", name, "\""]), json) }

// `field`, but `Option.None` when the object has no such field or it is null.
let optionalField = (name: string, d: t<'a>): t<Option.t<'a>> =>
  { decode: (json) =>
      match json with
      | Json.Object(fields) =>
        (match Map.get(name, fields) |> Option.defaultValue(Json.Null) with
         | Json.Null => Result.Ok(Option.None)
         | inner => d.decode(inner) |> Result.map((a) => Option.Some(a)))
        |> Result.mapError((e) => within(Binary.Field(name), e))
      | _ => mismatch(Text.join("", ["an object with \"", name, "\""]), json) }

// Every item of an array, read with `d`; a failure names its index.
let list = (d: t<'a>): t<List<'a>> =>
  { decode: (json) =>
      match json with
      | Json.Array(items) =>
        items
          |> List.tryMapAccum((i: float, item: Json.t) =>
            match d.decode(item) with
            | Result.Ok(a) => Result.Ok((i + 1.0, a))
            | Result.Error(e) => Result.Error(within(Binary.Index(i), e)), 0.0)
          |> Result.map((done) => match done with | (count, values) => values)
      | _ => mismatch("an array", json) }

// The array's item at `i` (from 0), read with `d`.
let index = (i: float, d: t<'a>): t<'a> =>
  { decode: (json) =>
      match json with
      | Json.Array(items) =>
        (match List.nth(i, items) with
         | Option.Some(inner) => d.decode(inner)
         | Option.None => Result.Error(failure("missing")))
        |> Result.mapError((e) => within(Binary.Index(i), e))
      | _ => mismatch("an array", json) }

// Decode a piece of JSON.
let run = (d: t<'a>, json: Json.t): Result.t<'a, Json.error> => d.decode(json)

// Decode JSON text; text that isn't JSON fails with an empty path.
let parse = (d: t<'a>, text: string): Result.t<'a, Json.error> =>
  Json.parse(text) |> Result.mapError(failure) |> Result.bind((json) => d.decode(json))

// The text of an error: `poses[2].x: expected a number, got a string`.
let describe = (e: Json.error): string =>
  let path =
    e.path |> List.fold((text: string, segment: Binary.segment) =>
      match segment with
      | Binary.Field(name) => if text == "" then name else Text.join(".", [text, name])
      | Binary.Index(i) => Text.join("", [text, "[", Text.fromFloat(i), "]"]), "") in
  if path == "" then e.message else Text.join(": ", [path, e.message])
