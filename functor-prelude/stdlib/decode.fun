// Decode — binary decoders as values. A decoder reads a value at a byte
// offset; compose small ones (numbers, text, counted lists) into a whole
// format and `run` it on `Bytes.t`. A failure is a `Binary.error` with the
// byte offset and the path of `field` names and list indices that led there.
//
//   let vec3 = Decode.map3(Vec3.make, Decode.f32le, Decode.f32le, Decode.f32le)
//   let points = Decode.u32le |> Decode.andThen((n) => Decode.list(n, vec3))
//   let result = Decode.run(points, File.bytes("points.bin"))

// A decoded value and the offset just past it.
type step<'a> = { value: 'a, next: float }

// Reads at an offset in the bytes, or fails.
type t<'a> = { read: (Bytes.t, float) => Result.t<step<'a>, Binary.error> }

let failure = (offset: float, message: string): Binary.error =>
  { offset: offset, path: [], message: message }

// Always `value`, reading nothing.
let succeed = (value: 'a): t<'a> =>
  { read: (data, offset) => Result.Ok({ value: value, next: offset }) }

// Always fails with `message`.
let fail = (message: string): t<'a> =>
  { read: (data, offset) => Result.Error(failure(offset, message)) }

// A fixed-size number read by a `Bytes` reader.
let number = (size: float, reader: (float, Bytes.t) => Result.t<float, Binary.error>): t<float> =>
  { read: (data, offset) =>
      reader(offset, data) |> Result.map((v) => { value: v, next: offset + size }) }

let u8 = number(1.0, Bytes.u8)
let i8 = number(1.0, Bytes.i8)
let u16le = number(2.0, Bytes.u16le)
let u16be = number(2.0, Bytes.u16be)
let i16le = number(2.0, Bytes.i16le)
let i16be = number(2.0, Bytes.i16be)
let u32le = number(4.0, Bytes.u32le)
let u32be = number(4.0, Bytes.u32be)
let i32le = number(4.0, Bytes.i32le)
let i32be = number(4.0, Bytes.i32be)
let f32le = number(4.0, Bytes.f32le)
let f32be = number(4.0, Bytes.f32be)
let f64le = number(8.0, Bytes.f64le)
let f64be = number(8.0, Bytes.f64be)

// `length` bytes as their own `Bytes.t` (offsets inside it start at 0).
let bytes = (length: float): t<Bytes.t> =>
  { read: (data, offset) =>
      Bytes.slice(offset, length, data) |> Result.map((s) => { value: s, next: offset + length }) }

// `length` bytes as Latin-1 text (one character per byte).
let latin1 = (length: float): t<string> =>
  { read: (data, offset) =>
      Bytes.latin1(offset, length, data) |> Result.map((s) => { value: s, next: offset + length }) }

// `length` bytes as UTF-8 text.
let utf8 = (length: float): t<string> =>
  { read: (data, offset) =>
      Bytes.utf8(offset, length, data) |> Result.map((s) => { value: s, next: offset + length }) }

// The decoded value, transformed.
let map = (f: ('a) => 'b, d: t<'a>): t<'b> =>
  { read: (data, offset) =>
      d.read(data, offset) |> Result.map((s) => { value: f(s.value), next: s.next }) }

// Decode with `d`, then with the decoder `f` chooses from its value — for
// counts and versions that shape what follows.
let andThen = (f: ('a) => t<'b>, d: t<'a>): t<'b> =>
  { read: (data, offset) =>
      d.read(data, offset)
        |> Result.bind((s) =>
          let next = f(s.value) in
          next.read(data, s.next)) }

// Two decoders in a row, combined.
let map2 = (f: ('a, 'b) => 'c, a: t<'a>, b: t<'b>): t<'c> =>
  a |> andThen((x) => b |> map((y) => f(x, y)))

let map3 = (f: ('a, 'b, 'c) => 'd, a: t<'a>, b: t<'b>, c: t<'c>): t<'d> =>
  a |> andThen((x) => map2((y, z) => f(x, y, z), b, c))

let map4 = (f: ('a, 'b, 'c, 'd) => 'e, a: t<'a>, b: t<'b>, c: t<'c>, d: t<'d>): t<'e> =>
  a |> andThen((x) => map3((y, z, w) => f(x, y, z, w), b, c, d))

let map5 =
  (f: ('a, 'b, 'c, 'd, 'e) => 'f, a: t<'a>, b: t<'b>, c: t<'c>, d: t<'d>, e: t<'e>): t<'f> =>
  a |> andThen((x) => map4((y, z, w, v) => f(x, y, z, w, v), b, c, d, e))

// Name failures inside `d`: `header.count at byte 12: needs 4 bytes, 1 left`.
let field = (name: string, d: t<'a>): t<'a> =>
  { read: (data, offset) =>
      d.read(data, offset)
        |> Result.mapError((e: Binary.error) => { e with path: List.append(e.path, [Binary.Field(name)]) }) }

// `count` values in a row (up to a million); a failure names its index.
let list = (count: float, d: t<'a>): t<List<'a>> =>
  { read: (data, offset) =>
      if count < 0.0 || count > 1000000.0 || count != Math.floor(count)
      then Result.Error(failure(offset, "a list count must be a whole number from 0 to 1000000"))
      else
        List.range(count)
          |> List.tryMapAccum((here, i) =>
            d.read(data, here)
              |> Result.map((s) => (s.next, s.value))
              |> Result.mapError((e: Binary.error) =>
                { e with path: List.append(e.path, [Binary.Index(i)]) }), offset)
          |> Result.map((done) =>
            match done with
            | (next, values) => { value: values, next: next }) }

// `d` read at the absolute `position`; reading then continues where it was —
// for formats whose header lists where each section starts.
let at = (position: float, d: t<'a>): t<'a> =>
  { read: (data, offset) =>
      d.read(data, position) |> Result.map((s) => { value: s.value, next: offset }) }

// Skip `count` bytes, then read `d`.
let skipThen = (count: float, d: t<'a>): t<'a> =>
  { read: (data, offset) => d.read(data, offset + count) }

// Decode `data` from its first byte.
let run = (d: t<'a>, data: Bytes.t): Result.t<'a, Binary.error> =>
  d.read(data, 0.0) |> Result.map((s) => s.value)
