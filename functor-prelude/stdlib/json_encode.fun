// JsonEncode — JSON output as values: the mirror of `JsonDecode`. What an
// encoder makes is the `Json.t` to write; build it up and `run` turns it
// into text.
//
//   let text = JsonEncode.run(JsonEncode.object([
//     ("name", JsonEncode.string("scan-3")),
//     ("frames", JsonEncode.list(JsonEncode.number, [1.0, 2.0]))]))

let number = (n: float): Json.t => Json.Number(n)
let string = (text: string): Json.t => Json.String(text)
let bool = (b: bool): Json.t => Json.Bool(b)
let null: Json.t = Json.Null

// Each item encoded, as an array.
let list = (encode: ('a) => Json.t, items: List<'a>): Json.t => Json.Array(List.map(encode, items))

// An object of named values; a repeated name keeps its last value.
let object = (fields: List<(string, Json.t)>): Json.t => Json.Object(Map.fromList(fields))

// `Option.None` as null.
let optional = (encode: ('a) => Json.t, value: Option.t<'a>): Json.t =>
  value |> Option.map(encode) |> Option.defaultValue(Json.Null)

// All of it as compact JSON text.
let run = (json: Json.t): string => Json.stringify(json)
