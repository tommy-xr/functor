//! `Json.*` — JSON text to and from the `Json.t` variant (see `json.funi`).
//! The decoders and encoders over it are stdlib (`json_decode.fun`, …).

use functor_lang::value::MapKey;

use super::*;

/// The arrays and objects serde_json lets nest (it rejects the 128th),
/// applied when writing too, so what is written parses back.
const MAX_DEPTH: usize = 127;

fn variant(ctor: &str, args: Vec<Value>) -> Value {
    Value::Variant {
        ctor: ctor.into(),
        args: Rc::new(args),
    }
}

/// serde_json caps nesting, so this recursion is bounded.
fn to_value(json: serde_json::Value) -> Value {
    use serde_json::Value as J;
    match json {
        J::Null => variant("Json.Null", vec![]),
        J::Bool(b) => variant("Json.Bool", vec![Value::Bool(b)]),
        // Integers beyond 2^53 round to the nearest float.
        J::Number(n) => variant("Json.Number", vec![Value::Number(n.as_f64().unwrap_or(f64::NAN))]),
        J::String(s) => variant("Json.String", vec![Value::String(s.into())]),
        J::Array(items) => {
            let items = items.into_iter().map(to_value).collect();
            variant("Json.Array", vec![Value::List(Rc::new(items))])
        }
        J::Object(fields) => {
            let fields = fields
                .into_iter()
                .map(|(name, value)| (MapKey::String(name.into()), to_value(value)))
                .collect();
            // The map's canonical key order, whatever order serde_json keeps.
            let (fields, _) = functor_lang::value::canonicalize_map_entries(fields);
            variant("Json.Object", vec![Value::Map(functor_lang::pmap::PMap::from_sorted(fields).0)])
        }
    }
}

fn from_value(json: &Value, depth: usize) -> Result<serde_json::Value, String> {
    use serde_json::Value as J;
    let Value::Variant { ctor, args } = json else {
        return Err(format!("Json.stringify: expected a Json.t, got {}", json.kind_name()));
    };
    // `depth` containers hold this value.
    if depth >= MAX_DEPTH && matches!(&**ctor, "Json.Array" | "Json.Object") {
        return Err(format!("Json.stringify: nested deeper than {MAX_DEPTH}"));
    }
    Ok(match (&**ctor, args.as_slice()) {
        ("Json.Null", []) => J::Null,
        ("Json.Bool", [Value::Bool(b)]) => J::Bool(*b),
        ("Json.Number", [Value::Number(n)]) => {
            // A whole number prints as `12`, not `12.0`.
            if n.fract() == 0.0 && n.abs() < 9_007_199_254_740_992.0 {
                J::from(*n as i64)
            } else {
                serde_json::Number::from_f64(*n)
                    .map(J::Number)
                    .ok_or_else(|| format!("Json.stringify: {n} isn't a JSON number"))?
            }
        }
        ("Json.String", [Value::String(s)]) => J::String(s.to_string()),
        ("Json.Array", [Value::List(items)]) => {
            J::Array(items.iter().map(|item| from_value(item, depth + 1)).collect::<Result<_, _>>()?)
        }
        ("Json.Object", [Value::Map(fields)]) => {
            let mut object = serde_json::Map::new();
            for (key, value) in fields.iter() {
                let MapKey::String(key) = key else {
                    return Err("Json.stringify: an object's keys are strings".to_string());
                };
                object.insert(key.to_string(), from_value(value, depth + 1)?);
            }
            J::Object(object)
        }
        _ => return Err(format!("Json.stringify: expected a Json.t, got {ctor}")),
    })
}

pub(super) fn register(reg: &mut crate::host_registry::Registry) {
    reg.fn1("Json.parse", "Json.parse(text)", |text: String| {
        // Files often start with a byte-order mark, which isn't JSON.
        match serde_json::from_str(text.strip_prefix('\u{FEFF}').unwrap_or(&text)) {
            Ok(json) => bytes::ok(to_value(json)),
            Err(e) => variant("Result.Error", vec![Value::String(e.to_string().into())]),
        }
    });
    reg.fn1("Json.stringify", "Json.stringify(json)", |json: Value| {
        from_value(&json, 0).map(|json| Value::String(json.to_string().into()))
    });
}

#[cfg(test)]
mod tests {
    use super::super::tests::eval_with_prelude;
    use super::*;

    fn check_all(prelude: &str, checks: &[&str]) {
        for check in checks {
            let src = format!("{prelude}let main = {check}\n");
            assert!(
                matches!(eval_with_prelude(&src), Ok(Value::Bool(true))),
                "{check}: {:?}",
                eval_with_prelude(&src).map(|v| v.to_string())
            );
        }
    }

    #[test]
    fn parses_and_writes_text() {
        check_all(
            "",
            &[
                r#"Json.parse("null") == Result.Ok(Json.Null)"#,
                r#"Json.parse(" [true, -1.5e1, \"a\\n\"] ") == Result.Ok(Json.Array([Json.Bool(true), Json.Number(-15.0), Json.String("a\n")]))"#,
                r#"Json.parse("{\"b\": 1, \"a\": {\"c\": 2}, \"b\": 3}") == Result.Ok(Json.Object(Map.fromList([("a", Json.Object(Map.fromList([("c", Json.Number(2.0))]))), ("b", Json.Number(3.0))])))"#,
                "Json.parse(\"\u{FEFF}1\") == Result.Ok(Json.Number(1.0))",
                r#"match Json.parse("{\"a\": 1,}") with | Result.Error(why) => Text.contains("line 1", why) | Result.Ok(_) => false"#,
                r#"Result.isError(Json.parse("")) && Result.isError(Json.parse("1 2"))"#,
                r#"Json.stringify(Json.Object(Map.fromList([("b", Json.Array([Json.Number(1.0), Json.Number(0.5), Json.Null])), ("a", Json.String("x\"y"))]))) == "{\"a\":\"x\\\"y\",\"b\":[1,0.5,null]}""#,
                r#"(Json.parse("{\"a\":[1,2.5,true,null,\"s\"]}") |> Result.map(Json.stringify)) == Result.Ok("{\"a\":[1,2.5,true,null,\"s\"]}")"#,
            ],
        );
        let failure = |main: &str| eval_with_prelude(&format!("let main = {main}\n")).err().unwrap_or_default();
        assert!(failure("Json.stringify(Json.Number(0.0 / 0.0))").contains("isn't a JSON number"));
        // What nests as deep as a parse allows writes and reads back; deeper is an error.
        let wrapped = |depth: usize| {
            format!("Json.stringify(List.range({depth}.0) |> List.fold((json, i) => Json.Array([json]), Json.Null))")
        };
        check_all("", &[&format!("Result.isOk(Json.parse({}))", wrapped(127))]);
        assert!(failure(&wrapped(128)).contains("nested deeper"));
    }

    // Deep nesting is a parse error, not a stack overflow.
    #[test]
    fn deep_nesting_is_an_error() {
        let nested = |depth: usize| {
            let text = "[".repeat(depth) + &"]".repeat(depth);
            eval_with_prelude(&format!("let main = Result.isError(Json.parse(\"{text}\"))\n"))
        };
        assert!(matches!(nested(100), Ok(Value::Bool(false))));
        assert!(matches!(nested(10_000), Ok(Value::Bool(true))));
    }

    /// `JsonDecode`, on the scanner's status: nested objects, lists, error
    /// paths, an `andThen`-driven shape, and long lists without recursion.
    #[test]
    fn json_decoders_compose_and_locate_failures() {
        let prelude = r#"type box = { lo: List<float>, hi: List<float> }
type pose = { x: float, y: float, z: float }
type status = { stream: string, frames: float, box: Option.t<box>, poses: List<pose> }
let numbers = JsonDecode.list(JsonDecode.number)
let box = JsonDecode.map2((lo, hi): box => { lo: lo, hi: hi },
  JsonDecode.field("min", numbers), JsonDecode.field("max", numbers))
let pose = JsonDecode.map3((x, y, z): pose => { x: x, y: y, z: z },
  JsonDecode.index(0.0, JsonDecode.number), JsonDecode.index(1.0, JsonDecode.number), JsonDecode.index(2.0, JsonDecode.number))
let status = JsonDecode.map4((stream, frames, box, poses): status => { stream: stream, frames: frames, box: box, poses: poses },
  JsonDecode.field("stream", JsonDecode.string),
  JsonDecode.field("frames", JsonDecode.field("depth", JsonDecode.number)),
  JsonDecode.optionalField("box", box),
  JsonDecode.field("poses", JsonDecode.list(pose)))
let text = "{\"stream\":\"bbd6\",\"frames\":{\"depth\":80,\"camera\":24},\"box\":{\"min\":[0,0,0],\"max\":[1,2,3]},\"poses\":[[0,1,2],[3,4,5]]}"
let versioned = JsonDecode.field("version", JsonDecode.number) |> JsonDecode.andThen((v) =>
  if v == 2.0 then JsonDecode.field("name", JsonDecode.string) else JsonDecode.fail("unknown version"))
let why = (d, text: string) => match JsonDecode.parse(d, text) with
  | Result.Error(e) => JsonDecode.describe(e)
  | Result.Ok(_) => "ok"
"#;
        check_all(
            prelude,
            &[
                r#"JsonDecode.parse(status, text) == Result.Ok({ stream: "bbd6", frames: 80.0, box: Option.Some({ lo: [0.0, 0.0, 0.0], hi: [1.0, 2.0, 3.0] }), poses: [{ x: 0.0, y: 1.0, z: 2.0 }, { x: 3.0, y: 4.0, z: 5.0 }] })"#,
                r#"(JsonDecode.parse(status, "{\"stream\":\"s\",\"frames\":{\"depth\":1},\"box\":null,\"poses\":[]}") |> Result.map((s: status) => s.box)) == Result.Ok(Option.None)"#,
                r#"(JsonDecode.parse(status, "{\"stream\":\"s\",\"frames\":{\"depth\":1},\"poses\":[]}") |> Result.map((s: status) => s.box)) == Result.Ok(Option.None)"#,
                r#"why(status, "{\"stream\":\"s\",\"frames\":{\"depth\":1},\"poses\":[[0,1,2],[3,\"4\",5]]}") == "poses[1][1]: expected a number, got a string""#,
                r#"why(status, "{\"stream\":\"s\",\"frames\":{},\"poses\":[]}") == "frames.depth: missing""#,
                r#"why(status, "{\"stream\":\"s\",\"frames\":{\"depth\":1},\"box\":{\"min\":[],\"max\":7},\"poses\":[]}") == "box.max: expected an array, got a number""#,
                r#"why(status, "[]") == "expected an object with \"stream\", got an array""#,
                r#"why(status, "{") == "EOF while parsing an object at line 1 column 1""#,
                r#"why(JsonDecode.index(2.0, JsonDecode.number), "[1, 2]") == "[2]: missing""#,
                r#"why(JsonDecode.index(0.5, JsonDecode.number), "[1]") == "an index must be a whole number from 0" && why(JsonDecode.index(0.0 / 0.0, JsonDecode.number), "[1]") != "ok""#,
                r#"JsonDecode.parse(versioned, "{\"version\":2,\"name\":\"n\"}") == Result.Ok("n") && why(versioned, "{\"version\":3}") == "unknown version""#,
                r#"JsonDecode.parse(JsonDecode.succeed(1.0), "null") == Result.Ok(1.0) && JsonDecode.parse(JsonDecode.bool, "true") == Result.Ok(true)"#,
                r#"JsonDecode.run(JsonDecode.value, Json.Null) == Result.Ok(Json.Null)"#,
                r#"(JsonDecode.run(JsonDecode.list(JsonDecode.number), Json.Array(List.range(100000.0) |> List.map(Json.Number))) |> Result.map(List.length)) == Result.Ok(100000.0)"#,
            ],
        );
    }

    /// A decoder that uses itself (`lazy`) reads a tree, five fields a level.
    #[test]
    fn recursive_decoders_read_trees() {
        let prelude = r#"type tree = { name: string, a: float, b: float, c: float, kids: List<tree> }
let tree = JsonDecode.map5((name, a, b, c, kids): tree => { name: name, a: a, b: b, c: c, kids: kids },
  JsonDecode.field("name", JsonDecode.string), JsonDecode.field("a", JsonDecode.number),
  JsonDecode.field("b", JsonDecode.number), JsonDecode.field("c", JsonDecode.number),
  JsonDecode.field("kids", JsonDecode.list(JsonDecode.lazy(() => tree))))
let depth = (t: tree): float => 1.0 + (t.kids |> List.map(depth) |> List.fold((most, d) => Math.max(most, d), 0.0))
"#;
        // The tree `levels` deep, as a Functor-lang string literal.
        let nested = |levels: usize| {
            let open = r#"{"name":"n","a":1,"b":2,"c":3,"kids":["#.repeat(levels);
            format!("{:?}", format!("{open}{}", "]}".repeat(levels)))
        };
        // Deeper trees are the interpreter's limit, not the decoder's: recursion
        // through a list built-in's callback outruns a 2 MiB test stack first.
        let n = 6;
        check_all(
            prelude,
            &[&format!("(JsonDecode.parse(tree, {}) |> Result.map(depth)) == Result.Ok({n}.0)", nested(n))],
        );
    }

    /// `JsonEncode` writes what `JsonDecode` reads, and a `JsonCodec` round-trips.
    #[test]
    fn json_encoders_and_codecs_round_trip() {
        let prelude = r#"type point = { x: float, y: float }
type shot = { name: string, at: point, tags: List<string>, ok: bool }
let point = JsonCodec.object2((x, y): point => { x: x, y: y },
  "x", (p: point) => p.x, JsonCodec.number, "y", (p: point) => p.y, JsonCodec.number)
let shot = JsonCodec.object4((name, at, tags, ok): shot => { name: name, at: at, tags: tags, ok: ok },
  "name", (s: shot) => s.name, JsonCodec.string, "at", (s: shot) => s.at, point,
  "tags", (s: shot) => s.tags, JsonCodec.list(JsonCodec.string), "ok", (s: shot) => s.ok, JsonCodec.bool)
let one: shot = { name: "a", at: { x: 1.0, y: 2.5 }, tags: ["t", "u"], ok: true }
let percent = JsonCodec.map((n) => n * 100.0, (p) => p / 100.0, JsonCodec.number)
"#;
        check_all(
            prelude,
            &[
                r#"JsonCodec.encode(shot, one) == "{\"at\":{\"x\":1,\"y\":2.5},\"name\":\"a\",\"ok\":true,\"tags\":[\"t\",\"u\"]}""#,
                r#"JsonCodec.decode(shot, JsonCodec.encode(shot, one)) == Result.Ok(one)"#,
                r#"JsonCodec.decode(percent, "0.5") == Result.Ok(50.0) && JsonCodec.encode(percent, 50.0) == "0.5""#,
                r#"JsonEncode.run(JsonEncode.object([("n", JsonEncode.null), ("o", JsonEncode.optional(JsonEncode.number, Option.Some(1.0))), ("l", JsonEncode.list(JsonEncode.bool, [true]))])) == "{\"l\":[true],\"n\":null,\"o\":1}""#,
                r#"match JsonCodec.decode(shot, "{\"name\":\"a\",\"at\":{\"x\":1},\"tags\":[],\"ok\":true}") with | Result.Error(e) => JsonDecode.describe(e) == "at.y: missing" | Result.Ok(_) => false"#,
            ],
        );
    }
}
