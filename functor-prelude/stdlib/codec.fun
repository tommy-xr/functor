// Codec — a decoder and an encoder for the same value, defined once: read a
// format with `decode`, write it with `encode`, and test that they agree
// (`expect Codec.decode(c, Codec.encode(c, x)) == Result.Ok(x)`). Suits
// fixed-layout records; formats with offset tables decode with `Decode`.
//
//   type point = { x: float, y: float }
//   let point = Codec.record2((x, y): point => { x: x, y: y },
//     (p: point) => p.x, Codec.f32le, (p: point) => p.y, Codec.f32le)

type t<'a> = { decoder: Decode.t<'a>, encoder: ('a) => Encode.t }

let u8 = { decoder: Decode.u8, encoder: Encode.u8 }
let i8 = { decoder: Decode.i8, encoder: Encode.i8 }
let u16le = { decoder: Decode.u16le, encoder: Encode.u16le }
let u16be = { decoder: Decode.u16be, encoder: Encode.u16be }
let i16le = { decoder: Decode.i16le, encoder: Encode.i16le }
let i16be = { decoder: Decode.i16be, encoder: Encode.i16be }
let u32le = { decoder: Decode.u32le, encoder: Encode.u32le }
let u32be = { decoder: Decode.u32be, encoder: Encode.u32be }
let i32le = { decoder: Decode.i32le, encoder: Encode.i32le }
let i32be = { decoder: Decode.i32be, encoder: Encode.i32be }
let f32le = { decoder: Decode.f32le, encoder: Encode.f32le }
let f32be = { decoder: Decode.f32be, encoder: Encode.f32be }
let f64le = { decoder: Decode.f64le, encoder: Encode.f64le }
let f64be = { decoder: Decode.f64be, encoder: Encode.f64be }

// Latin-1 text of exactly `length` characters.
let latin1 = (length: float): t<string> =>
  { decoder: Decode.latin1(length), encoder: (s: string) => Encode.latin1(s) }

// A value stored as another: `to` after decoding, `from` before encoding.
let map = (to: ('a) => 'b, from: ('b) => 'a, c: t<'a>): t<'b> =>
  { decoder: Decode.map(to, c.decoder), encoder: (b: 'b) => c.encoder(from(b)) }

// `count` values in a row.
let list = (count: float, c: t<'a>): t<List<'a>> =>
  { decoder: Decode.list(count, c.decoder), encoder: (items: List<'a>) => Encode.list(c.encoder, items) }

// A record of two fields, in order: how to build it, and each field's getter
// and codec.
let record2 =
  (make: ('a, 'b) => 'r, getA: ('r) => 'a, a: t<'a>, getB: ('r) => 'b, b: t<'b>): t<'r> =>
  { decoder: Decode.map2(make, a.decoder, b.decoder),
    encoder: (r: 'r) => Encode.sequence([a.encoder(getA(r)), b.encoder(getB(r))]) }

let record3 =
  (make: ('a, 'b, 'c) => 'r, getA: ('r) => 'a, a: t<'a>, getB: ('r) => 'b, b: t<'b>,
   getC: ('r) => 'c, c: t<'c>): t<'r> =>
  { decoder: Decode.map3(make, a.decoder, b.decoder, c.decoder),
    encoder: (r: 'r) =>
      Encode.sequence([a.encoder(getA(r)), b.encoder(getB(r)), c.encoder(getC(r))]) }

let record4 =
  (make: ('a, 'b, 'c, 'd) => 'r, getA: ('r) => 'a, a: t<'a>, getB: ('r) => 'b, b: t<'b>,
   getC: ('r) => 'c, c: t<'c>, getD: ('r) => 'd, d: t<'d>): t<'r> =>
  { decoder: Decode.map4(make, a.decoder, b.decoder, c.decoder, d.decoder),
    encoder: (r: 'r) =>
      Encode.sequence([a.encoder(getA(r)), b.encoder(getB(r)), c.encoder(getC(r)), d.encoder(getD(r))]) }

// Read a value from the start of `data`.
let decode = (c: t<'a>, data: Bytes.t): Result.t<'a, Binary.error> => Decode.run(c.decoder, data)

// Write a value.
let encode = (c: t<'a>, value: 'a): Bytes.t => Encode.run(c.encoder(value))
