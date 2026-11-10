// JsonCodec — a JSON decoder and encoder for the same value, defined once:
// `Codec` for JSON. Read text with `decode`, write it with `encode`, and test
// that they agree (`expect JsonCodec.decode(c, JsonCodec.encode(c, x)) == Result.Ok(x)`).
//
//   type point = { x: float, y: float }
//   let point = JsonCodec.object2((x, y): point => { x: x, y: y },
//     "x", (p: point) => p.x, JsonCodec.number, "y", (p: point) => p.y, JsonCodec.number)

type t<'a> = { decoder: JsonDecode.t<'a>, encoder: ('a) => Json.t }

let number: t<float> = { decoder: JsonDecode.number, encoder: JsonEncode.number }
let string: t<string> = { decoder: JsonDecode.string, encoder: JsonEncode.string }
let bool: t<bool> = { decoder: JsonDecode.bool, encoder: JsonEncode.bool }

// A value stored as another: `to` after decoding, `from` before encoding.
let map = (to: ('a) => 'b, from: ('b) => 'a, c: t<'a>): t<'b> =>
  { decoder: JsonDecode.map(to, c.decoder), encoder: (b: 'b) => c.encoder(from(b)) }

// An array of values.
let list = (c: t<'a>): t<List<'a>> =>
  { decoder: JsonDecode.list(c.decoder), encoder: (items: List<'a>) => JsonEncode.list(c.encoder, items) }

// An object of two fields: how to build the record, and each field's name,
// getter and codec.
let object2 =
  (make: ('a, 'b) => 'r, nameA: string, getA: ('r) => 'a, a: t<'a>,
   nameB: string, getB: ('r) => 'b, b: t<'b>): t<'r> =>
  { decoder: JsonDecode.map2(make, JsonDecode.field(nameA, a.decoder), JsonDecode.field(nameB, b.decoder)),
    encoder: (r: 'r) => JsonEncode.object([(nameA, a.encoder(getA(r))), (nameB, b.encoder(getB(r)))]) }

let object3 =
  (make: ('a, 'b, 'c) => 'r, nameA: string, getA: ('r) => 'a, a: t<'a>,
   nameB: string, getB: ('r) => 'b, b: t<'b>, nameC: string, getC: ('r) => 'c, c: t<'c>): t<'r> =>
  { decoder:
      JsonDecode.map3(make, JsonDecode.field(nameA, a.decoder), JsonDecode.field(nameB, b.decoder),
        JsonDecode.field(nameC, c.decoder)),
    encoder: (r: 'r) =>
      JsonEncode.object([(nameA, a.encoder(getA(r))), (nameB, b.encoder(getB(r))), (nameC, c.encoder(getC(r)))]) }

let object4 =
  (make: ('a, 'b, 'c, 'd) => 'r, nameA: string, getA: ('r) => 'a, a: t<'a>,
   nameB: string, getB: ('r) => 'b, b: t<'b>, nameC: string, getC: ('r) => 'c, c: t<'c>,
   nameD: string, getD: ('r) => 'd, d: t<'d>): t<'r> =>
  { decoder:
      JsonDecode.map4(make, JsonDecode.field(nameA, a.decoder), JsonDecode.field(nameB, b.decoder),
        JsonDecode.field(nameC, c.decoder), JsonDecode.field(nameD, d.decoder)),
    encoder: (r: 'r) =>
      JsonEncode.object([(nameA, a.encoder(getA(r))), (nameB, b.encoder(getB(r))),
        (nameC, c.encoder(getC(r))), (nameD, d.encoder(getD(r)))]) }

// Read a value from JSON text.
let decode = (c: t<'a>, text: string): Result.t<'a, Json.error> => JsonDecode.parse(c.decoder, text)

// Write a value as JSON text.
let encode = (c: t<'a>, value: 'a): string => JsonEncode.run(c.encoder(value))
