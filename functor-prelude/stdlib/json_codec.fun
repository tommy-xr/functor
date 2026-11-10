// JsonCodec — a JSON decoder and encoder for the same value, defined once:
// `Codec` for JSON. Read text with `decode`, write it with `encode`, and test
// that they agree (`expect JsonCodec.decode(c, JsonCodec.encode(c, x)) == Result.Ok(x)`).
//
//   type point = { x: float, y: float }
//   let point = JsonCodec.object2((x, y): point => { x: x, y: y },
//     "x", (p: point) => p.x, JsonCodec.number, "y", (p: point) => p.y, JsonCodec.number)

type t<'a> = { reads: JsonDecode.t<'a>, writes: ('a) => Json.t }

let number: t<float> = { reads: JsonDecode.number, writes: JsonEncode.number }
let string: t<string> = { reads: JsonDecode.string, writes: JsonEncode.string }
let bool: t<bool> = { reads: JsonDecode.bool, writes: JsonEncode.bool }

// A value stored as another: `to` after decoding, `from` before encoding.
let map = (to: ('a) => 'b, from: ('b) => 'a, c: t<'a>): t<'b> =>
  { reads: JsonDecode.map(to, c.reads), writes: (b: 'b) => c.writes(from(b)) }

// An array of values.
let list = (c: t<'a>): t<List<'a>> =>
  { reads: JsonDecode.list(c.reads), writes: (items: List<'a>) => JsonEncode.list(c.writes, items) }

// An object of two fields: how to build the record, and each field's name,
// getter and codec.
let object2 =
  (make: ('a, 'b) => 'r, nameA: string, getA: ('r) => 'a, a: t<'a>,
   nameB: string, getB: ('r) => 'b, b: t<'b>): t<'r> =>
  { reads: JsonDecode.map2(make, JsonDecode.field(nameA, a.reads), JsonDecode.field(nameB, b.reads)),
    writes: (r: 'r) => JsonEncode.object([(nameA, a.writes(getA(r))), (nameB, b.writes(getB(r)))]) }

let object3 =
  (make: ('a, 'b, 'c) => 'r, nameA: string, getA: ('r) => 'a, a: t<'a>,
   nameB: string, getB: ('r) => 'b, b: t<'b>, nameC: string, getC: ('r) => 'c, c: t<'c>): t<'r> =>
  { reads:
      JsonDecode.map3(make, JsonDecode.field(nameA, a.reads), JsonDecode.field(nameB, b.reads),
        JsonDecode.field(nameC, c.reads)),
    writes: (r: 'r) =>
      JsonEncode.object([(nameA, a.writes(getA(r))), (nameB, b.writes(getB(r))), (nameC, c.writes(getC(r)))]) }

let object4 =
  (make: ('a, 'b, 'c, 'd) => 'r, nameA: string, getA: ('r) => 'a, a: t<'a>,
   nameB: string, getB: ('r) => 'b, b: t<'b>, nameC: string, getC: ('r) => 'c, c: t<'c>,
   nameD: string, getD: ('r) => 'd, d: t<'d>): t<'r> =>
  { reads:
      JsonDecode.map4(make, JsonDecode.field(nameA, a.reads), JsonDecode.field(nameB, b.reads),
        JsonDecode.field(nameC, c.reads), JsonDecode.field(nameD, d.reads)),
    writes: (r: 'r) =>
      JsonEncode.object([(nameA, a.writes(getA(r))), (nameB, b.writes(getB(r))),
        (nameC, c.writes(getC(r))), (nameD, d.writes(getD(r)))]) }

// Read a value from JSON text.
let decode = (c: t<'a>, text: string): Result.t<'a, Json.error> => JsonDecode.parse(c.reads, text)

// Write a value as JSON text.
let encode = (c: t<'a>, value: 'a): string => JsonEncode.run(c.writes(value))
