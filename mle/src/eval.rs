//! Tree-walking interpreter over the core IR — Track B3 of `docs/mle.md`.
//!
//! ## Program semantics
//!
//! [`run`] evaluates a module's top-level defs **eagerly, in file order**,
//! into a global environment. Top-level names are mutually visible (see
//! [`crate::lower`]), but eager evaluation means a *top-level value
//! expression* may only demand a global defined above it — demanding a later
//! one is a "used before its definition" error. Code inside a lambda body
//! resolves globals at **call time** (late binding), so forward references
//! between functions work, and a swapped-in global rebinds every caller —
//! the hot-reload semantics the MLE design picked (docs/mle.md, Milestone 0).
//!
//! If the module defines `main` as a zero-parameter function, [`run`] calls
//! it and reports its result; otherwise the outcome is the list of top-level
//! bindings.
//!
//! ## Externals
//!
//! [`ExprKind::External`] names resolve against the builtin registry at
//! evaluation time ([`builtin`]): `List.map`/`filter`/`fold`/`maximum`,
//! `Text.concat`/`fromFloat`/`toBullets`, `Math.clamp01`. An unregistered
//! external is a spanned runtime error.
//!
//! ## Tracing
//!
//! With [`Tracing::On`], every call (closure or builtin) records an
//! enter/exit event pair with rendered argument and result values — the
//! LLM-readable execution story (`mle trace`). Recording stops (with a
//! marker event) after [`MAX_TRACE_EVENTS`] so a hot loop can't produce an
//! unbounded transcript; evaluation itself continues.

use crate::ir::{BindingId, Def, Expr, ExprKind, Module, Pattern, PatternKind};
use crate::span::Span;
use crate::value::{Closure, Env, Value};
use crate::RunError;
use std::collections::HashMap;
use std::rc::Rc;

/// Evaluation depth cap: pathological or unboundedly recursive MLE code must
/// fail as a clean spanned error, not a host stack overflow. Counts *every*
/// nested `eval` entry (expression nesting and calls alike), so it bounds
/// host stack usage directly. Like the parser's `MAX_DEPTH`, the cap must fit
/// a 2 MiB test-thread stack in debug builds (each eval level costs several
/// KB of debug frames); deep iteration belongs in the iterative builtins
/// (`List.map`/`fold`), not user-level recursion.
const MAX_EVAL_DEPTH: usize = 200;

/// Trace-event cap; see the module doc.
pub const MAX_TRACE_EVENTS: usize = 10_000;

#[derive(Clone, Copy, PartialEq)]
pub enum Tracing {
    Off,
    On,
}

/// One recorded call boundary. `depth` is the call nesting at the event, for
/// indentation; values are pre-rendered so the trace owns no `Value`s.
pub enum TraceEvent {
    Enter {
        depth: usize,
        callee: String,
        args: Vec<String>,
    },
    Exit {
        depth: usize,
        result: String,
    },
    Truncated,
}

/// What `run` produced: `main`'s result, or (when there is no runnable
/// `main`) every top-level binding in file order.
pub enum RunOutcome {
    Main(Value),
    Bindings(Vec<(String, Value)>),
}

pub struct RunRecord {
    pub outcome: RunOutcome,
    pub trace: Vec<TraceEvent>,
}

/// A failed run: the error **plus the trace recorded up to it** — a failing
/// program is exactly when the execution story matters most, so the partial
/// trace is never discarded (`mle trace` prints it before the diagnostic).
pub struct RunFailure {
    pub error: RunError,
    pub trace: Vec<TraceEvent>,
}

/// Host-provided externals: the embedding runtime (e.g. Functor's shells)
/// registers extra `Module.function` names beyond the built-in registry —
/// the capability seam from the MLE design notes (Rust handlers provide what
/// the pure language can't). An [`ExprKind::External`] resolves against the
/// builtin registry first, then the host; host calls appear in the trace
/// like any other call. See `functor_runtime_common::mle_prelude` for the
/// first real host.
pub trait Host {
    /// Does this host provide `path` (a joined qualified name, `Scene.cube`)?
    fn provides(&self, path: &str) -> bool;

    /// Perform a host call. Only invoked for paths `provides` accepted.
    fn call(&mut self, path: &str, args: Vec<Value>, span: Span) -> Result<Value, RunError>;
}

/// The hostless host: no externals beyond the builtin registry.
pub struct NoHost;

impl Host for NoHost {
    fn provides(&self, _path: &str) -> bool {
        false
    }

    fn call(&mut self, path: &str, _args: Vec<Value>, span: Span) -> Result<Value, RunError> {
        Err(RunError {
            message: format!("internal: NoHost cannot call `{path}`"),
            span,
        })
    }
}

/// Evaluate a lowered module (see the module doc for semantics).
pub fn run(module: &Module, tracing: Tracing) -> Result<RunRecord, RunFailure> {
    run_with_host(module, tracing, &mut NoHost)
}

/// [`run`], with host-provided externals (see [`Host`]).
pub fn run_with_host(
    module: &Module,
    tracing: Tracing,
    host: &mut dyn Host,
) -> Result<RunRecord, RunFailure> {
    let mut interp = Interp {
        globals: HashMap::new(),
        mut_slots: HashMap::new(),
        trace: Vec::new(),
        tracing,
        depth: 0,
        call_depth: 0,
        host,
    };
    match interp.run_module(module) {
        Ok(outcome) => Ok(RunRecord {
            outcome,
            trace: interp.trace,
        }),
        Err(error) => Err(RunFailure {
            error,
            trace: interp.trace,
        }),
    }
}

/// A persistent interpreter session for embedding (the C2 producer): load a
/// module once, then call top-level functions per frame. Globals are
/// evaluated at load; each `call` runs with a fresh interpreter over the
/// session's globals (Rc-cheap clones), so per-frame state lives entirely in
/// the VALUES passed in and returned — the model stays data (hot-reload can
/// swap the session and keep the model; docs/mle.md C3).
pub struct Session {
    globals: HashMap<String, Value>,
}

impl Session {
    /// Evaluate a module's top-level defs (eagerly, in file order) into a
    /// session. Unlike [`run`], a `main` def is NOT called — loading a game
    /// must not execute anything beyond its initializers.
    pub fn load(module: &Module, host: &mut dyn Host) -> Result<Session, RunFailure> {
        let mut interp = Interp {
            globals: HashMap::new(),
            mut_slots: HashMap::new(),
            trace: Vec::new(),
            tracing: Tracing::Off,
            depth: 0,
            call_depth: 0,
            host,
        };
        match interp.eval_defs(module) {
            Ok(_) => Ok(Session {
                globals: interp.globals,
            }),
            Err(error) => Err(RunFailure {
                error,
                trace: interp.trace,
            }),
        }
    }

    /// The value of a top-level def, if any.
    pub fn global(&self, name: &str) -> Option<Value> {
        self.globals.get(name).cloned()
    }

    /// Call the top-level function `name` with `args`. `span` 0..0 is used
    /// for errors with no better location (the caller is not MLE code).
    pub fn call(
        &self,
        name: &str,
        args: Vec<Value>,
        host: &mut dyn Host,
    ) -> Result<Value, RunError> {
        let callee = self.globals.get(name).cloned().ok_or_else(|| RunError {
            message: format!("no top-level `let {name}` in the module"),
            span: Span::new(0, 0),
        })?;
        let mut interp = Interp {
            globals: self.globals.clone(),
            mut_slots: HashMap::new(),
            trace: Vec::new(),
            tracing: Tracing::Off,
            depth: 0,
            call_depth: 0,
            host,
        };
        interp.call(callee, args, name.to_string(), Span::new(0, 0), None)
    }

    /// Apply a function VALUE (a closure the host is holding — e.g. an
    /// effect's tagger) with `args`. Same execution shape as [`Self::call`];
    /// the label names it in traces/errors.
    pub fn apply(
        &self,
        callee: Value,
        args: Vec<Value>,
        label: &str,
        host: &mut dyn Host,
    ) -> Result<Value, RunError> {
        let mut interp = Interp {
            globals: self.globals.clone(),
            mut_slots: HashMap::new(),
            trace: Vec::new(),
            tracing: Tracing::Off,
            depth: 0,
            call_depth: 0,
            host,
        };
        interp.call(callee, args, label.to_string(), Span::new(0, 0), None)
    }
}

struct Interp<'h> {
    globals: HashMap<String, Value>,
    /// Live `let mut` slots, keyed by binding, as a stack per binding (the
    /// same binding can be re-entered through indirect recursion). Lowering
    /// guarantees a slot is only touched within its `let`'s dynamic extent —
    /// mut bindings cannot be captured — so push/pop brackets are exact.
    mut_slots: HashMap<u32, Vec<Value>>,
    trace: Vec<TraceEvent>,
    tracing: Tracing,
    depth: usize,
    call_depth: usize,
    host: &'h mut dyn Host,
}

impl Interp<'_> {
    /// Evaluate every top-level def, eagerly, in file order (no `main` call —
    /// that is [`Interp::run_module`]'s CLI behavior, not a load semantic).
    fn eval_defs(&mut self, module: &Module) -> Result<Vec<(String, Value)>, RunError> {
        let mut bindings = Vec::new();
        for def in &module.defs {
            let value = self.eval(&def.value, &Env::empty())?;
            self.globals.insert(def.name.clone(), value.clone());
            bindings.push((def.name.clone(), value));
        }
        Ok(bindings)
    }

    fn run_module(&mut self, module: &Module) -> Result<RunOutcome, RunError> {
        let bindings = self.eval_defs(module)?;
        match module.defs.iter().find(|def| def.name == "main") {
            Some(main_def) => Ok(RunOutcome::Main(self.call_main(main_def)?)),
            None => Ok(RunOutcome::Bindings(bindings)),
        }
    }

    fn call_main(&mut self, def: &Def) -> Result<Value, RunError> {
        let main = self.globals.get("main").expect("just defined").clone();
        match &main {
            Value::Closure(closure) if closure.params.is_empty() => {
                self.call(main.clone(), vec![], "main".to_string(), def.span, None)
            }
            // Builtins, host functions, and unapplied constructors take
            // arguments by definition, so they get the same error as a
            // parameterful closure rather than printing as a value.
            Value::Closure(_) | Value::Builtin(_) | Value::HostFn(_) | Value::Ctor { .. } => {
                Err(RunError {
                    message: "`main` must take no parameters to be runnable".to_string(),
                    span: def.span,
                })
            }
            // A non-function `main` is just a value; report it directly.
            _ => Ok(main),
        }
    }

    fn eval(&mut self, expr: &Expr, env: &Env) -> Result<Value, RunError> {
        self.depth += 1;
        if self.depth > MAX_EVAL_DEPTH {
            self.depth -= 1;
            return Err(RunError {
                message:
                    "evaluation nested too deeply (deep recursion, or deeply nested expressions)"
                        .to_string(),
                span: expr.span,
            });
        }
        let result = self.eval_inner(expr, env);
        self.depth -= 1;
        result
    }

    fn eval_inner(&mut self, expr: &Expr, env: &Env) -> Result<Value, RunError> {
        match &expr.kind {
            ExprKind::Number(n) => Ok(Value::Number(*n)),
            ExprKind::String(s) => Ok(Value::String(Rc::from(s.as_str()))),
            ExprKind::Bool(b) => Ok(Value::Bool(*b)),
            ExprKind::Local { binding, name } => env.lookup(*binding).ok_or_else(|| RunError {
                // Unreachable if lowering is correct; fail loud rather than UB.
                message: format!("internal: unbound local `{name}`"),
                span: expr.span,
            }),
            ExprKind::Global(name) => self.globals.get(name).cloned().ok_or_else(|| RunError {
                message: format!("global `{name}` used before its definition"),
                span: expr.span,
            }),
            ExprKind::External(path) => {
                let joined = path.join(".");
                match builtin(path) {
                    Some(b) => Ok(Value::Builtin(b)),
                    None if self.host.provides(&joined) => {
                        Ok(Value::HostFn(Rc::from(joined.as_str())))
                    }
                    None => Err(RunError {
                        message: format!("unknown external `{joined}`"),
                        span: expr.span,
                    }),
                }
            }
            ExprKind::Record(fields) => {
                let mut out = Vec::with_capacity(fields.len());
                for field in fields {
                    out.push((field.name.clone(), self.eval(&field.value, env)?));
                }
                Ok(Value::Record(Rc::new(out)))
            }
            ExprKind::Tuple(items) => {
                let mut values = Vec::with_capacity(items.len());
                for item in items {
                    values.push(self.eval(item, env)?);
                }
                Ok(Value::Tuple(Rc::new(values)))
            }
            ExprKind::List(items) => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(self.eval(item, env)?);
                }
                Ok(Value::List(Rc::new(out)))
            }
            ExprKind::ListCons { items, tail } => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(self.eval(item, env)?);
                }
                match self.eval(tail, env)? {
                    Value::List(rest) => {
                        out.extend(rest.iter().cloned());
                        Ok(Value::List(Rc::new(out)))
                    }
                    other => Err(RunError {
                        message: format!(
                            "`..` spreads a list, but the tail is {}",
                            other.kind_name()
                        ),
                        span: tail.span,
                    }),
                }
            }
            ExprKind::RecordUpdate { base, fields } => {
                let base_value = self.eval(base, env)?;
                let Value::Record(base_fields) = &base_value else {
                    return Err(RunError {
                        message: format!(
                            "`with` update on {}, not a record",
                            base_value.kind_name()
                        ),
                        span: expr.span,
                    });
                };
                let mut out = base_fields.as_ref().clone();
                for field in fields {
                    // Validate the target BEFORE evaluating the replacement:
                    // with host externals the RHS can have effects, and an
                    // invalid update must reject without running them.
                    if !out.iter().any(|(name, _)| *name == field.name) {
                        return Err(RunError {
                            message: format!("record has no field `{}` to update", field.name),
                            span: field.span,
                        });
                    }
                    let value = self.eval(&field.value, env)?;
                    let slot = out
                        .iter_mut()
                        .find(|(name, _)| *name == field.name)
                        .expect("checked above");
                    slot.1 = value;
                }
                Ok(Value::Record(Rc::new(out)))
            }
            ExprKind::LocalMut { binding, name } => self
                .mut_slots
                .get(&binding.0)
                .and_then(|stack| stack.last())
                .cloned()
                .ok_or_else(|| RunError {
                    // Unreachable if lowering is correct; fail loud.
                    message: format!("internal: dead mut slot `{name}`"),
                    span: expr.span,
                }),
            ExprKind::Let {
                binding,
                mutable,
                value,
                body,
                ..
            } => {
                let value = self.eval(value, env)?;
                if *mutable {
                    self.mut_slots.entry(binding.0).or_default().push(value);
                    let result = self.eval(body, env);
                    self.mut_slots
                        .get_mut(&binding.0)
                        .expect("pushed above")
                        .pop();
                    result
                } else {
                    let child = env.child(vec![(*binding, value)]);
                    self.eval(body, &child)
                }
            }
            ExprKind::Assign {
                binding,
                name,
                value,
                rest,
            } => {
                let value = self.eval(value, env)?;
                match self
                    .mut_slots
                    .get_mut(&binding.0)
                    .and_then(|stack| stack.last_mut())
                {
                    Some(slot) => *slot = value,
                    None => {
                        return Err(RunError {
                            message: format!("internal: dead mut slot `{name}`"),
                            span: expr.span,
                        })
                    }
                }
                self.eval(rest, env)
            }
            ExprKind::FieldAccess { object, field } => {
                let object_value = self.eval(object, env)?;
                match &object_value {
                    Value::Record(fields) => fields
                        .iter()
                        .find(|(name, _)| name == field)
                        .map(|(_, value)| value.clone())
                        .ok_or_else(|| RunError {
                            message: format!("record has no field `{field}`"),
                            span: expr.span,
                        }),
                    other => Err(RunError {
                        message: format!("`.{field}` on {}, not a record", other.kind_name()),
                        span: expr.span,
                    }),
                }
            }
            ExprKind::Lambda { params, body, .. } => Ok(Value::Closure(Rc::new(Closure {
                params: params.clone(),
                body: body.clone(),
                env: env.clone(),
                expr_id: expr.id,
            }))),
            ExprKind::Call { callee, args } => {
                let callee_value = self.eval(callee, env)?;
                let mut arg_values = Vec::with_capacity(args.len());
                for arg in args {
                    arg_values.push(self.eval(arg, env)?);
                }
                self.call(
                    callee_value,
                    arg_values,
                    callee_label(callee),
                    expr.span,
                    None,
                )
            }
            ExprKind::Binary { .. } => {
                // Left-assoc chains (`a + b + c + …`) nest down the lhs, and
                // the parser builds them iteratively (no depth guard), so
                // evaluate the lhs spine iteratively too — a flat 500-term sum
                // must not consume eval depth (or host stack) per term.
                let mut spine = Vec::new();
                let mut leaf = expr;
                while let ExprKind::Binary { op, lhs, rhs } = &leaf.kind {
                    spine.push((*op, rhs, leaf.span));
                    leaf = lhs;
                }
                let mut acc = self.eval(leaf, env)?;
                for (op, rhs, span) in spine.into_iter().rev() {
                    let rhs_value = self.eval(rhs, env)?;
                    acc = self.binary_op(op, acc, rhs_value, span)?;
                }
                Ok(acc)
            }
            ExprKind::Neg(inner) => match self.eval(inner, env)? {
                Value::Number(n) => Ok(Value::Number(-n)),
                other => Err(RunError {
                    message: format!("cannot negate {}", other.kind_name()),
                    span: expr.span,
                }),
            },
            // A nullary constructor used bare IS the variant value; a
            // parameterful one is a callable constructor (so `List.map(xs,
            // Circle)` works like any function argument).
            ExprKind::Ctor { name, arity } => {
                if *arity == 0 {
                    Ok(Value::Variant {
                        ctor: Rc::from(name.as_str()),
                        args: Rc::new(Vec::new()),
                    })
                } else {
                    Ok(Value::Ctor {
                        name: Rc::from(name.as_str()),
                        arity: *arity,
                    })
                }
            }
            ExprKind::Match { scrutinee, arms } => {
                let value = self.eval(scrutinee, env)?;
                for arm in arms {
                    let mut vars = Vec::new();
                    if match_pattern(&arm.pattern, &value, &mut vars) {
                        let child = env.child(vars);
                        return self.eval(&arm.body, &child);
                    }
                }
                Err(RunError {
                    message: format!("no pattern matched {value}"),
                    span: expr.span,
                })
            }
        }
    }

    fn binary_op(
        &mut self,
        op: crate::ast::BinOp,
        lhs: Value,
        rhs: Value,
        span: Span,
    ) -> Result<Value, RunError> {
        use crate::ast::BinOp;
        match op {
            BinOp::Add => self.arith(lhs, rhs, span, |a, b| a + b),
            BinOp::Sub => self.arith(lhs, rhs, span, |a, b| a - b),
            BinOp::Mul => self.arith(lhs, rhs, span, |a, b| a * b),
            // Division follows IEEE-754 (x/0 is ±inf/NaN, printed as
            // `inf`/`NaN`) — one number type, no checked division.
            BinOp::Div => self.arith(lhs, rhs, span, |a, b| a / b),
            BinOp::Lt => self.compare(lhs, rhs, span, |a, b| a < b),
            BinOp::Gt => self.compare(lhs, rhs, span, |a, b| a > b),
            BinOp::Eq => Ok(Value::Bool(value_eq(&lhs, &rhs, span)?)),
        }
    }

    fn arith(
        &mut self,
        lhs: Value,
        rhs: Value,
        span: Span,
        op: fn(f64, f64) -> f64,
    ) -> Result<Value, RunError> {
        match (lhs, rhs) {
            (Value::Number(a), Value::Number(b)) => Ok(Value::Number(op(a, b))),
            (a, b) => Err(RunError {
                message: format!(
                    "arithmetic needs numbers, got {} and {}",
                    a.kind_name(),
                    b.kind_name()
                ),
                span,
            }),
        }
    }

    fn compare(
        &mut self,
        lhs: Value,
        rhs: Value,
        span: Span,
        op: fn(f64, f64) -> bool,
    ) -> Result<Value, RunError> {
        match (lhs, rhs) {
            (Value::Number(a), Value::Number(b)) => Ok(Value::Bool(op(a, b))),
            (a, b) => Err(RunError {
                message: format!(
                    "comparison needs numbers, got {} and {}",
                    a.kind_name(),
                    b.kind_name()
                ),
                span,
            }),
        }
    }

    /// Call a value. `label` is the callee's source-level name for the trace
    /// (`report`, `List.map`, `<lambda>`); `span` is the call site, used for
    /// errors raised by the call itself (arity, not-callable). `via` is set
    /// when a builtin is invoking its function argument, so an arity error
    /// blames "the function passed to List.map", not the builtin.
    fn call(
        &mut self,
        callee: Value,
        args: Vec<Value>,
        label: String,
        span: Span,
        via: Option<&'static str>,
    ) -> Result<Value, RunError> {
        self.trace_enter(&label, &args);
        self.call_depth += 1;
        let result = match &callee {
            Value::Closure(closure) => {
                if closure.params.len() != args.len() {
                    let who = match via {
                        Some(builtin) => format!("the function passed to {builtin}"),
                        None => format!("`{label}`"),
                    };
                    Err(RunError {
                        message: format!(
                            "{who} takes {} argument(s), got {}",
                            closure.params.len(),
                            args.len()
                        ),
                        span,
                    })
                } else {
                    let vars = closure.params.iter().map(|p| p.binding).zip(args).collect();
                    let env = closure.env.child(vars);
                    let body = closure.body.clone();
                    self.eval(&body, &env)
                }
            }
            Value::Ctor { name, arity } => {
                if args.len() != *arity {
                    let who = match via {
                        Some(builtin) => format!("the function passed to {builtin}"),
                        None => format!("`{label}`"),
                    };
                    Err(RunError {
                        message: format!("{who} takes {arity} argument(s), got {}", args.len()),
                        span,
                    })
                } else {
                    Ok(Value::Variant {
                        ctor: name.clone(),
                        args: Rc::new(args),
                    })
                }
            }
            Value::Builtin(b) => self.call_builtin(*b, args, span),
            Value::HostFn(path) => self.host.call(path, args, span),
            other => Err(RunError {
                message: format!("cannot call {}", other.kind_name()),
                span,
            }),
        };
        self.call_depth -= 1;
        if let Ok(value) = &result {
            self.trace_exit(value);
        }
        result
    }

    fn trace_enter(&mut self, callee: &str, args: &[Value]) {
        if self.tracing == Tracing::Off {
            return;
        }
        if self.trace.len() >= MAX_TRACE_EVENTS {
            if !matches!(self.trace.last(), Some(TraceEvent::Truncated)) {
                self.trace.push(TraceEvent::Truncated);
            }
            return;
        }
        self.trace.push(TraceEvent::Enter {
            depth: self.call_depth,
            callee: callee.to_string(),
            args: args.iter().map(|a| a.to_string()).collect(),
        });
    }

    fn trace_exit(&mut self, result: &Value) {
        if self.tracing == Tracing::Off {
            return;
        }
        if self.trace.len() >= MAX_TRACE_EVENTS {
            // Same marker as trace_enter: without it, a transcript whose last
            // recorded event is an Enter would read like a hang.
            if !matches!(self.trace.last(), Some(TraceEvent::Truncated)) {
                self.trace.push(TraceEvent::Truncated);
            }
            return;
        }
        self.trace.push(TraceEvent::Exit {
            depth: self.call_depth,
            result: result.to_string(),
        });
    }

    fn call_builtin(
        &mut self,
        b: Builtin,
        args: Vec<Value>,
        span: Span,
    ) -> Result<Value, RunError> {
        let err = |message: String| Err(RunError { message, span });
        match b {
            Builtin::ListMap => match args.as_slice() {
                [Value::List(items), f] => {
                    let mut out = Vec::with_capacity(items.len());
                    for (i, item) in items.iter().enumerate() {
                        out.push(self.call(
                            f.clone(),
                            vec![item.clone()],
                            element_label(b, i),
                            span,
                            Some(builtin_name(b)),
                        )?);
                    }
                    Ok(Value::List(Rc::new(out)))
                }
                _ => err("List.map(list, fn) expects a list and a function".to_string()),
            },
            Builtin::ListFilter => match args.as_slice() {
                [Value::List(items), f] => {
                    let mut out = Vec::new();
                    for (i, item) in items.iter().enumerate() {
                        match self.call(
                            f.clone(),
                            vec![item.clone()],
                            element_label(b, i),
                            span,
                            Some(builtin_name(b)),
                        )? {
                            Value::Bool(true) => out.push(item.clone()),
                            Value::Bool(false) => {}
                            other => {
                                return err(format!(
                                    "List.filter predicate must return a bool, got {}",
                                    other.kind_name()
                                ))
                            }
                        }
                    }
                    Ok(Value::List(Rc::new(out)))
                }
                _ => err("List.filter(list, fn) expects a list and a function".to_string()),
            },
            // List-first like map/filter, so it composes with `|>` (the piped
            // value is PREPENDED as the first argument — see crate::lower).
            Builtin::ListFold => match args.as_slice() {
                [Value::List(items), f, init] => {
                    let mut acc = init.clone();
                    for (i, item) in items.iter().enumerate() {
                        acc = self.call(
                            f.clone(),
                            vec![acc, item.clone()],
                            element_label(b, i),
                            span,
                            Some(builtin_name(b)),
                        )?;
                    }
                    Ok(acc)
                }
                _ => err(
                    "List.fold(list, fn, init) expects a list, a function, and an initial value"
                        .to_string(),
                ),
            },
            // NaN handling follows Rust's `f64::max` (IEEE maximumNumber):
            // NaN elements are ignored unless every element is NaN.
            // `List.range(n)` -> [0, 1, …, n-1] as Floats; n truncates. The
            // count must be finite and sane: MLE numbers permit `inf`
            // (IEEE division), and `inf as usize` would ask the allocator
            // for usize::MAX elements — a process-killing panic, not a
            // recoverable frame error.
            Builtin::ListRange => match args.as_slice() {
                [Value::Number(n)] if n.is_finite() && *n <= 1_000_000.0 => {
                    let count = n.max(0.0) as usize;
                    Ok(Value::List(Rc::new(
                        (0..count).map(|i| Value::Number(i as f64)).collect(),
                    )))
                }
                [Value::Number(n)] => err(format!(
                    "List.range needs a finite count up to 1000000, got {n}"
                )),
                _ => err("List.range(n) expects one number".to_string()),
            },
            Builtin::ListMaximum => match args.as_slice() {
                [Value::List(items)] => {
                    let mut best: Option<f64> = None;
                    for item in items.iter() {
                        match item {
                            Value::Number(n) => {
                                best = Some(best.map_or(*n, |b: f64| b.max(*n)));
                            }
                            other => {
                                return err(format!(
                                    "List.maximum expects numbers, got {}",
                                    other.kind_name()
                                ))
                            }
                        }
                    }
                    match best {
                        Some(n) => Ok(Value::Number(n)),
                        None => err("List.maximum of an empty list".to_string()),
                    }
                }
                _ => err("List.maximum(list) expects one list".to_string()),
            },
            Builtin::TextConcat => match args.as_slice() {
                [Value::String(a), Value::String(b)] => {
                    Ok(Value::String(Rc::from(format!("{a}{b}").as_str())))
                }
                _ => err("Text.concat(a, b) expects two strings".to_string()),
            },
            Builtin::TextFromFloat => match args.as_slice() {
                [Value::Number(n)] => Ok(Value::String(Rc::from(n.to_string().as_str()))),
                _ => err("Text.fromFloat(n) expects one number".to_string()),
            },
            // Fixed-decimal formatting (the F# `sprintf "%.1f"` shape a HUD
            // needs): `Text.fixed(3.14159, 1.0)` is `"3.1"`, and 0 decimals
            // formats whole numbers with no point (`Text.fixed(42.0, 0.0)` is
            // `"42"` — the `%d` shape too). Rust's `{:.prec}` rounding.
            Builtin::TextFixed => match args.as_slice() {
                [Value::Number(n), Value::Number(d)]
                    if *d >= 0.0 && d.fract() == 0.0 && *d <= 12.0 =>
                {
                    Ok(Value::String(Rc::from(
                        format!("{:.*}", *d as usize, n).as_str(),
                    )))
                }
                [Value::Number(_), Value::Number(d)] => err(format!(
                    "Text.fixed needs a whole number of decimals between 0 and 12, got {d}"
                )),
                _ => err("Text.fixed(n, decimals) expects two numbers".to_string()),
            },
            Builtin::TextToBullets => match args.as_slice() {
                [Value::List(items)] => {
                    let mut lines = Vec::with_capacity(items.len());
                    for item in items.iter() {
                        match item {
                            Value::String(s) => lines.push(format!("- {s}")),
                            other => {
                                return err(format!(
                                    "Text.toBullets expects strings, got {}",
                                    other.kind_name()
                                ))
                            }
                        }
                    }
                    Ok(Value::String(Rc::from(lines.join("\n").as_str())))
                }
                _ => err("Text.toBullets(list) expects one list of strings".to_string()),
            },
            Builtin::MathClamp01 => match args.as_slice() {
                [Value::Number(n)] => Ok(Value::Number(n.clamp(0.0, 1.0))),
                _ => err("Math.clamp01(n) expects one number".to_string()),
            },
            Builtin::MathSin => match args.as_slice() {
                [Value::Number(n)] => Ok(Value::Number(n.sin())),
                _ => err("Math.sin(n) expects one number".to_string()),
            },
            Builtin::MathCos => match args.as_slice() {
                [Value::Number(n)] => Ok(Value::Number(n.cos())),
                _ => err("Math.cos(n) expects one number".to_string()),
            },
        }
    }
}

/// Does `pattern` match `value`? Appends each pattern variable's binding on
/// the way (bindings are only used if the whole pattern matched — a pattern
/// either fully matches or its arm is skipped, and sub-patterns are leaves,
/// so a partial append can never observe a half-match). Pure: literal
/// patterns compare primitively, so no function-equality error can arise.
fn match_pattern(pattern: &Pattern, value: &Value, vars: &mut Vec<(BindingId, Value)>) -> bool {
    match &pattern.kind {
        PatternKind::Wildcard => true,
        PatternKind::Var { binding, .. } => {
            vars.push((*binding, value.clone()));
            true
        }
        PatternKind::Ctor { name, args } => match value {
            // Same constructor AND same arg count: lowering fixes the
            // pattern's arity to the declaration, but the VALUE may come
            // from a host (`Session::call`) — mismatched args are a
            // non-match, not UB.
            Value::Variant { ctor, args: vals } if ctor.as_ref() == name => {
                args.len() == vals.len()
                    && args
                        .iter()
                        .zip(vals.iter())
                        .all(|(p, v)| match_pattern(p, v, vars))
            }
            _ => false,
        },
        // Arity must match exactly — a 2-pattern against a 3-tuple is a
        // non-match, like a mismatched ctor.
        PatternKind::Tuple(args) => match value {
            Value::Tuple(vals) => {
                args.len() == vals.len()
                    && args
                        .iter()
                        .zip(vals.iter())
                        .all(|(p, v)| match_pattern(p, v, vars))
            }
            _ => false,
        },
        // `[a, b]` needs an exact-length list; `[h, ..t]` needs at least
        // `items.len()` and binds `t` to the remainder.
        PatternKind::List { items, tail } => match value {
            Value::List(vals) => {
                let long_enough = match tail {
                    Some(_) => vals.len() >= items.len(),
                    None => vals.len() == items.len(),
                };
                if !long_enough {
                    return false;
                }
                if !items
                    .iter()
                    .zip(vals.iter())
                    .all(|(p, v)| match_pattern(p, v, vars))
                {
                    return false;
                }
                if let Some(tail) = tail {
                    let rest = Value::List(Rc::new(vals[items.len()..].to_vec()));
                    match_pattern(tail, &rest, vars)
                } else {
                    true
                }
            }
            _ => false,
        },
        PatternKind::Number(n) => matches!(value, Value::Number(v) if v == n),
        PatternKind::Bool(b) => matches!(value, Value::Bool(v) if v == b),
        PatternKind::String(s) => matches!(value, Value::String(v) if v.as_ref() == s),
    }
}

/// Structural equality for `==`. Functions have no equality — comparing them
/// is a runtime error rather than a silent `false`.
fn value_eq(a: &Value, b: &Value, span: Span) -> Result<bool, RunError> {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => Ok(x == y),
        (Value::String(x), Value::String(y)) => Ok(x == y),
        (Value::Bool(x), Value::Bool(y)) => Ok(x == y),
        (Value::List(xs), Value::List(ys)) => {
            if xs.len() != ys.len() {
                return Ok(false);
            }
            for (x, y) in xs.iter().zip(ys.iter()) {
                if !value_eq(x, y, span)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        // Structural, element-wise; arity difference is simply unequal.
        (Value::Tuple(xs), Value::Tuple(ys)) => {
            if xs.len() != ys.len() {
                return Ok(false);
            }
            for (x, y) in xs.iter().zip(ys.iter()) {
                if !value_eq(x, y, span)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        (Value::Record(xs), Value::Record(ys)) => {
            if xs.len() != ys.len() {
                return Ok(false);
            }
            for (name, x) in xs.iter() {
                match ys.iter().find(|(n, _)| n == name) {
                    Some((_, y)) if value_eq(x, y, span)? => {}
                    _ => return Ok(false),
                }
            }
            Ok(true)
        }
        // Structural: same constructor, equal args (a function argument
        // still raises the function-comparison error below).
        (Value::Variant { ctor: xc, args: xs }, Value::Variant { ctor: yc, args: ys }) => {
            if xc != yc || xs.len() != ys.len() {
                return Ok(false);
            }
            for (x, y) in xs.iter().zip(ys.iter()) {
                if !value_eq(x, y, span)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        // An unapplied constructor is a function value — no equality.
        (Value::Closure(_) | Value::Builtin(_) | Value::HostFn(_) | Value::Ctor { .. }, _)
        | (_, Value::Closure(_) | Value::Builtin(_) | Value::HostFn(_) | Value::Ctor { .. }) => {
            Err(RunError {
                message: "functions cannot be compared with `==`".to_string(),
                span,
            })
        }
        (Value::HostData(_), _) | (_, Value::HostData(_)) => Err(RunError {
            message: "host values cannot be compared with `==`".to_string(),
            span,
        }),
        // Different kinds are simply unequal (structural, not typed — B4 adds
        // the typechecker that would reject this statically).
        _ => Ok(false),
    }
}

/// The trace label for a call site, from its callee's IR shape. Also used by
/// the typechecker ([`crate::types`]) so its call diagnostics name callees
/// the same way.
pub(crate) fn callee_label(callee: &Expr) -> String {
    match &callee.kind {
        ExprKind::Global(name) => name.clone(),
        ExprKind::Local { name, .. } => name.clone(),
        ExprKind::External(path) => path.join("."),
        ExprKind::Ctor { name, .. } => name.clone(),
        _ => "<lambda>".to_string(),
    }
}

/// The trace label for a builtin invoking its function argument on element
/// `i` (`List.map[2]`).
fn element_label(b: Builtin, i: usize) -> String {
    format!("{}[{i}]", builtin_name(b))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Builtin {
    ListMap,
    ListFilter,
    ListFold,
    ListRange,
    MathSin,
    MathCos,
    ListMaximum,
    TextConcat,
    TextFromFloat,
    TextFixed,
    TextToBullets,
    MathClamp01,
}

/// Resolve an [`ExprKind::External`] path against the registry.
pub fn builtin(path: &[String]) -> Option<Builtin> {
    let joined = path.join(".");
    Some(match joined.as_str() {
        "List.map" => Builtin::ListMap,
        "List.filter" => Builtin::ListFilter,
        "List.fold" => Builtin::ListFold,
        "List.range" => Builtin::ListRange,
        "List.maximum" => Builtin::ListMaximum,
        "Text.concat" => Builtin::TextConcat,
        "Text.fromFloat" => Builtin::TextFromFloat,
        "Text.fixed" => Builtin::TextFixed,
        "Text.toBullets" => Builtin::TextToBullets,
        "Math.clamp01" => Builtin::MathClamp01,
        "Math.sin" => Builtin::MathSin,
        "Math.cos" => Builtin::MathCos,
        _ => return None,
    })
}

/// The registry name of a builtin (`List.map`), for display.
pub fn builtin_name(b: Builtin) -> &'static str {
    match b {
        Builtin::ListMap => "List.map",
        Builtin::ListFilter => "List.filter",
        Builtin::ListFold => "List.fold",
        Builtin::ListRange => "List.range",
        Builtin::ListMaximum => "List.maximum",
        Builtin::TextConcat => "Text.concat",
        Builtin::TextFromFloat => "Text.fromFloat",
        Builtin::TextFixed => "Text.fixed",
        Builtin::TextToBullets => "Text.toBullets",
        Builtin::MathClamp01 => "Math.clamp01",
        Builtin::MathSin => "Math.sin",
        Builtin::MathCos => "Math.cos",
    }
}

/// Render a trace as indented enter/exit lines (the `mle trace` output):
///
/// ```text
/// > report([12, 3.5, 40])
///   > List.filter([12, 3.5, 40], <fn(score)>)
///   < [12, 40]
/// < "- score: 12"
/// ```
pub fn render_trace(events: &[TraceEvent]) -> String {
    let mut out = String::new();
    for event in events {
        match event {
            TraceEvent::Enter {
                depth,
                callee,
                args,
            } => {
                out.push_str(&"  ".repeat(*depth));
                out.push_str(&format!("> {callee}({})\n", args.join(", ")));
            }
            TraceEvent::Exit { depth, result } => {
                out.push_str(&"  ".repeat(*depth));
                out.push_str(&format!("< {result}\n"));
            }
            TraceEvent::Truncated => {
                out.push_str("… trace truncated …\n");
            }
        }
    }
    out
}
