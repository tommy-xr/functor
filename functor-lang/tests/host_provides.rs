//! An external is re-checked with the host each time it's evaluated, so a
//! host that stops providing a function is obeyed (the interpreter caches
//! resolved externals per call site).

use functor_lang::{Host, HostReturn, RunError, Span, Tracing, Value};

/// Provides `T.once` until it's been called once.
#[derive(Default)]
struct OnceHost {
    called: bool,
}

impl Host for OnceHost {
    fn provides(&self, path: &str) -> bool {
        path == "T.once" && !self.called
    }

    fn call(&mut self, _path: &str, args: Vec<Value>, _span: Span) -> Result<HostReturn, RunError> {
        self.called = true;
        Ok(HostReturn::Value(args.into_iter().next().unwrap()))
    }
}

#[test]
fn a_withdrawn_host_function_is_an_unknown_external() {
    let src = "let main = () => List.map((i) => T.once(i), [1.0, 2.0])";
    let module = functor_lang::lower(functor_lang::parse(src).expect("parse")).expect("lower");
    let failure = functor_lang::run_with_host(&module, Tracing::Off, &mut OnceHost::default())
        .err()
        .expect("the second call should fail");
    assert!(failure.error.message.contains("T.once"), "{}", failure.error.message);
}
