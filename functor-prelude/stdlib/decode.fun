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

// Helpers for writing decoders: an error at `offset`, and an error with
// `segment` added in front of its path.
let failure = (offset: float, message: string): Binary.error =>
  { offset: offset, path: [], message: message }

// Always `value`, reading nothing.
let succeed = (value: 'a): t<'a> =>
  { read: (data, offset) => Result.Ok({ value: value, next: offset }) }

// Always fails with `message`.
let fail = (message: string): t<'a> =>
  { read: (data, offset) => Result.Error(failure(offset, message)) }

// `size` bytes read by `reader(offset, data)`.
let sized = (size: float, reader: (float, Bytes.t) => Result.t<'a, Binary.error>): t<'a> =>
  { read: (data, offset) =>
      reader(offset, data) |> Result.map((v) => { value: v, next: offset + size }) }

let within = (segment: Binary.segment, e: Binary.error): Binary.error =>
  { e with path: List.append(e.path, [segment]) }

// Numbers: unsigned/signed integers and floats, little- or big-endian.
let u8 = sized(1.0, (offset, data) => Bytes.u8(offset, data))
let i8 = sized(1.0, (offset, data) => Bytes.i8(offset, data))
let u16le = sized(2.0, (offset, data) => Bytes.u16le(offset, data))
let u16be = sized(2.0, (offset, data) => Bytes.u16be(offset, data))
let i16le = sized(2.0, (offset, data) => Bytes.i16le(offset, data))
let i16be = sized(2.0, (offset, data) => Bytes.i16be(offset, data))
let u32le = sized(4.0, (offset, data) => Bytes.u32le(offset, data))
let u32be = sized(4.0, (offset, data) => Bytes.u32be(offset, data))
let i32le = sized(4.0, (offset, data) => Bytes.i32le(offset, data))
let i32be = sized(4.0, (offset, data) => Bytes.i32be(offset, data))
let f32le = sized(4.0, (offset, data) => Bytes.f32le(offset, data))
let f32be = sized(4.0, (offset, data) => Bytes.f32be(offset, data))
let f64le = sized(8.0, (offset, data) => Bytes.f64le(offset, data))
let f64be = sized(8.0, (offset, data) => Bytes.f64be(offset, data))

// `length` bytes as their own `Bytes.t` (offsets inside it start at 0).
let bytes = (length: float): t<Bytes.t> =>
  sized(length, (offset, data) => Bytes.slice(offset, length, data))

// `length` bytes as Latin-1 text (one character per byte).
let latin1 = (length: float): t<string> =>
  sized(length, (offset, data) => Bytes.latin1(offset, length, data))

// `length` bytes as UTF-8 text.
let utf8 = (length: float): t<string> =>
  sized(length, (offset, data) => Bytes.utf8(offset, length, data))

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

// Two decoders in a row, combined. Chains of `map2`..`map5`/`andThen` nest as
// they read (about 50 links fit); use `sequence` for longer runs.
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
        |> Result.mapError((e) => within(Binary.Field(name), e)) }

// `count` values in a row (up to a million); a failure names its index.
let list = (count: float, d: t<'a>): t<List<'a>> =>
  { read: (data, offset) =>
      if count < 0.0 || count > 1000000.0 || count != Math.floor(count)
      then Result.Error(failure(offset, "a list count must be a whole number from 0 to 1000000"))
      else
        List.range(count)
          |> List.tryMapAccum((here, i) =>
            match d.read(data, here) with
            | Result.Ok(s) => Result.Ok((s.next, s.value))
            | Result.Error(e) => Result.Error(within(Binary.Index(i), e)), offset)
          |> Result.map((done) =>
            match done with
            | (next, values) => { value: values, next: next }) }

// The decoders in a row, as a list — flat at any length.
let sequence = (decoders: List<t<'a>>): t<List<'a>> =>
  { read: (data, offset) =>
      decoders
        |> List.tryMapAccum((here: float, d: t<'a>) =>
          match d.read(data, here) with
          | Result.Ok(s) => Result.Ok((s.next, s.value))
          | Result.Error(e) => Result.Error(e), offset)
        |> Result.map((done) =>
          match done with
          | (next, values) => { value: values, next: next }) }

// `d` read at the absolute `position`; reading then continues where it was —
// for formats whose header lists where each section starts.
let at = (position: float, d: t<'a>): t<'a> =>
  { read: (data, offset) =>
      Bytes.slice(position, 0.0, data)
        |> Result.bind((_) => d.read(data, position))
        |> Result.map((s) => { value: s.value, next: offset }) }

// Skip `count` bytes (which must be there), then read `d`.
let skipThen = (count: float, d: t<'a>): t<'a> =>
  { read: (data, offset) =>
      Bytes.slice(offset, count, data) |> Result.bind((_) => d.read(data, offset + count)) }

// Decode `data` from its first byte.
let run = (d: t<'a>, data: Bytes.t): Result.t<'a, Binary.error> =>
  d.read(data, 0.0) |> Result.map((s) => s.value)
