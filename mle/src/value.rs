//! Runtime values for the B3 interpreter. `Rc`-shared and cheap to clone;
//! MLE data is immutable, so sharing is safe.
//!
//! The `Display` impl is the canonical textual form used by `mle run`/`trace`
//! output (and the committed `.run`/`.trace` goldens): numbers via Rust's
//! `f64` `Display`, strings in double quotes with `Debug` escaping, records
//! and lists structurally, closures as `<fn(param, …)>` (their environment is
//! not printed).

use crate::eval::builtin_name;
use crate::ir::{BindingId, Expr, Param};
use std::fmt;
use std::rc::Rc;

#[derive(Clone)]
pub enum Value {
    Number(f64),
    String(Rc<str>),
    Bool(bool),
    List(Rc<Vec<Value>>),
    /// Field order is the construction order (deterministic output).
    Record(Rc<Vec<(String, Value)>>),
    Closure(Rc<Closure>),
    Builtin(crate::eval::Builtin),
}

/// A lambda value: its IR params/body (shared with the [`crate::ir::Module`])
/// plus the environment captured at its creation site.
pub struct Closure {
    pub params: Rc<Vec<Param>>,
    pub body: Rc<Expr>,
    pub env: Env,
}

/// Lexical environment: one scope per enclosing lambda call, as a persistent
/// parent chain (closures keep their creation-site chain alive via `Rc`).
#[derive(Clone)]
pub struct Env(Option<Rc<Scope>>);

struct Scope {
    vars: Vec<(BindingId, Value)>,
    parent: Env,
}

impl Env {
    pub fn empty() -> Env {
        Env(None)
    }

    pub fn child(&self, vars: Vec<(BindingId, Value)>) -> Env {
        Env(Some(Rc::new(Scope {
            vars,
            parent: self.clone(),
        })))
    }

    pub fn lookup(&self, binding: BindingId) -> Option<Value> {
        let mut cur = self;
        while let Some(scope) = &cur.0 {
            if let Some((_, value)) = scope.vars.iter().find(|(b, _)| *b == binding) {
                return Some(value.clone());
            }
            cur = &scope.parent;
        }
        None
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Number(n) => write!(f, "{n}"),
            Value::String(s) => write!(f, "{s:?}"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::List(items) => {
                write!(f, "[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{item}")?;
                }
                write!(f, "]")
            }
            Value::Record(fields) => {
                write!(f, "{{ ")?;
                for (i, (name, value)) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{name}: {value}")?;
                }
                write!(f, " }}")
            }
            Value::Closure(closure) => {
                write!(f, "<fn(")?;
                for (i, param) in closure.params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", param.name)?;
                }
                write!(f, ")>")
            }
            Value::Builtin(b) => write!(f, "<builtin {}>", builtin_name(*b)),
        }
    }
}

impl Value {
    /// What kind of value this is, for error messages ("cannot call a number").
    pub fn kind_name(&self) -> &'static str {
        match self {
            Value::Number(_) => "a number",
            Value::String(_) => "a string",
            Value::Bool(_) => "a bool",
            Value::List(_) => "a list",
            Value::Record(_) => "a record",
            Value::Closure(_) => "a function",
            Value::Builtin(_) => "a builtin",
        }
    }
}
