//! Hot-reload rebinding for closures stored in the model — B5 part 2
//! (docs/mle.md; design: `~/notes/ideas/mle-language/closures.md`).
//!
//! Globals rebind across a reload for free (they are late-bound by name at
//! call time), but a closure VALUE held in the model keeps `Rc`s into the
//! old module's IR — before this pass, editing a stored behavior did
//! nothing. The design note's decisions, implemented here:
//!
//! - **Rebind, not content-address**: a stored closure adopts the edited
//!   code, exactly like `tick` does. Its captured environment is carried
//!   over — new code, old data.
//! - **Stable ids by name, resolved at the boundary**: a lambda's id is its
//!   enclosing def's name (`spawn`), with `#k` ordinals for lambdas after
//!   the first in traversal order (`spawn#1`, …). Runtime closures carry
//!   only their [`ExprId`]; both id tables are derived HERE, at reload
//!   time, off the hot path.
//! - **Fail loud, keep running**: a closure whose id has no match in the
//!   new module (renamed/deleted def, shifted ordinal), or whose new body
//!   captures a name the old env cannot supply, keeps its old behavior and
//!   reports a warning — a reload must never kill the session.
//!
//! Captured-env carry-over is BY NAME: binding ids are module-specific, so
//! the new lambda's free variables (name + new id) are resolved against the
//! old env innermost-first via the old module's binder-name table, and a
//! fresh single-scope env is built. Values inside that env are rebound
//! recursively (closures capturing closures), as are values inside lists,
//! records, and variants. MLE data is acyclic (immutable values, late-bound
//! recursion), so the walk terminates.
//!
//! A stale closure from TWO reloads ago (kept once with a warning) carries
//! an id from a module we no longer have; ids are sequential per module, so
//! lookup could collide. The body `Rc` pointer-identity guard makes that
//! impossible: a closure only rebinds if its body IS the old module's node
//! for that id.

use std::collections::HashMap;
use std::rc::Rc;

use crate::ir::{BindingId, Expr, ExprId, ExprKind, Module, Param, Pattern, PatternKind};
use crate::value::{Closure, Env, Value};

/// What a reload's rebind pass did — the producer prints this.
#[derive(Debug, Default)]
pub struct RebindReport {
    /// Closures rebound to new code (old env carried over).
    pub rebound: usize,
    /// Closures kept on their old code, with why.
    pub warnings: Vec<String>,
}

/// One lambda of a module, keyed by stable id.
struct LambdaInfo {
    params: Rc<Vec<Param>>,
    body: Rc<Expr>,
    expr_id: ExprId,
    /// Free variables of the body — captured LOCALS only (globals are
    /// late-bound by name and need no carry-over): `(name, binding)` pairs
    /// in first-use order.
    free: Vec<(String, BindingId)>,
}

/// Both directions of a module's lambda identity, derived on demand.
struct ModuleIndex {
    by_stable: HashMap<String, LambdaInfo>,
    stable_by_expr: HashMap<u32, String>,
    /// Binder name for every binding id (params, `let`s, pattern vars) —
    /// the table that makes old envs resolvable by name.
    binder_names: HashMap<u32, String>,
}

/// Rebind every closure reachable from `value`: closures created by
/// `old` whose stable id resolves in `new` get the new code with their
/// captured env carried over by name. Everything else is preserved.
pub fn rebind_value(value: &Value, old: &Module, new: &Module) -> (Value, RebindReport) {
    let old_index = index_module(old);
    let new_index = index_module(new);
    let mut report = RebindReport::default();
    let rebound = walk(value, &old_index, &new_index, &mut report);
    (rebound, report)
}

fn walk(value: &Value, old: &ModuleIndex, new: &ModuleIndex, report: &mut RebindReport) -> Value {
    match value {
        Value::Number(_)
        | Value::String(_)
        | Value::Bool(_)
        | Value::Ctor { .. }
        | Value::Builtin(_)
        | Value::HostFn(_)
        // Host values are opaque to the language; they cannot hold MLE
        // closures.
        | Value::HostData(_) => value.clone(),
        Value::List(items) => Value::List(Rc::new(
            items.iter().map(|v| walk(v, old, new, report)).collect(),
        )),
        Value::Record(fields) => Value::Record(Rc::new(
            fields
                .iter()
                .map(|(n, v)| (n.clone(), walk(v, old, new, report)))
                .collect(),
        )),
        Value::Variant { ctor, args } => Value::Variant {
            ctor: ctor.clone(),
            args: Rc::new(args.iter().map(|v| walk(v, old, new, report)).collect()),
        },
        Value::Closure(closure) => rebind_closure(closure, old, new, report),
    }
}

fn rebind_closure(
    closure: &Rc<Closure>,
    old: &ModuleIndex,
    new: &ModuleIndex,
    report: &mut RebindReport,
) -> Value {
    let keep = |report: &mut RebindReport, why: String| {
        report.warnings.push(why);
        Value::Closure(closure.clone())
    };
    // Identify the closure against the OLD module. The pointer guard makes a
    // stale id (a closure kept across an earlier reload) unidentifiable
    // rather than wrongly identified.
    let stable = old.stable_by_expr.get(&closure.expr_id.raw()).filter(|id| {
        old.by_stable
            .get(id.as_str())
            .is_some_and(|info| Rc::ptr_eq(&info.body, &closure.body))
    });
    let Some(stable) = stable else {
        return keep(
            report,
            "a stored closure predates the previous reload; keeping its old body".to_string(),
        );
    };
    let Some(info) = new.by_stable.get(stable) else {
        return keep(
            report,
            format!(
                "stored closure `{stable}` has no match after the edit \
(renamed, deleted, or moved); keeping its old body"
            ),
        );
    };
    // Carry the captured env over BY NAME: resolve each free variable of the
    // NEW body against the OLD env (innermost-first), rebinding the carried
    // values themselves recursively.
    let mut vars = Vec::with_capacity(info.free.len());
    for (name, new_binding) in &info.free {
        let is_name = |b: BindingId| old.binder_names.get(&b.0) == Some(name);
        let Some(value) = closure.env.find_by(is_name) else {
            return keep(
                report,
                format!(
                    "stored closure `{stable}` now captures `{name}`, which its saved \
environment does not have; keeping its old body"
                ),
            );
        };
        let value = walk(&value.clone(), old, new, report);
        vars.push((*new_binding, value));
    }
    report.rebound += 1;
    Value::Closure(Rc::new(Closure {
        params: info.params.clone(),
        body: info.body.clone(),
        env: Env::empty().child(vars),
        expr_id: info.expr_id,
    }))
}

/// Derive a module's lambda-identity index (both directions) plus its
/// binder-name table. Runs only at the reload boundary.
fn index_module(module: &Module) -> ModuleIndex {
    let mut index = ModuleIndex {
        by_stable: HashMap::new(),
        stable_by_expr: HashMap::new(),
        binder_names: HashMap::new(),
    };
    for def in &module.defs {
        let mut ordinal = 0usize;
        collect(&def.value, &def.name, &mut ordinal, &mut index);
    }
    index
}

/// Walk one def's expression tree: record every binder's name, and register
/// each lambda under `def` / `def#k` (traversal order).
fn collect(expr: &Expr, def: &str, ordinal: &mut usize, index: &mut ModuleIndex) {
    if let ExprKind::Lambda { params, body, .. } = &expr.kind {
        let stable = if *ordinal == 0 {
            def.to_string()
        } else {
            format!("{def}#{ordinal}")
        };
        *ordinal += 1;
        for param in params.iter() {
            index
                .binder_names
                .insert(param.binding.0, param.name.clone());
        }
        let mut free = Vec::new();
        let mut bound: Vec<BindingId> = params.iter().map(|p| p.binding).collect();
        free_vars(body, &mut bound, &mut free);
        index.stable_by_expr.insert(expr.id.0, stable.clone());
        index.by_stable.insert(
            stable,
            LambdaInfo {
                params: params.clone(),
                body: body.clone(),
                expr_id: expr.id,
                free,
            },
        );
    }
    each_child(expr, &mut |child| collect(child, def, ordinal, index));
    // Binders outside lambdas (top-level initializers' `let`s) still need
    // names for the table; pattern/let binders are recorded in free_vars for
    // lambda bodies, so cover the rest here.
    record_binders(expr, index);
}

fn record_binders(expr: &Expr, index: &mut ModuleIndex) {
    match &expr.kind {
        ExprKind::Let { binding, name, .. } => {
            index.binder_names.insert(binding.0, name.clone());
        }
        ExprKind::Match { arms, .. } => {
            for arm in arms {
                pattern_binders(&arm.pattern, &mut |binding, name| {
                    index.binder_names.insert(binding.0, name.to_string());
                });
            }
        }
        _ => {}
    }
}

/// Free variables of `body`: `Local` references whose binding is not in
/// `bound` (params of the lambda itself plus binders introduced inside).
/// `LocalMut` cannot cross a lambda boundary (lowering rejects the capture),
/// so only `Local` matters. First-use order, deduplicated.
fn free_vars(expr: &Expr, bound: &mut Vec<BindingId>, free: &mut Vec<(String, BindingId)>) {
    match &expr.kind {
        ExprKind::Local { binding, name } => {
            if !bound.contains(binding) && !free.iter().any(|(_, b)| b == binding) {
                free.push((name.clone(), *binding));
            }
        }
        ExprKind::Let {
            binding,
            value,
            body,
            ..
        } => {
            free_vars(value, bound, free);
            bound.push(*binding);
            free_vars(body, bound, free);
            bound.pop();
        }
        ExprKind::Match { scrutinee, arms } => {
            free_vars(scrutinee, bound, free);
            for arm in arms {
                let before = bound.len();
                pattern_binders(&arm.pattern, &mut |binding, _| bound.push(binding));
                free_vars(&arm.body, bound, free);
                bound.truncate(before);
            }
        }
        // A nested lambda's own params bind inside it; its OTHER free vars
        // are (transitively) free here too.
        ExprKind::Lambda { params, body, .. } => {
            let before = bound.len();
            bound.extend(params.iter().map(|p| p.binding));
            free_vars(body, bound, free);
            bound.truncate(before);
        }
        _ => each_child(expr, &mut |child| free_vars(child, bound, free)),
    }
}

fn pattern_binders(pattern: &Pattern, f: &mut impl FnMut(BindingId, &str)) {
    match &pattern.kind {
        PatternKind::Var { binding, name } => f(*binding, name),
        PatternKind::Ctor { args, .. } => {
            for arg in args {
                pattern_binders(arg, f);
            }
        }
        PatternKind::Wildcard
        | PatternKind::Number(_)
        | PatternKind::Bool(_)
        | PatternKind::String(_) => {}
    }
}

/// Apply `f` to every direct child expression of `expr`.
fn each_child(expr: &Expr, f: &mut impl FnMut(&Expr)) {
    match &expr.kind {
        ExprKind::Number(_)
        | ExprKind::String(_)
        | ExprKind::Bool(_)
        | ExprKind::Local { .. }
        | ExprKind::LocalMut { .. }
        | ExprKind::Global(_)
        | ExprKind::External(_)
        | ExprKind::Ctor { .. } => {}
        ExprKind::Record(fields) => {
            for field in fields {
                f(&field.value);
            }
        }
        ExprKind::RecordUpdate { base, fields } => {
            f(base);
            for field in fields {
                f(&field.value);
            }
        }
        ExprKind::List(items) => {
            for item in items {
                f(item);
            }
        }
        ExprKind::Let { value, body, .. } => {
            f(value);
            f(body);
        }
        ExprKind::Assign { value, rest, .. } => {
            f(value);
            f(rest);
        }
        ExprKind::FieldAccess { object, .. } => f(object),
        ExprKind::Lambda { body, .. } => f(body),
        ExprKind::Call { callee, args } => {
            f(callee);
            for arg in args {
                f(arg);
            }
        }
        ExprKind::Binary { lhs, rhs, .. } => {
            f(lhs);
            f(rhs);
        }
        ExprKind::Neg(inner) => f(inner),
        ExprKind::Match { scrutinee, arms } => {
            f(scrutinee);
            for arm in arms {
                f(&arm.body);
            }
        }
    }
}
