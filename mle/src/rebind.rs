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
//! - **Stable ids by name/path, never by bare index** (the note's explicit
//!   warning): a lambda's id is its enclosing def's name plus a path of
//!   NAMED segments where the source provides names — record fields
//!   (`make/fn/.mul`), `let` binders (`=f`) — with positional segments
//!   (`[2]`, `:0`, `|1`) only where nothing is named. Named positions are
//!   stable under inserting/removing siblings; a lambda that only has a
//!   positional identity can still drift on such edits — give it a `let`
//!   name to give it a stable identity.
//! - **Identity resolved at the boundary**: runtime closures carry only
//!   their [`ExprId`]; both id tables are derived HERE, at reload time,
//!   off the hot path.
//! - **Fail loud, keep running**: a closure whose id has no match in the
//!   new module, whose new body captures a name the old one didn't, or
//!   whose capture is now made by a different KIND of binder (a parameter
//!   vs a `let` vs a pattern variable) keeps its old behavior and reports
//!   a warning — a reload must never kill the session.
//!
//! Captured-env carry-over resolves each free variable of the NEW body
//! against the OLD lambda's own free list (name → the exact old
//! `BindingId` the old code referred to → its value in the saved env) —
//! never by scanning the env chain, so a stale shadowed binding of the
//! same name can't be picked up. Carried values rebind recursively
//! (closures capturing closures), as do values inside lists, records, and
//! variants. MLE data is acyclic (immutable values, late-bound recursion),
//! so the walk terminates.
//!
//! A stale closure from TWO reloads ago (kept once with a warning) carries
//! an id from a module we no longer have; ids could collide, so the body
//! `Rc` pointer-identity guard makes it unidentifiable rather than wrongly
//! identified: a closure only rebinds if its body IS the old module's node
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

/// What kind of binder introduced a binding — carried captures must agree
/// (a `k` that was a parameter and is now a `let` is a semantic change the
/// old saved value can't stand in for; see the module doc).
#[derive(Clone, Copy, PartialEq, Debug)]
enum BinderKind {
    Param,
    Let,
    PatternVar,
}

impl BinderKind {
    fn describe(self) -> &'static str {
        match self {
            BinderKind::Param => "a parameter",
            BinderKind::Let => "a `let`",
            BinderKind::PatternVar => "a pattern variable",
        }
    }
}

/// One lambda of a module, keyed by stable id.
struct LambdaInfo {
    params: Rc<Vec<Param>>,
    body: Rc<Expr>,
    expr_id: ExprId,
    /// Free variables of the body — captured LOCALS only (globals are
    /// late-bound by name and need no carry-over): `(name, binding)` pairs
    /// in first-use order. Names are unique within one lambda (lexical
    /// scoping: every free use of a name sees the same binding).
    free: Vec<(String, BindingId)>,
}

/// Both directions of a module's lambda identity, derived on demand.
struct ModuleIndex {
    by_stable: HashMap<String, LambdaInfo>,
    stable_by_expr: HashMap<u32, String>,
    binder_kinds: HashMap<u32, BinderKind>,
}

/// Rebind every closure reachable from `value`: closures created by
/// `old` whose stable id resolves in `new` get the new code with their
/// captured env carried over. Everything else is preserved.
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
        Value::Tuple(items) => Value::Tuple(Rc::new(
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
    // A kept closure is kept ATOMICALLY: its captured env is not walked, so
    // closures inside it stay on their old bodies too — the old body refers
    // to old binding ids, and mixing rebuilt inner values into it would be
    // incoherent. The warning names the outermost closure.
    let keep = |report: &mut RebindReport, why: String| {
        report.warnings.push(why);
        Value::Closure(closure.clone())
    };
    // Identify the closure against the OLD module. The pointer guard makes a
    // stale id (a closure kept across an earlier reload) unidentifiable
    // rather than wrongly identified.
    let old_info = old
        .stable_by_expr
        .get(&closure.expr_id.raw())
        .and_then(|id| old.by_stable.get(id.as_str()).map(|info| (id, info)))
        .filter(|(_, info)| Rc::ptr_eq(&info.body, &closure.body));
    let Some((stable, old_info)) = old_info else {
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
    // An arity change would rebind cleanly and then fail at every call
    // site, one frame later and with a worse message — report it HERE.
    if info.params.len() != closure.params.len() {
        return keep(
            report,
            format!(
                "stored closure `{stable}` changed arity ({} -> {} parameters); \
keeping its old body",
                closure.params.len(),
                info.params.len()
            ),
        );
    }
    // Carry the captured env over: each free variable of the NEW body must
    // be a capture the OLD body also made (same name, same kind of binder) —
    // then the value comes from the exact binding the old code referred to.
    let mut vars = Vec::with_capacity(info.free.len());
    for (name, new_binding) in &info.free {
        let Some((_, old_binding)) = old_info.free.iter().find(|(n, _)| n == name) else {
            return keep(
                report,
                format!(
                    "stored closure `{stable}` now captures `{name}`, which it did not \
capture before; keeping its old body"
                ),
            );
        };
        let (old_kind, new_kind) = (
            old.binder_kinds.get(&old_binding.0).copied(),
            new.binder_kinds.get(&new_binding.0).copied(),
        );
        if old_kind != new_kind {
            let describe = |k: Option<BinderKind>| k.map_or("unknown", BinderKind::describe);
            return keep(
                report,
                format!(
                    "stored closure `{stable}` captures `{name}` differently after the \
edit ({} before, {} now); keeping its old body",
                    describe(old_kind),
                    describe(new_kind)
                ),
            );
        }
        let Some(value) = closure.env.lookup(*old_binding) else {
            return keep(
                report,
                format!(
                    "stored closure `{stable}`: its saved environment is missing \
`{name}`; keeping its old body"
                ),
            );
        };
        let value = walk(&value, old, new, report);
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
/// binder-kind table. Runs only at the reload boundary.
fn index_module(module: &Module) -> ModuleIndex {
    let mut index = ModuleIndex {
        by_stable: HashMap::new(),
        stable_by_expr: HashMap::new(),
        binder_kinds: HashMap::new(),
    };
    for def in &module.defs {
        let mut path: Vec<String> = Vec::new();
        collect(&def.value, &def.name, &mut path, &mut index);
    }
    index
}

/// Walk one def's expression tree building path-based ids (see the module
/// doc) and the binder-kind table. Every multi-child edge contributes a
/// segment — named where the source names it — so ids are unique; a def's
/// root lambda gets the bare def name.
fn collect(expr: &Expr, def: &str, path: &mut Vec<String>, index: &mut ModuleIndex) {
    let seg = |expr: &Expr, segment: String, path: &mut Vec<String>, index: &mut ModuleIndex| {
        path.push(segment);
        collect(expr, def, path, index);
        path.pop();
    };
    match &expr.kind {
        ExprKind::Lambda { params, body, .. } => {
            let stable = if path.is_empty() {
                def.to_string()
            } else {
                format!("{def}/{}", path.join("/"))
            };
            for param in params.iter() {
                index
                    .binder_kinds
                    .insert(param.binding.0, BinderKind::Param);
            }
            let mut free = Vec::new();
            let mut bound: Vec<BindingId> = params.iter().map(|p| p.binding).collect();
            free_vars(body, &mut bound, &mut free);
            index.stable_by_expr.insert(expr.id.raw(), stable.clone());
            index.by_stable.insert(
                stable,
                LambdaInfo {
                    params: params.clone(),
                    body: body.clone(),
                    expr_id: expr.id,
                    free,
                },
            );
            seg(body, "fn".to_string(), path, index);
        }
        ExprKind::Let {
            binding,
            name,
            value,
            body,
            ..
        } => {
            index.binder_kinds.insert(binding.0, BinderKind::Let);
            seg(value, format!("={name}"), path, index);
            // The body is the continuation — transparent, so wrapping code
            // in a new `let` does not shift ids further down the spine.
            collect(body, def, path, index);
        }
        ExprKind::Assign {
            name, value, rest, ..
        } => {
            seg(value, format!(":={name}"), path, index);
            collect(rest, def, path, index);
        }
        ExprKind::Record(fields) => {
            for field in fields {
                seg(&field.value, format!(".{}", field.name), path, index);
            }
        }
        ExprKind::RecordUpdate { base, fields } => {
            seg(base, "base".to_string(), path, index);
            for field in fields {
                seg(&field.value, format!(".{}", field.name), path, index);
            }
        }
        ExprKind::List(items) => {
            for (i, item) in items.iter().enumerate() {
                seg(item, format!("[{i}]"), path, index);
            }
        }
        ExprKind::Tuple(items) => {
            for (i, item) in items.iter().enumerate() {
                seg(item, format!("({i})"), path, index);
            }
        }
        ExprKind::Call { callee, args } => {
            seg(callee, "callee".to_string(), path, index);
            for (i, arg) in args.iter().enumerate() {
                seg(arg, format!(":{i}"), path, index);
            }
        }
        ExprKind::Binary { lhs, rhs, .. } => {
            seg(lhs, "lhs".to_string(), path, index);
            seg(rhs, "rhs".to_string(), path, index);
        }
        ExprKind::Match { scrutinee, arms } => {
            seg(scrutinee, "match".to_string(), path, index);
            for (i, arm) in arms.iter().enumerate() {
                pattern_binders(&arm.pattern, &mut |binding, _| {
                    index.binder_kinds.insert(binding.0, BinderKind::PatternVar);
                });
                seg(&arm.body, format!("|{i}"), path, index);
            }
        }
        // Single-child edges are transparent: uniqueness is preserved, and
        // e.g. negating or field-accessing around a lambda keeps its id.
        ExprKind::Neg(inner) => collect(inner, def, path, index),
        ExprKind::FieldAccess { object, .. } => collect(object, def, path, index),
        ExprKind::Number(_)
        | ExprKind::String(_)
        | ExprKind::Bool(_)
        | ExprKind::Local { .. }
        | ExprKind::LocalMut { .. }
        | ExprKind::Global(_)
        | ExprKind::External(_)
        | ExprKind::Ctor { .. } => {}
    }
}

/// Free variables of `body`: `Local` references whose binding is not in
/// `bound` (params of the lambda itself plus binders introduced inside).
/// `LocalMut` cannot cross a lambda boundary (lowering rejects the capture),
/// so only `Local` matters. First-use order, deduplicated by binding.
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
            // MLE `let` is non-recursive: the value cannot see the binding.
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
        PatternKind::Ctor { args, .. } | PatternKind::Tuple(args) => {
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
pub(crate) fn each_child(expr: &Expr, f: &mut impl FnMut(&Expr)) {
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
        ExprKind::Tuple(items) => {
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
