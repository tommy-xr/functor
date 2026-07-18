//! The typed external registry — the generalized Functor Lang ↔ Rust bridge.
//!
//! The prelude's contract has two halves: the `.funi` interface (types, in
//! `functor-prelude/prelude/*.funi`) and the host implementation. Historically
//! the implementation half was one hand-written `match path` arm per external
//! in [`crate::functor_lang_prelude`], each destructuring its `&[Value]` slice,
//! calling extractors, and hand-writing a usage string — three places to keep
//! in sync per API, guarded only by a drift test.
//!
//! This module replaced that with REGISTRATION: the host declares each
//! external once, as a typed Rust closure —
//!
//! ```ignore
//! reg.fn3("Color.rgb", "Color.rgb(r, g, b)", |r: f64, g: f64, b: f64| {
//!     FunctorLangColor((r as f32, g as f32, b as f32))
//! });
//! ```
//!
//! and the registry derives everything the arm used to hand-roll:
//!
//! - **Arity errors** become `usage: <sig>` from the registered signature, so
//!   the usage text cannot drift from the actual parameter count.
//! - **Argument conversion** comes from [`FromArg`] — implemented once per
//!   Rust-side type, which is where the per-type TEACHING ERRORS live
//!   ("expected a Color, got a bare number — wrap the channels: …"), instead
//!   of being re-plumbed at every consuming arm.
//! - **Returns** convert through [`IntoHostResult`] (a [`HostData`] newtype
//!   wraps itself via [`host_returnable!`]), and a closure may return
//!   `Result<_, String>` for domain-validation errors — the registry
//!   attaches the call span.
//!
//! The registry is now the ONLY dispatch — [`FunctorHost::call`] is a
//! registry lookup (the legacy match is fully retired). The drift test
//! asserts `.funi` signatures ≡ the registered paths, with matching arities.
//!
//! [`FunctorHost::call`]: crate::functor_lang_prelude::FunctorHost
//! [`HostData`]: functor_lang::HostData

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

use functor_lang::value::Value;
use functor_lang::{RunError, Span};

/// The FxHash multiply-xor hasher (rustc's map hasher, inlined to stay
/// dependency-light): registry lookups sit on the per-frame hot path — every
/// host call hashes its path string — and SipHash measurably regressed
/// frame_bench (~+30% us/cell), while Fx restores it. Not DoS-resistant,
/// which is fine: keys are the fixed, host-authored external paths.
#[derive(Default)]
struct FxHasher(u64);

impl Hasher for FxHasher {
    fn write(&mut self, bytes: &[u8]) {
        const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;
        for chunk in bytes.chunks(8) {
            let mut word = [0u8; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            self.0 = (self.0.rotate_left(5) ^ u64::from_le_bytes(word)).wrapping_mul(SEED);
        }
    }
    fn write_u8(&mut self, byte: u8) {
        self.write(&[byte]);
    }
    fn finish(&self) -> u64 {
        self.0
    }
}

type PathMap<V> = HashMap<&'static str, V, BuildHasherDefault<FxHasher>>;

/// Convert one call argument into a typed Rust value. Implementations own the
/// error text for a mismatched argument — including the branded types'
/// teaching errors — so the message is written once per TYPE, not once per
/// consuming external. `path` is the external being called (`"Fog.linear"`),
/// for `{path}: expected …`-shaped messages.
pub trait FromArg: Sized {
    fn from_arg(value: &Value, path: &str, span: Span) -> Result<Self, RunError>;
}

/// A float at the engine boundary: finite (as f32) or a spanned error — the
/// same contract as the legacy `num()` extractor, byte-identical messages.
impl FromArg for f64 {
    fn from_arg(value: &Value, _path: &str, span: Span) -> Result<Self, RunError> {
        match value {
            Value::Number(n) if (*n as f32).is_finite() => Ok(*n),
            Value::Number(n) => Err(RunError {
                message: format!("expected a finite number, got {n}"),
                span,
            }),
            other => Err(RunError {
                message: format!("expected a number, got {}", other.kind_name()),
                span,
            }),
        }
    }
}

impl FromArg for String {
    fn from_arg(value: &Value, _path: &str, span: Span) -> Result<Self, RunError> {
        match value {
            Value::String(s) => Ok(s.to_string()),
            other => Err(RunError {
                message: format!("expected a string, got {}", other.kind_name()),
                span,
            }),
        }
    }
}

/// An `Rc<str>` argument — allocation-neutral for identity-shaped externals
/// (`Physics.tag`) that hand the string straight back.
impl FromArg for std::rc::Rc<str> {
    fn from_arg(value: &Value, _path: &str, span: Span) -> Result<Self, RunError> {
        match value {
            Value::String(s) => Ok(s.clone()),
            other => Err(RunError {
                message: format!("expected a string, got {}", other.kind_name()),
                span,
            }),
        }
    }
}

impl FromArg for bool {
    fn from_arg(value: &Value, _path: &str, span: Span) -> Result<Self, RunError> {
        match value {
            Value::Bool(b) => Ok(*b),
            other => Err(RunError {
                message: format!("expected a bool, got {}", other.kind_name()),
                span,
            }),
        }
    }
}

/// A homogeneous list argument (`Scene.group([scene, …])`): each element
/// converts through `T`'s [`FromArg`] — the leftmost failing element's typed
/// error is reported. A non-list gets "expected a list".
impl<T: FromArg> FromArg for Vec<T> {
    fn from_arg(value: &Value, path: &str, span: Span) -> Result<Self, RunError> {
        match value {
            Value::List(items) => items
                .iter()
                .map(|item| T::from_arg(item, path, span))
                .collect(),
            other => Err(RunError {
                message: format!("expected a list, got {}", other.kind_name()),
                span,
            }),
        }
    }
}

/// The raw value, unconverted — for externals that accept several shapes
/// (taggers, subject-last handles) and dispatch themselves.
impl FromArg for Value {
    fn from_arg(value: &Value, _path: &str, _span: Span) -> Result<Self, RunError> {
        Ok(value.clone())
    }
}

/// What a registered closure may return: a value (a [`HostData`] newtype
/// wraps itself opaquely; [`Value`] passes through for plain data), or
/// `Result<value, String>` for domain validation (`far must be greater than
/// near`) — the registry attaches the call span to the message.
///
/// Blanket impls over [`HostData`] fall foul of coherence (a foreign type
/// could implement it), so host-data newtypes opt in explicitly with
/// [`host_returnable!`] — one line per migrated type.
pub trait IntoHostResult {
    fn into_host_result(self, span: Span) -> Result<Value, RunError>;
}

impl IntoHostResult for Value {
    fn into_host_result(self, _span: Span) -> Result<Value, RunError> {
        Ok(self)
    }
}

impl IntoHostResult for Result<Value, String> {
    fn into_host_result(self, span: Span) -> Result<Value, RunError> {
        self.map_err(|message| RunError { message, span })
    }
}

/// Opt a [`HostData`] newtype into being returned from registered externals,
/// bare or as `Result<T, String>`.
#[macro_export]
macro_rules! host_returnable {
    ($($ty:ty),+ $(,)?) => {$(
        impl $crate::host_registry::IntoHostResult for $ty {
            fn into_host_result(
                self,
                _span: functor_lang::Span,
            ) -> Result<functor_lang::value::Value, functor_lang::RunError> {
                Ok(functor_lang::value::Value::HostData(std::rc::Rc::new(self)))
            }
        }
        impl $crate::host_registry::IntoHostResult for Result<$ty, String> {
            fn into_host_result(
                self,
                span: functor_lang::Span,
            ) -> Result<functor_lang::value::Value, functor_lang::RunError> {
                match self {
                    Ok(value) => {
                        Ok(functor_lang::value::Value::HostData(std::rc::Rc::new(value)))
                    }
                    Err(message) => Err(functor_lang::RunError { message, span }),
                }
            }
        }
    )+};
}

/// One registered external: its dispatch closure plus the human signature its
/// arity error teaches (`usage: Angle.degrees(n)`).
struct External {
    sig: &'static str,
    /// The registered parameter count — asserted against the `.funi`
    /// signature's arity by the drift test, so the usage text and the
    /// interface cannot teach different shapes.
    arity: usize,
    call: Box<dyn Fn(&str, &[Value], Span) -> Result<Value, RunError> + Send + Sync>,
}

/// The registry: external path → typed implementation. Built once at startup
/// ([`crate::functor_lang_prelude::registry`]) — the host's only dispatch.
#[derive(Default)]
pub struct Registry {
    fns: PathMap<External>,
}

/// Implements `fn1`/`fn2`/… — one registration method per arity. Each checks
/// the argument count (else the `usage:` error), converts each argument with
/// [`FromArg`] left to right, and converts the return with [`IntoHostResult`].
///
/// NOTE: when several arguments are malformed at once, the error reported is
/// the LEFTMOST conversion failure — and all conversions run before the
/// closure's own domain validation. Legacy match arms interleaved those
/// checks in ad-hoc orders, so multi-error calls may report a different
/// (equally legitimate) error than the arm they replaced.
macro_rules! register_arity {
    ($method:ident, $count:literal, $($arg:ident : $ty:ident),+) => {
        pub fn $method<$($ty: FromArg + 'static,)+ R: IntoHostResult + 'static>(
            &mut self,
            path: &'static str,
            sig: &'static str,
            f: impl Fn($($ty),+) -> R + Send + Sync + 'static,
        ) {
            self.register(path, sig, $count, move |path, args, span| {
                let [$($arg),+] = args else {
                    return Err(RunError {
                        message: format!("usage: {sig}"),
                        span,
                    });
                };
                f($($ty::from_arg($arg, path, span)?),+).into_host_result(span)
            });
        }
    };
}

impl Registry {
    /// A zero-argument external (`Ui.topLeft()`): any argument is rejected
    /// with the usage error, like the legacy constructor arms.
    pub fn fn0<R: IntoHostResult + 'static>(
        &mut self,
        path: &'static str,
        sig: &'static str,
        f: impl Fn() -> R + Send + Sync + 'static,
    ) {
        self.register(path, sig, 0, move |_path, args, span| {
            if !args.is_empty() {
                return Err(RunError {
                    message: format!("usage: {sig}"),
                    span,
                });
            }
            f().into_host_result(span)
        });
    }

    register_arity!(fn1, 1, a: A);
    register_arity!(fn2, 2, a: A, b: B);
    register_arity!(fn3, 3, a: A, b: B, c: C);
    register_arity!(fn4, 4, a: A, b: B, c: C, d: D);
    register_arity!(fn5, 5, a: A, b: B, c: C, d: D, e: E);
    register_arity!(fn6, 6, a: A, b: B, c: C, d: D, e: E, f_: F);

    fn register(
        &mut self,
        path: &'static str,
        sig: &'static str,
        arity: usize,
        call: impl Fn(&str, &[Value], Span) -> Result<Value, RunError> + Send + Sync + 'static,
    ) {
        assert!(
            sig.starts_with(path),
            "external `{path}`: its usage signature must start with the path, got `{sig}`"
        );
        let previous = self.fns.insert(
            path,
            External {
                sig,
                arity,
                call: Box::new(call),
            },
        );
        assert!(previous.is_none(), "external `{path}` registered twice");
    }

    /// Whether `path` is a registered external.
    pub fn provides(&self, path: &str) -> bool {
        self.fns.contains_key(path)
    }

    /// Dispatch a call if `path` is registered; `None` means an unregistered
    /// path (the caller reports it).
    pub fn call(&self, path: &str, args: &[Value], span: Span) -> Option<Result<Value, RunError>> {
        self.fns.get(path).map(|ext| (ext.call)(path, args, span))
    }

    /// The registered paths, for the `.funi` drift test.
    pub fn paths(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.fns.keys().copied()
    }

    /// `(path, registered arity)` pairs — the drift test asserts each against
    /// the `.funi` signature's parameter count.
    pub fn arities(&self) -> impl Iterator<Item = (&'static str, usize)> + '_ {
        self.fns.iter().map(|(path, ext)| (*path, ext.arity))
    }

    /// The registered signature for `path`, for tests.
    pub fn sig(&self, path: &str) -> Option<&'static str> {
        self.fns.get(path).map(|ext| ext.sig)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use functor_lang::HostData;

    struct Marker(f64);
    impl HostData for Marker {
        fn type_name(&self) -> &'static str {
            "Marker"
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    fn span() -> Span {
        Span { start: 3, end: 9 }
    }

    crate::host_returnable!(Marker);

    fn unwrap_err(result: Result<Value, RunError>) -> RunError {
        match result {
            Err(e) => e,
            Ok(v) => panic!("expected an error, got {v}"),
        }
    }

    fn registry() -> Registry {
        let mut reg = Registry::default();
        reg.fn1("T.wrap", "T.wrap(n)", Marker);
        reg.fn2("T.ratio", "T.ratio(num, denom)", |n: f64, d: f64| {
            if d == 0.0 {
                Err("T.ratio: denom must not be zero".to_string())
            } else {
                Ok(Marker(n / d))
            }
        });
        reg
    }

    #[test]
    fn arity_mismatch_teaches_the_registered_signature() {
        let err = unwrap_err(registry().call("T.wrap", &[], span()).expect("registered"));
        assert_eq!(err.message, "usage: T.wrap(n)");
        assert_eq!(err.span, span());
    }

    #[test]
    fn arguments_convert_left_to_right_with_typed_errors() {
        let err = unwrap_err(
            registry()
                .call(
                    "T.ratio",
                    &[Value::Number(1.0), Value::String("x".into())],
                    span(),
                )
                .expect("registered"),
        );
        assert_eq!(err.message, "expected a number, got a string");
        // Non-finite floats are rejected at the boundary, like the legacy num().
        let err = unwrap_err(
            registry()
                .call("T.wrap", &[Value::Number(f64::INFINITY)], span())
                .expect("registered"),
        );
        assert_eq!(err.message, "expected a finite number, got inf");
    }

    #[test]
    fn domain_validation_errors_carry_the_call_span() {
        let err = unwrap_err(
            registry()
                .call("T.ratio", &[Value::Number(1.0), Value::Number(0.0)], span())
                .expect("registered"),
        );
        assert_eq!(err.message, "T.ratio: denom must not be zero");
        assert_eq!(err.span, span());
    }

    #[test]
    fn success_wraps_host_data_and_unknown_paths_fall_through() {
        let reg = registry();
        let value = reg
            .call("T.ratio", &[Value::Number(9.0), Value::Number(3.0)], span())
            .expect("registered")
            .expect("ok");
        match value {
            Value::HostData(data) => {
                assert_eq!(data.as_any().downcast_ref::<Marker>().unwrap().0, 3.0)
            }
            other => panic!("expected host data, got {other}"),
        }
        assert!(reg.call("T.unknown", &[], span()).is_none());
        assert!(!reg.provides("T.unknown") && reg.provides("T.wrap"));
    }

    #[test]
    #[should_panic(expected = "registered twice")]
    fn double_registration_panics() {
        let mut reg = registry();
        reg.fn1("T.wrap", "T.wrap(n)", Marker);
    }
}
