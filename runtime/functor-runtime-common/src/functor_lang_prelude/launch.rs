//! `Launch.*` — the `--arg key=value` pairs the shell was started with (see
//! `launch.funi`). Process-global, like the project root: one process runs
//! one game.

use std::collections::BTreeMap;
use std::sync::Mutex;

use super::*;

static ARGS: Mutex<BTreeMap<String, String>> = Mutex::new(BTreeMap::new());

/// One `--arg` value: `key=value`, split at the first `=`. The key can't be
/// empty; the value can.
pub fn parse_arg(arg: &str) -> Result<(String, String), String> {
    match arg.split_once('=') {
        Some((key, value)) if !key.is_empty() => Ok((key.to_string(), value.to_string())),
        _ => Err(format!("expected key=value, got {arg:?}")),
    }
}

/// Declare the launch arguments; the shell calls this before the game loads.
/// A repeated key keeps its last value.
pub fn set_args(args: impl IntoIterator<Item = (String, String)>) {
    *ARGS.lock().unwrap_or_else(|e| e.into_inner()) = args.into_iter().collect();
}

pub(super) fn register(reg: &mut crate::host_registry::Registry) {
    reg.fn0("Launch.args", "Launch.args()", || {
        // A BTreeMap iterates in `str` order, the map's canonical key order.
        let entries = ARGS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(key, value)| {
                (functor_lang::value::MapKey::String(key.as_str().into()), Value::String(value.as_str().into()))
            })
            .collect();
        Value::Map(functor_lang::pmap::PMap::from_sorted(entries).0)
    });
}

#[cfg(test)]
mod tests {
    use super::super::tests::eval_with_prelude;
    use super::*;

    #[test]
    fn args_split_at_the_first_equals() {
        assert_eq!(parse_arg("server=http://h:1/?a=b"), Ok(("server".into(), "http://h:1/?a=b".into())));
        assert_eq!(parse_arg("flag="), Ok(("flag".into(), String::new())));
        assert!(parse_arg("flag").is_err() && parse_arg("=x").is_err());
    }

    // One test: the arguments are process-global.
    #[test]
    fn args_read_back_as_a_map() {
        let eval = |main: &str| eval_with_prelude(&format!("let main = {main}\n")).map(|v| v.to_string());
        assert_eq!(eval("Map.size(Launch.args()) == 0.0").as_deref(), Ok("true"));
        set_args(["scan=1", "server=http://h", "scan=latest"].map(|arg| parse_arg(arg).unwrap()));
        let checks = [
            r#"(Launch.args() |> Map.get("server")) == Option.Some("http://h")"#,
            r#"(Launch.args() |> Map.get("scan")) == Option.Some("latest")"#,
            r#"(Launch.args() |> Map.get("missing")) == Option.None"#,
            r#"Launch.args() == Map.fromList([("server", "http://h"), ("scan", "latest")])"#,
        ];
        for check in checks {
            assert_eq!(eval(check).as_deref(), Ok("true"), "{check}");
        }
        set_args([]);
    }
}
