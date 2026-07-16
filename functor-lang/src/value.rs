//! Runtime values for the B3 interpreter. `Rc`-shared and cheap to clone;
//! Functor Lang data is immutable, so sharing is safe.
//!
//! The `Display` impl is the canonical textual form used by `functor-lang run`/`trace`
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
    /// At least two elements; structural equality, `(1, 2)` display.
    Tuple(Rc<Vec<Value>>),
    /// Field order is the construction order (deterministic output).
    Record(Rc<Vec<(String, Value)>>),
    /// A variant-type value: a constructor applied to its (positional)
    /// arguments. Nullary constructors are variants with no args.
    /// Equality is structural (same constructor, equal args).
    Variant {
        ctor: Rc<str>,
        args: Rc<Vec<Value>>,
    },
    /// An unapplied parameterful constructor (`Circle` passed as a value,
    /// e.g. to `List.map`) — callable; applying it with the declared arity
    /// yields a [`Value::Variant`]. Nullary constructors never produce this
    /// (used bare, they ARE the variant value).
    Ctor {
        name: Rc<str>,
        arity: usize,
    },
    Closure(Rc<Closure>),
    /// A partially-applied callable: the underlying callable plus the
    /// arguments supplied so far. Produced only when a call supplies FEWER
    /// arguments than the callee's arity; calling it with the remaining
    /// arguments saturates and dispatches. The saturated call path never
    /// allocates one.
    Partial(Rc<Partial>),
    Builtin(crate::eval::Builtin),
    /// A host-provided external function (see [`crate::eval::Host`]),
    /// identified by its qualified path (`Scene.cube`).
    HostFn(Rc<str>),
    /// An opaque host-owned value (a scene node, a camera, …). The language
    /// can pass it around and hand it back to host functions; it cannot look
    /// inside, compare it, or serialize it.
    HostData(Rc<dyn HostData>),
}

/// What a host value must provide: a type name for display/errors, and
/// `Any` access so the host can downcast its own values back out.
pub trait HostData {
    /// Shown as `<{type_name}>` in run/trace output and error messages.
    fn type_name(&self) -> &'static str;
    fn as_any(&self) -> &dyn std::any::Any;
    /// Whether this opaque value is safe to retain in a model snapshot across
    /// a module reload. Conservative by default: host values such as effects,
    /// subscriptions, and UI trees may contain Functor Lang taggers/messages
    /// that the generic value walker cannot inspect.
    fn is_reload_safe_snapshot(&self) -> bool {
        false
    }
}

/// A lambda value: its IR params/body (shared with the [`crate::ir::Module`])
/// plus the environment captured at its creation site.
pub struct Closure {
    pub params: Rc<Vec<Param>>,
    pub body: Rc<Expr>,
    pub env: Env,
    /// The lambda's node id in the module that created it — the hook
    /// hot-reload rebinding resolves to a *stable* id at the reload boundary
    /// (see [`crate::rebind`]; runtime closures stay lean, Decision 3 of the
    /// closures design note). Guarded by body pointer identity there, so a
    /// stale id from an older module can never mis-rebind.
    pub expr_id: crate::ir::ExprId,
}

/// A partially-applied callable. `callee` is the underlying callable (a
/// closure, constructor, or builtin — never a host fn, which the host
/// saturates, nor another `Partial`, which is unwrapped before capture);
/// `applied` are the arguments already supplied. Only produced when a call
/// supplies FEWER args than the callee's arity — the saturated call path never
/// allocates one. The count still needed is derived from the callee's arity
/// (see `Display`), so it can never drift out of sync (e.g. after a hot-reload
/// that changes the callee's parameter count).
pub struct Partial {
    pub callee: Value,
    pub applied: Vec<Value>,
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

/// Preview caps (see [`Value::preview`]): the longest string shown unelided,
/// and how many list/tuple elements and record fields render before `…`.
const MAX_PREVIEW_STRING: usize = 40;
const MAX_PREVIEW_ITEMS: usize = 4;
const MAX_PREVIEW_FIELDS: usize = 6;

impl Value {
    /// Whether this value renders short and complete on one line — the
    /// editor shows primitives inline in full, while composites get the
    /// depth-limited [`Value::preview`] inline and the full `Display` on
    /// hover. Callables and host data count as primitive: their `Display` is
    /// already a short opaque tag (`<fn(x)>`, `<ctor Circle>`). Empty
    /// collections are primitive too (`[]` is complete).
    pub fn is_primitive(&self) -> bool {
        match self {
            Value::Number(_) | Value::Bool(_) => true,
            // The cap is CHARACTERS; `take(N+1)` bounds the count work.
            Value::String(s) => s.chars().take(MAX_PREVIEW_STRING + 1).count() <= MAX_PREVIEW_STRING,
            Value::Variant { args, .. } => args.is_empty(),
            Value::List(items) => items.is_empty(),
            Value::Record(fields) => fields.is_empty(),
            Value::Tuple(_) => false, // never empty (two elements minimum)
            Value::Ctor { .. }
            | Value::Closure(_)
            | Value::Partial(_)
            | Value::Builtin(_)
            | Value::HostFn(_)
            | Value::HostData(_) => true,
        }
    }

    /// A one-line, depth-limited preview: scalars in full (long strings
    /// capped), ONE level of structure with nested composites elided to `…`,
    /// and long collections elided after a few items. The full rendering is
    /// `Display`; a primitive's preview equals it.
    pub fn preview(&self) -> String {
        self.preview_at(0)
    }

    fn preview_at(&self, depth: usize) -> String {
        // Below the top level, only scalar-shaped values still render —
        // nested structure elides.
        if depth >= 1 && !self.is_primitive() {
            return "…".to_string();
        }
        match self {
            Value::String(s) if !self.is_primitive() => {
                // Cap at MAX_PREVIEW_STRING CHARACTERS; the trailing ellipsis
                // (inside the quotes) marks the cut. Pop exactly the closing
                // delimiter — a trim would also eat an escaped quote at the cut.
                let cut = s
                    .char_indices()
                    .nth(MAX_PREVIEW_STRING)
                    .map(|(i, _)| i)
                    .unwrap_or(s.len());
                let mut out = format!("{:?}", &s[..cut]);
                out.pop();
                out.push('…');
                out.push('"');
                out
            }
            Value::List(items) => {
                let shown: Vec<String> = items
                    .iter()
                    .take(MAX_PREVIEW_ITEMS)
                    .map(|v| v.preview_at(depth + 1))
                    .collect();
                let tail = if items.len() > MAX_PREVIEW_ITEMS { ", …" } else { "" };
                format!("[{}{tail}]", shown.join(", "))
            }
            Value::Tuple(items) => {
                let shown: Vec<String> = items
                    .iter()
                    .take(MAX_PREVIEW_ITEMS)
                    .map(|v| v.preview_at(depth + 1))
                    .collect();
                let tail = if items.len() > MAX_PREVIEW_ITEMS { ", …" } else { "" };
                format!("({}{tail})", shown.join(", "))
            }
            Value::Record(fields) => {
                let shown: Vec<String> = fields
                    .iter()
                    .take(MAX_PREVIEW_FIELDS)
                    .map(|(name, value)| format!("{name}: {}", value.preview_at(depth + 1)))
                    .collect();
                let tail = if fields.len() > MAX_PREVIEW_FIELDS { ", …" } else { "" };
                format!("{{ {}{tail} }}", shown.join(", "))
            }
            Value::Variant { ctor, args } if !args.is_empty() => {
                let shown: Vec<String> = args
                    .iter()
                    .take(MAX_PREVIEW_ITEMS)
                    .map(|v| v.preview_at(depth + 1))
                    .collect();
                let tail = if args.len() > MAX_PREVIEW_ITEMS { ", …" } else { "" };
                format!("{ctor}({}{tail})", shown.join(", "))
            }
            // Every remaining shape is primitive: `Display` is already short.
            other => other.to_string(),
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Number(n) => write!(f, "{n}"),
            Value::String(s) => write!(f, "{s:?}"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Tuple(items) => {
                write!(f, "(")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{item}")?;
                }
                write!(f, ")")
            }
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
            Value::Variant { ctor, args } => {
                write!(f, "{ctor}")?;
                if args.is_empty() {
                    return Ok(());
                }
                write!(f, "(")?;
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{arg}")?;
                }
                write!(f, ")")
            }
            Value::Ctor { name, .. } => write!(f, "<ctor {name}>"),
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
            Value::Partial(p) => {
                // How many args are still needed, derived live from the
                // callee's arity (a Partial's callee is always a closure,
                // ctor, or builtin — see [`Partial`]).
                let arity = match &p.callee {
                    Value::Closure(c) => c.params.len(),
                    Value::Ctor { arity, .. } => *arity,
                    Value::Builtin(b) => crate::eval::builtin_arity(*b),
                    _ => p.applied.len(),
                };
                write!(
                    f,
                    "<partial {} more>",
                    arity.saturating_sub(p.applied.len())
                )
            }
            Value::Builtin(b) => write!(f, "<builtin {}>", builtin_name(*b)),
            Value::HostFn(path) => write!(f, "<host {path}>"),
            Value::HostData(data) => write!(f, "<{}>", data.type_name()),
        }
    }
}

impl Value {
    /// Whether a retained snapshot is independent of a loaded Functor module.
    ///
    /// Closures (including a partially-applied closure) hold `Rc`s into the
    /// module IR that created them. First-class constructors also carry the
    /// arity declared by that module. Plain immutable data is safe; callable
    /// values are conservatively excluded, and opaque host values decide
    /// through [`HostData`].
    pub fn is_reload_safe_snapshot(&self) -> bool {
        match self {
            Value::List(items) | Value::Tuple(items) => {
                items.iter().all(Value::is_reload_safe_snapshot)
            }
            Value::Record(fields) => fields
                .iter()
                .all(|(_, value)| value.is_reload_safe_snapshot()),
            Value::Variant { args, .. } => args.iter().all(Value::is_reload_safe_snapshot),
            Value::Closure(_) => false,
            Value::Partial(partial) => {
                partial.callee.is_reload_safe_snapshot()
                    && partial.applied.iter().all(Value::is_reload_safe_snapshot)
            }
            Value::Number(_) | Value::String(_) | Value::Bool(_) => true,
            Value::Ctor { .. } | Value::Builtin(_) | Value::HostFn(_) => false,
            Value::HostData(data) => data.is_reload_safe_snapshot(),
        }
    }

    /// What kind of value this is, for error messages ("cannot call a number").
    pub fn kind_name(&self) -> &'static str {
        match self {
            Value::Number(_) => "a number",
            Value::String(_) => "a string",
            Value::Bool(_) => "a bool",
            Value::List(_) => "a list",
            Value::Tuple(_) => "a tuple",
            Value::Record(_) => "a record",
            Value::Variant { .. } => "a variant",
            Value::Ctor { .. } => "a constructor",
            Value::Closure(_) => "a function",
            Value::Partial(_) => "a partially-applied function",
            Value::Builtin(_) => "a builtin",
            Value::HostFn(_) => "a host function",
            Value::HostData(data) => data.type_name(),
        }
    }
}
