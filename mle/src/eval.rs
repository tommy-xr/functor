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

use crate::ir::{Def, Expr, ExprKind, Module};
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

/// Evaluate a lowered module (see the module doc for semantics).
pub fn run(module: &Module, tracing: Tracing) -> Result<RunRecord, RunError> {
    let mut interp = Interp {
        globals: HashMap::new(),
        trace: Vec::new(),
        tracing,
        depth: 0,
        call_depth: 0,
    };
    let mut bindings = Vec::new();
    for def in &module.defs {
        let value = interp.eval(&def.value, &Env::empty())?;
        interp.globals.insert(def.name.clone(), value.clone());
        bindings.push((def.name.clone(), value));
    }
    let outcome = match module.defs.iter().find(|def| def.name == "main") {
        Some(main_def) => RunOutcome::Main(interp.call_main(main_def)?),
        None => RunOutcome::Bindings(bindings),
    };
    Ok(RunRecord {
        outcome,
        trace: interp.trace,
    })
}

struct Interp {
    globals: HashMap<String, Value>,
    trace: Vec<TraceEvent>,
    tracing: Tracing,
    depth: usize,
    call_depth: usize,
}

impl Interp {
    fn call_main(&mut self, def: &Def) -> Result<Value, RunError> {
        let main = self.globals.get("main").expect("just defined").clone();
        match &main {
            Value::Closure(closure) if closure.params.is_empty() => {
                self.call(main.clone(), vec![], "main".to_string(), def.span)
            }
            Value::Closure(_) => Err(RunError {
                message: "`main` must take no parameters to be runnable".to_string(),
                span: def.span,
            }),
            // A non-function `main` is just a value; report it directly.
            _ => Ok(main),
        }
    }

    fn eval(&mut self, expr: &Expr, env: &Env) -> Result<Value, RunError> {
        self.depth += 1;
        if self.depth > MAX_EVAL_DEPTH {
            self.depth -= 1;
            return Err(RunError {
                message: "evaluation nested too deeply (infinite recursion?)".to_string(),
                span: expr.span,
            });
        }
        let result = self.eval_inner(expr, env);
        self.depth -= 1;
        result
    }

    fn eval_inner(&mut self, expr: &Expr, env: &Env) -> Result<Value, RunError> {
        use crate::ast::BinOp;
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
            ExprKind::External(path) => match builtin(path) {
                Some(b) => Ok(Value::Builtin(b)),
                None => Err(RunError {
                    message: format!("unknown external `{}`", path.join(".")),
                    span: expr.span,
                }),
            },
            ExprKind::Record(fields) => {
                let mut out = Vec::with_capacity(fields.len());
                for field in fields {
                    out.push((field.name.clone(), self.eval(&field.value, env)?));
                }
                Ok(Value::Record(Rc::new(out)))
            }
            ExprKind::List(items) => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(self.eval(item, env)?);
                }
                Ok(Value::List(Rc::new(out)))
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
            }))),
            ExprKind::Call { callee, args } => {
                let callee_value = self.eval(callee, env)?;
                let mut arg_values = Vec::with_capacity(args.len());
                for arg in args {
                    arg_values.push(self.eval(arg, env)?);
                }
                self.call(callee_value, arg_values, callee_label(callee), expr.span)
            }
            ExprKind::Binary { op, lhs, rhs } => {
                let lhs_value = self.eval(lhs, env)?;
                let rhs_value = self.eval(rhs, env)?;
                match op {
                    BinOp::Add => self.arith(lhs_value, rhs_value, expr.span, |a, b| a + b),
                    BinOp::Sub => self.arith(lhs_value, rhs_value, expr.span, |a, b| a - b),
                    BinOp::Mul => self.arith(lhs_value, rhs_value, expr.span, |a, b| a * b),
                    // Division follows IEEE-754 (x/0 is ±inf/NaN, printed as
                    // `inf`/`NaN`) — one number type, no checked division.
                    BinOp::Div => self.arith(lhs_value, rhs_value, expr.span, |a, b| a / b),
                    BinOp::Lt => self.compare(lhs_value, rhs_value, expr.span, |a, b| a < b),
                    BinOp::Gt => self.compare(lhs_value, rhs_value, expr.span, |a, b| a > b),
                    BinOp::Eq => Ok(Value::Bool(value_eq(&lhs_value, &rhs_value, expr.span)?)),
                }
            }
            ExprKind::Neg(inner) => match self.eval(inner, env)? {
                Value::Number(n) => Ok(Value::Number(-n)),
                other => Err(RunError {
                    message: format!("cannot negate {}", other.kind_name()),
                    span: expr.span,
                }),
            },
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
    /// errors raised by the call itself (arity, not-callable).
    fn call(
        &mut self,
        callee: Value,
        args: Vec<Value>,
        label: String,
        span: Span,
    ) -> Result<Value, RunError> {
        self.trace_enter(&label, &args);
        self.call_depth += 1;
        let result = match &callee {
            Value::Closure(closure) => {
                if closure.params.len() != args.len() {
                    Err(RunError {
                        message: format!(
                            "`{label}` takes {} argument(s), got {}",
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
            Value::Builtin(b) => self.call_builtin(*b, args, span),
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
        if self.tracing == Tracing::Off || self.trace.len() >= MAX_TRACE_EVENTS {
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
                        match self.call(f.clone(), vec![item.clone()], element_label(b, i), span)? {
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
            Builtin::ListFold => match args.as_slice() {
                [f, init, Value::List(items)] => {
                    let mut acc = init.clone();
                    for (i, item) in items.iter().enumerate() {
                        acc = self.call(
                            f.clone(),
                            vec![acc, item.clone()],
                            element_label(b, i),
                            span,
                        )?;
                    }
                    Ok(acc)
                }
                _ => err(
                    "List.fold(fn, init, list) expects a function, an initial value, and a list"
                        .to_string(),
                ),
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
        }
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
        (Value::Closure(_) | Value::Builtin(_), _) | (_, Value::Closure(_) | Value::Builtin(_)) => {
            Err(RunError {
                message: "functions cannot be compared with `==`".to_string(),
                span,
            })
        }
        // Different kinds are simply unequal (structural, not typed — B4 adds
        // the typechecker that would reject this statically).
        _ => Ok(false),
    }
}

/// The trace label for a call site, from its callee's IR shape.
fn callee_label(callee: &Expr) -> String {
    match &callee.kind {
        ExprKind::Global(name) => name.clone(),
        ExprKind::Local { name, .. } => name.clone(),
        ExprKind::External(path) => path.join("."),
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
    ListMaximum,
    TextConcat,
    TextFromFloat,
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
        "List.maximum" => Builtin::ListMaximum,
        "Text.concat" => Builtin::TextConcat,
        "Text.fromFloat" => Builtin::TextFromFloat,
        "Text.toBullets" => Builtin::TextToBullets,
        "Math.clamp01" => Builtin::MathClamp01,
        _ => return None,
    })
}

/// The registry name of a builtin (`List.map`), for display.
pub fn builtin_name(b: Builtin) -> &'static str {
    match b {
        Builtin::ListMap => "List.map",
        Builtin::ListFilter => "List.filter",
        Builtin::ListFold => "List.fold",
        Builtin::ListMaximum => "List.maximum",
        Builtin::TextConcat => "Text.concat",
        Builtin::TextFromFloat => "Text.fromFloat",
        Builtin::TextToBullets => "Text.toBullets",
        Builtin::MathClamp01 => "Math.clamp01",
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
