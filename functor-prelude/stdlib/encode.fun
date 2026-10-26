// Encode — binary output as values: the mirror of `Decode`. An encoder holds
// byte chunks; compose them in order and `run` joins them once.
//
//   let header = Encode.sequence([Encode.latin1("PTS!"), Encode.u32le(count)])
//   let data = Encode.run(Encode.sequence([header, Encode.f32les(coordinates)]))

type t = { chunks: List<Bytes.t> }

// Bytes as they are.
let bytes = (data: Bytes.t): t => { chunks: [data] }

// Numbers (whole and in range for integers; an error otherwise).
let u8 = (n: float): t => { chunks: [Bytes.fromU8([n])] }
let i8 = (n: float): t => { chunks: [Bytes.fromI8([n])] }
let u16le = (n: float): t => { chunks: [Bytes.fromU16le([n])] }
let u16be = (n: float): t => { chunks: [Bytes.fromU16be([n])] }
let i16le = (n: float): t => { chunks: [Bytes.fromI16le([n])] }
let i16be = (n: float): t => { chunks: [Bytes.fromI16be([n])] }
let u32le = (n: float): t => { chunks: [Bytes.fromU32le([n])] }
let u32be = (n: float): t => { chunks: [Bytes.fromU32be([n])] }
let i32le = (n: float): t => { chunks: [Bytes.fromI32le([n])] }
let i32be = (n: float): t => { chunks: [Bytes.fromI32be([n])] }
let f32le = (n: float): t => { chunks: [Bytes.fromF32le([n])] }
let f32be = (n: float): t => { chunks: [Bytes.fromF32be([n])] }
let f64le = (n: float): t => { chunks: [Bytes.fromF64le([n])] }
let f64be = (n: float): t => { chunks: [Bytes.fromF64be([n])] }

// Many numbers at once (fast for big arrays, e.g. vertex positions).
let u16les = (ns: List<float>): t => { chunks: [Bytes.fromU16le(ns)] }
let u32les = (ns: List<float>): t => { chunks: [Bytes.fromU32le(ns)] }
let f32les = (ns: List<float>): t => { chunks: [Bytes.fromF32le(ns)] }

// Text.
let latin1 = (text: string): t => { chunks: [Bytes.fromLatin1(text)] }
let utf8 = (text: string): t => { chunks: [Bytes.fromUtf8(text)] }

// `count` zero bytes (padding).
let zeros = (count: float): t => { chunks: [Bytes.fromU8(List.range(count) |> List.map((i) => 0.0))] }

// Encoders in order.
let sequence = (parts: List<t>): t => { chunks: List.concatMap((p: t) => p.chunks, parts) }

// Each item encoded in order.
let list = (encode: ('a) => t, items: List<'a>): t => sequence(List.map(encode, items))

// All of it as one `Bytes.t`.
let run = (e: t): Bytes.t => Bytes.concat(e.chunks)
