//! `Json.*` — JSON text parsed into the `Json.t` variant (see `json.funi`).

use super::*;

fn variant(ctor: &str, args: Vec<Value>) -> Value {
    Value::Variant {
        ctor: ctor.into(),
        args: Rc::new(args),
    }
}

/// serde_json caps nesting at 128, so this recursion is bounded too.
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
                .map(|(name, value)| {
                    Value::Tuple(Rc::new(vec![Value::String(name.into()), to_value(value)]))
                })
                .collect();
            variant("Json.Object", vec![Value::List(Rc::new(fields))])
        }
    }
}

/// The payload of `json` if it was built by `ctor`.
fn payload<'a>(ctor: &str, json: &'a Value) -> Option<&'a Value> {
    match json {
        Value::Variant { ctor: c, args } if &**c == ctor => args.first(),
        _ => None,
    }
}

fn option(value: Option<Value>) -> Value {
    match value {
        Some(value) => variant("Option.Some", vec![value]),
        None => variant("Option.None", vec![]),
    }
}

fn field(name: &str, json: &Value) -> Option<Value> {
    let Value::List(fields) = payload("Json.Object", json)? else {
        return None;
    };
    fields.iter().find_map(|entry| match entry {
        Value::Tuple(pair) => match pair.as_slice() {
            [Value::String(key), value] if &**key == name => Some(value.clone()),
            _ => None,
        },
        _ => None,
    })
}

pub(super) fn register(reg: &mut crate::host_registry::Registry) {
    reg.fn1("Json.parse", "Json.parse(text)", |text: String| {
        match serde_json::from_str(&text) {
            Ok(json) => variant("Result.Ok", vec![to_value(json)]),
            Err(e) => variant("Result.Error", vec![Value::String(e.to_string().into())]),
        }
    });
    reg.fn2("Json.field", "Json.field(name, json)", |name: String, json: Value| {
        option(field(&name, &json))
    });
    for (path, sig, ctor) in [
        ("Json.asNumber", "Json.asNumber(json)", "Json.Number"),
        ("Json.asString", "Json.asString(json)", "Json.String"),
        ("Json.asBool", "Json.asBool(json)", "Json.Bool"),
        ("Json.asList", "Json.asList(json)", "Json.Array"),
    ] {
        reg.fn1(path, sig, move |json: Value| option(payload(ctor, &json).cloned()));
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::eval_with_prelude;
    use super::*;

    #[test]
    fn parses_and_reads_fields() {
        let checks = [
            r#"Json.parse("null") == Result.Ok(Json.Null)"#,
            r#"Json.parse(" [true, -1.5e1, \"a\\n\"] ") == Result.Ok(Json.Array([Json.Bool(true), Json.Number(-15.0), Json.String("a\n")]))"#,
            r#"Json.parse("{\"a\": {\"b\": 2}}") == Result.Ok(Json.Object([("a", Json.Object([("b", Json.Number(2.0))]))]))"#,
            r#"(Json.parse("{\"a\": {\"b\": 2}}") |> Result.map((j) => j |> Json.field("a") |> Option.bind((a) => a |> Json.field("b")))) == Result.Ok(Option.Some(Json.Number(2.0)))"#,
            r#"(Json.Object([("n", Json.Number(3.0))]) |> Json.field("n") |> Option.bind(Json.asNumber) |> Option.defaultValue(0.0)) == 3.0"#,
            r#"Json.asNumber(Json.String("3")) == Option.None && Json.asString(Json.String("3")) == Option.Some("3")"#,
            r#"Json.asBool(Json.Bool(false)) == Option.Some(false) && Json.asList(Json.Array([Json.Null])) == Option.Some([Json.Null])"#,
            r#"Json.asList(Json.Null) == Option.None"#,
            r#"(match Json.parse("{\"frames\": 12}") with | Result.Ok(json) => json |> Json.field("frames") |> Option.bind(Json.asNumber) |> Option.defaultValue(0.0) | Result.Error(_) => 0.0) == 12.0"#,
            r#"Json.field("missing", Json.Object([])) == Option.None"#,
            r#"Json.field("a", Json.Array([])) == Option.None"#,
            r#"Json.parse("{\"a\": 1,}") == Result.Error("trailing comma at line 1 column 9")"#,
            r#"Result.isError(Json.parse("")) && Result.isError(Json.parse("1 2"))"#,
        ];
        for check in checks {
            let src = format!("let main = {check}\n");
            assert!(
                matches!(eval_with_prelude(&src), Ok(Value::Bool(true))),
                "{check}: {:?}",
                eval_with_prelude(&src).map(|v| v.to_string())
            );
        }
    }

    // Deep nesting is a parse error, not a stack overflow.
    #[test]
    fn deep_nesting_is_an_error() {
        let deep = "[".repeat(10_000);
        let src = format!("let main = Result.isError(Json.parse(\"{deep}\"))\n");
        assert!(matches!(eval_with_prelude(&src), Ok(Value::Bool(true))));
    }
}
