//! `HostReturn::Apply`: a host external hands a function back for the
//! interpreter to call in the caller's context, then finishes with its result.

use functor_lang::{Host, HostApply, HostReturn, RunError, RunOutcome, Span, Tracing, Value};

/// `T.applyThen(f, x)` = `f(x) * 10`, refusing a negative `f(x)`.
struct ApplyHost;

impl Host for ApplyHost {
    fn provides(&self, path: &str) -> bool {
        path == "T.applyThen"
    }

    fn call(&mut self, _path: &str, args: Vec<Value>, _span: Span) -> Result<HostReturn, RunError> {
        let mut args = args.into_iter();
        let (callee, x) = (args.next().unwrap(), args.next().unwrap());
        Ok(HostReturn::Apply(Box::new(HostApply {
            callee,
            args: vec![x],
            via: "T.applyThen",
            then: Box::new(|value| match value {
                Value::Number(n) if n >= 0.0 => Ok(Value::Number(n * 10.0)),
                other => Err(format!("T.applyThen: refused {other}")),
            }),
        })))
    }
}

fn run_err(src: &str) -> RunError {
    match run(src) {
        Err(err) => err,
        Ok(value) => panic!("expected an error, got {value}"),
    }
}

fn run(src: &str) -> Result<Value, RunError> {
    let module = functor_lang::lower(functor_lang::parse(src).expect("parse")).expect("lower");
    match functor_lang::run_with_host(&module, Tracing::Off, &mut ApplyHost) {
        Ok(record) => match record.outcome {
            RunOutcome::Main(value) => Ok(value),
            _ => panic!("expected a main result"),
        },
        Err(failure) => Err(failure.error),
    }
}

#[test]
fn the_applied_result_flows_through_then() {
    let value = run("let main = () => T.applyThen((n) => n + 1.0, 2.0)").unwrap();
    assert_eq!(value.to_string(), "30");
}

#[test]
fn a_then_error_carries_the_host_call_span() {
    let src = "let main = () => T.applyThen((n) => 0.0 - n, 2.0)";
    let err = run_err(src);
    assert_eq!(err.message, "T.applyThen: refused -2");
    assert_eq!(
        &src[err.span.start..err.span.end],
        "T.applyThen((n) => 0.0 - n, 2.0)"
    );
}
