//! Reload reuse: a hot reload keeps a top-level def's previous value when
//! evaluating it again could only produce the same value
//! ([`crate::Session::load_reusing`]).
//!
//! A def is *unchanged* when, compared with the previous load:
//!
//! - its lowered code is the same, ignoring ids and spans ([`shape_of`]);
//! - its checked type is the same (and known);
//! - its last evaluation read nothing from the host but [`HostInput`]s, and
//!   those still read the same ([`crate::Host::inputs_current`]);
//! - every def it references is unchanged, transitively, and sits on the same
//!   side of it in def order (eager evaluation can only demand earlier defs);
//! - the module's units and unit operators are the same, and resolving them
//!   read nothing from the host (an operator dispatches on its unit's brand).
//!
//! An unchanged def whose value is plain data
//! ([`Value::is_reload_safe_snapshot`]) is reused; every other def evaluates
//! as before. Functions always re-evaluate: a closure points into the module
//! that built it, and building one again is cheap anyway.

use crate::ir::{Def, Expr, ExprKind, Module, Pattern, PatternKind, StringPart};
use crate::ast::TypeName;
use crate::types::ExprTypes;
use crate::value::Value;
use crate::{Host, Span};
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;
use std::sync::Arc;

/// One thing a def's evaluation read from the host, named by the host
/// (`"bytes:data/level.bin"`), and an identity of what it read (a content
/// hash) — the host re-checks it on the next reload.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HostInput {
    pub key: String,
    pub identity: u64,
}

/// How a [`crate::Session::load_reusing`] went: each value def (not a
/// lambda — functions always re-evaluate) by span, and whether it was reused.
#[derive(Clone, Debug, Default)]
pub struct ReuseStats {
    pub values: Vec<(Span, bool)>,
}

impl ReuseStats {
    /// `(reused, values)` among the values `keep` selects (a project's own
    /// files, say, leaving bundled modules out).
    pub fn count(&self, keep: impl Fn(Span) -> bool) -> (usize, usize) {
        let kept = self.values.iter().filter(|(span, _)| keep(*span));
        kept.fold((0, 0), |(reused, values), (_, hit)| (reused + *hit as usize, values + 1))
    }
}

/// Each def's checked type, rendered — what [`crate::Session::load_reusing`]
/// compares across loads. A def missing here is never reused.
pub fn def_types(module: &Module, types: &ExprTypes) -> HashMap<String, String> {
    module
        .defs
        .iter()
        .filter_map(|def| Some((def.name.clone(), types.expr(def.value.id)?.to_string())))
        .collect()
}

/// What one load learned about a def, kept on the session for the next.
#[derive(Clone)]
struct DefRecord {
    shape: u64,
    ty: Option<String>,
    /// Where it sits in def order.
    position: usize,
    /// What its evaluation read from the host; `None` when it read host state
    /// that can change between loads (the live physics world, …).
    inputs: Option<Arc<[HostInput]>>,
}

/// Every def's [`DefRecord`] from one load, plus the unit-operator hash.
/// Plain data, so a load on a worker can hand it to
/// [`crate::Session::load_adopting`].
#[derive(Clone, Default)]
pub struct Records {
    units: u64,
    defs: HashMap<String, DefRecord>,
}

const _: () = {
    const fn send<T: Send>() {}
    send::<Records>()
};

impl Records {
    /// Whether `def`'s last evaluation read project files.
    pub fn read_files(&self, def: &str) -> bool {
        self.defs
            .get(def)
            .and_then(|record| record.inputs.as_ref())
            .is_some_and(|inputs| !inputs.is_empty())
    }
}

/// A def's code with ids and spans dropped, as a hash, plus the globals it
/// names.
struct Shape {
    hash: u64,
    refs: Vec<String>,
}

/// Decides, def by def, whether a load reuses the previous value, and records
/// this load for the next one.
pub(crate) struct Planner<'p> {
    previous: Option<(&'p HashMap<String, Value>, &'p Records)>,
    shapes: HashMap<String, Shape>,
    positions: HashMap<String, usize>,
    types: HashMap<String, String>,
    unchanged: HashSet<String>,
    pub(crate) records: Records,
    pub(crate) stats: ReuseStats,
}

impl<'p> Planner<'p> {
    pub(crate) fn new(
        module: &Module,
        previous: Option<(&'p HashMap<String, Value>, &'p Records)>,
        types: HashMap<String, String>,
        units_pure: bool,
        host: &mut dyn Host,
    ) -> Planner<'p> {
        let (units, unit_refs) = unit_shape(module);
        let mut shapes: HashMap<String, Shape> = module
            .defs
            .iter()
            .map(|def| (def.name.clone(), shape_of(def)))
            .collect();
        // A unit operator naming a global can run inside any def.
        for shape in shapes.values_mut() {
            shape.refs.extend(unit_refs.iter().cloned());
        }
        let positions = module
            .defs
            .iter()
            .enumerate()
            .map(|(i, def)| (def.name.clone(), i))
            .collect();
        let mut planner = Planner {
            previous,
            shapes,
            positions,
            types,
            unchanged: HashSet::new(),
            records: Records {
                units,
                defs: HashMap::new(),
            },
            stats: ReuseStats::default(),
        };
        if units_pure {
            planner.unchanged = planner.find_unchanged(module, host);
        }
        planner
    }

    /// The defs whose value can't differ from last load's (see the module doc;
    /// the ordering rule is checked per def in [`Self::reuse`]).
    fn find_unchanged(&self, module: &Module, host: &mut dyn Host) -> HashSet<String> {
        let Some((_, previous)) = self.previous else {
            return HashSet::new();
        };
        if previous.units != self.records.units {
            return HashSet::new();
        }
        let same_record = |def: &Def| -> Option<Arc<[HostInput]>> {
            let record = previous.defs.get(&def.name)?;
            let ty = self.types.get(&def.name)?;
            (record.shape == self.shapes[&def.name].hash && record.ty.as_ref() == Some(ty))
                .then(|| record.inputs.clone())
                .flatten()
        };
        let mut candidates: HashMap<&str, Arc<[HostInput]>> = module
            .defs
            .iter()
            .filter_map(|def| Some((def.name.as_str(), same_record(def)?)))
            .collect();
        // Cheap checks first, then a dependency sweep, so host inputs (file
        // re-reads) are only checked for defs that can still be reused.
        self.drop_dependents(module, &mut candidates);
        // Each input once, however many defs read it (a re-read can be large).
        let mut current: HashMap<&HostInput, bool> = HashMap::new();
        let stale: Vec<&str> = candidates
            .iter()
            .filter(|(_, inputs)| {
                !inputs.iter().all(|input| {
                    *current
                        .entry(input)
                        .or_insert_with(|| host.inputs_current(std::slice::from_ref(input)))
                })
            })
            .map(|(name, _)| *name)
            .collect();
        for name in stale {
            candidates.remove(name);
        }
        self.drop_dependents(module, &mut candidates);
        candidates.into_keys().map(str::to_string).collect()
    }

    /// Remove every candidate that references, transitively, a def that isn't
    /// one.
    fn drop_dependents<'a, V>(&'a self, module: &'a Module, candidates: &mut HashMap<&'a str, V>) {
        let mut users: HashMap<&str, Vec<&str>> = HashMap::new();
        for def in &module.defs {
            for name in &self.shapes[&def.name].refs {
                users.entry(name.as_str()).or_default().push(def.name.as_str());
            }
        }
        let mut work: VecDeque<&str> = module
            .defs
            .iter()
            .map(|def| def.name.as_str())
            .filter(|name| !candidates.contains_key(name))
            .collect();
        while let Some(changed) = work.pop_front() {
            for user in users.get(changed).into_iter().flatten() {
                if candidates.remove(user).is_some() {
                    work.push_back(user);
                }
            }
        }
    }

    /// The previous value of `def` when it can be reused here.
    pub(crate) fn reuse(&mut self, def: &Def) -> Option<Value> {
        let value = self.reusable(def);
        if !matches!(def.value.kind, ExprKind::Lambda { .. }) {
            self.stats.values.push((def.span, value.is_some()));
        }
        value
    }

    fn reusable(&mut self, def: &Def) -> Option<Value> {
        let (globals, previous) = self.previous?;
        if matches!(def.value.kind, ExprKind::Lambda { .. }) || !self.unchanged.contains(&def.name) {
            return None;
        }
        let value = globals.get(&def.name)?;
        if !value.is_reload_safe_snapshot() || !self.same_order(&def.name, previous) {
            return None;
        }
        let mut record = previous.defs[&def.name].clone();
        record.position = self.positions[&def.name];
        self.records.defs.insert(def.name.clone(), record);
        Some(value.clone())
    }

    /// Whether every def `name` reaches sits on the same side of it in def
    /// order as last load. A def that reaches itself never counts.
    fn same_order(&self, name: &str, previous: &Records) -> bool {
        let here = |def: &str| Some((*self.positions.get(def)?, previous.defs.get(def)?.position));
        let Some((new, old)) = here(name) else {
            return false;
        };
        let mut seen = HashSet::new();
        let mut work = vec![name];
        while let Some(at) = work.pop() {
            let Some(shape) = self.shapes.get(at) else {
                return false;
            };
            for next in &shape.refs {
                if next == name {
                    return false;
                }
                if seen.insert(next.as_str()) {
                    match here(next) {
                        Some((next_new, next_old)) if (next_new < new) == (next_old < old) => {}
                        _ => return false,
                    }
                    work.push(next);
                }
            }
        }
        true
    }

    /// Record an evaluated `def`; `inputs` as in [`DefRecord`].
    pub(crate) fn record(&mut self, def: &Def, inputs: Option<Vec<HostInput>>) {
        let inputs = inputs.map(|mut inputs| {
            inputs.sort();
            inputs.dedup();
            Arc::from(inputs)
        });
        self.records.defs.insert(
            def.name.clone(),
            DefRecord {
                shape: self.shapes[&def.name].hash,
                ty: self.types.get(&def.name).cloned(),
                position: self.positions[&def.name],
                inputs,
            },
        );
    }
}

/// What a job (`Job.run`) needs from a module's code: which defs a function
/// reaches, and a hash of that code to tell whether a reload changed it.
pub struct CodeIndex {
    units: u64,
    unit_refs: Vec<String>,
    shapes: HashMap<String, Shape>,
    /// Def names in def order (the order a load evaluates them).
    order: Vec<String>,
}

impl CodeIndex {
    pub fn new(module: &Module) -> CodeIndex {
        let (units, unit_refs) = unit_shape(module);
        CodeIndex {
            units,
            unit_refs,
            shapes: module.defs.iter().map(|def| (def.name.clone(), shape_of(def))).collect(),
            order: module.defs.iter().map(|def| def.name.clone()).collect(),
        }
    }

    /// `name` and every def it reaches, transitively, with what the unit
    /// operators name (any def may dispatch one): what evaluating it needs.
    /// `None` if `name` isn't a def.
    pub fn deps_of(&self, name: &str) -> Option<HashSet<String>> {
        self.shapes.get(name)?;
        let mut seen: HashSet<String> = HashSet::new();
        let mut work: Vec<&str> = vec![name];
        work.extend(self.unit_refs.iter().map(String::as_str));
        while let Some(at) = work.pop() {
            let Some(shape) = self.shapes.get(at) else { continue };
            if seen.insert(at.to_string()) {
                work.extend(shape.refs.iter().map(String::as_str));
            }
        }
        Some(seen)
    }

    /// A hash of the code calling `name` runs: each dependency's code and
    /// type annotation in def order, plus the unit operators. Equal across
    /// two loads, the call computes the same from the same input and files.
    pub fn code_key(&self, name: &str) -> Option<u64> {
        let deps = self.deps_of(name)?;
        let mut hasher = DefaultHasher::new();
        self.units.hash(&mut hasher);
        for def in self.order.iter().filter(|def| deps.contains(*def)) {
            def.hash(&mut hasher);
            self.shapes[def].hash.hash(&mut hasher);
        }
        Some(hasher.finish())
    }
}

/// Each top-level def whose value is a lambda, by that lambda's node, so a
/// closure can be told to be one (`Job.run`'s function, called by name on a
/// worker).
pub struct Roots {
    /// Lambda node id → (def name, body address: guards a stale id).
    by_expr: HashMap<u32, (String, usize)>,
}

impl Roots {
    pub fn of(module: &Module) -> Roots {
        let by_expr = module
            .defs
            .iter()
            .filter_map(|def| match &def.value.kind {
                ExprKind::Lambda { body, .. } => {
                    Some((def.value.id.raw(), (def.name.clone(), Rc::as_ptr(body) as usize)))
                }
                _ => None,
            })
            .collect();
        Roots { by_expr }
    }

    /// The top-level def `value` is the function of (an alias names the def
    /// it aliases), or why it isn't one.
    pub fn name_of(&self, value: &Value) -> Result<&str, String> {
        match value {
            Value::Closure(closure) => self
                .by_expr
                .get(&closure.expr_id.raw())
                .filter(|(_, body)| Rc::as_ptr(&closure.body) as usize == *body)
                .map(|(name, _)| name.as_str())
                .ok_or_else(|| {
                    "a function made inside another one, which may capture its locals — \
make it a top-level `let`"
                        .to_string()
                }),
            Value::Partial(_) => Err("a partial application — pass the top-level function and \
put the arguments in the input"
                .to_string()),
            Value::Builtin(_) | Value::HostFn(_) | Value::Ctor { .. } => {
                Err("a built-in function — wrap it in a top-level `let`".to_string())
            }
            other => Err(format!("{}, not a function", other.kind_name())),
        }
    }
}

/// The hash of the module's units and unit operators (an operator dispatches
/// on the brand its unit's constructor builds), and the globals they name.
fn unit_shape(module: &Module) -> (u64, Vec<String>) {
    let mut walk = Walk::default();
    for unit in &module.units {
        unit.suffix.hash(&mut walk.hasher);
        walk.expr(&unit.target);
    }
    for op in &module.unit_ops {
        op.suffix.hash(&mut walk.hasher);
        std::mem::discriminant(&op.op).hash(&mut walk.hasher);
        op.module.hash(&mut walk.hasher);
        walk.expr(&op.target);
    }
    (walk.hasher.finish(), walk.refs.into_iter().collect())
}

fn shape_of(def: &Def) -> Shape {
    let mut walk = Walk::default();
    walk.ty(&def.ty);
    walk.expr(&def.value);
    Shape {
        hash: walk.hasher.finish(),
        refs: walk.refs.into_iter().collect(),
    }
}

/// Hashes code structurally. Local bindings hash by the order they're
/// introduced in, so renumbered ids (an edit earlier in the project) don't
/// count as a change.
#[derive(Default)]
struct Walk {
    hasher: DefaultHasher,
    bindings: HashMap<u32, u32>,
    refs: HashSet<String>,
}

impl Walk {
    fn tag(&mut self, tag: u8) {
        tag.hash(&mut self.hasher);
    }

    fn bind(&mut self, binding: crate::ir::BindingId) {
        let next = self.bindings.len() as u32;
        self.bindings.insert(binding.0, next);
    }

    fn local(&mut self, binding: crate::ir::BindingId, name: &str) {
        match self.bindings.get(&binding.0) {
            Some(index) => index.hash(&mut self.hasher),
            // Unreachable in lowered code (defs are closed); hash the name.
            None => name.hash(&mut self.hasher),
        }
    }

    fn ty(&mut self, ty: &Option<TypeName>) {
        fn go(walk: &mut Walk, ty: &TypeName) {
            ty.name.hash(&mut walk.hasher);
            ty.args.len().hash(&mut walk.hasher);
            for arg in &ty.args {
                go(walk, arg);
            }
        }
        match ty {
            Some(ty) => {
                self.tag(1);
                go(self, ty);
            }
            None => self.tag(0),
        }
    }

    fn exprs(&mut self, exprs: &[Expr]) {
        exprs.len().hash(&mut self.hasher);
        for expr in exprs {
            self.expr(expr);
        }
    }

    fn fields(&mut self, fields: &[crate::ir::Field]) {
        fields.len().hash(&mut self.hasher);
        for field in fields {
            field.name.hash(&mut self.hasher);
            self.expr(&field.value);
        }
    }

    fn expr(&mut self, expr: &Expr) {
        // Iterative along `else if` chains, which can be long.
        let mut expr = expr;
        loop {
            match &expr.kind {
                ExprKind::If {
                    cond,
                    then_branch,
                    else_branch,
                } => {
                    self.tag(0);
                    self.expr(cond);
                    self.expr(then_branch);
                    expr = else_branch;
                }
                _ => return self.expr_node(expr),
            }
        }
    }

    fn expr_node(&mut self, expr: &Expr) {
        match &expr.kind {
            ExprKind::Number(n) => {
                self.tag(1);
                n.to_bits().hash(&mut self.hasher);
            }
            ExprKind::String(s) => {
                self.tag(2);
                s.hash(&mut self.hasher);
            }
            ExprKind::InterpolatedString(parts) => {
                self.tag(3);
                parts.len().hash(&mut self.hasher);
                for part in parts {
                    match part {
                        StringPart::Text(text) => {
                            self.tag(0);
                            text.hash(&mut self.hasher);
                        }
                        StringPart::Expr(expr) => {
                            self.tag(1);
                            self.expr(expr);
                        }
                    }
                }
            }
            ExprKind::Bool(b) => {
                self.tag(4);
                b.hash(&mut self.hasher);
            }
            ExprKind::Local { binding, name } => {
                self.tag(5);
                self.local(*binding, name);
            }
            ExprKind::Global(name) => {
                self.tag(6);
                name.hash(&mut self.hasher);
                self.refs.insert(name.clone());
            }
            ExprKind::External(path) => {
                self.tag(7);
                path.hash(&mut self.hasher);
            }
            ExprKind::Record(fields) => {
                self.tag(8);
                self.fields(fields);
            }
            ExprKind::RecordUpdate { base, fields } => {
                self.tag(9);
                self.expr(base);
                self.fields(fields);
            }
            ExprKind::List(items) => {
                self.tag(10);
                self.exprs(items);
            }
            ExprKind::ListCons { items, tail } => {
                self.tag(11);
                self.exprs(items);
                self.expr(tail);
            }
            ExprKind::Tuple(items) => {
                self.tag(12);
                self.exprs(items);
            }
            ExprKind::LocalMut { binding, name } => {
                self.tag(13);
                self.local(*binding, name);
            }
            ExprKind::Let {
                binding,
                name: _,
                mutable,
                ty,
                value,
                body,
            } => {
                self.tag(14);
                mutable.hash(&mut self.hasher);
                self.ty(ty);
                self.expr(value);
                self.bind(*binding);
                self.expr(body);
            }
            ExprKind::Assign {
                binding,
                name,
                value,
                rest,
            } => {
                self.tag(15);
                self.local(*binding, name);
                self.expr(value);
                self.expr(rest);
            }
            ExprKind::FieldAccess { object, field } => {
                self.tag(16);
                self.expr(object);
                field.hash(&mut self.hasher);
            }
            ExprKind::Lambda { params, ret, body } => {
                self.tag(17);
                params.len().hash(&mut self.hasher);
                for param in params.iter() {
                    self.ty(&param.ty);
                    self.bind(param.binding);
                }
                self.ty(ret);
                self.expr(body);
            }
            ExprKind::Call { callee, args } => {
                self.tag(18);
                self.expr(callee);
                self.exprs(args);
            }
            ExprKind::Binary { op, lhs, rhs } => {
                self.tag(19);
                std::mem::discriminant(op).hash(&mut self.hasher);
                self.expr(lhs);
                self.expr(rhs);
            }
            ExprKind::Logical { op, lhs, rhs } => {
                self.tag(20);
                std::mem::discriminant(op).hash(&mut self.hasher);
                self.expr(lhs);
                self.expr(rhs);
            }
            ExprKind::Neg(inner) => {
                self.tag(21);
                self.expr(inner);
            }
            ExprKind::Not(inner) => {
                self.tag(22);
                self.expr(inner);
            }
            ExprKind::If { .. } => unreachable!("`Walk::expr` walks if chains"),
            ExprKind::Ctor { name, arity } => {
                self.tag(23);
                name.hash(&mut self.hasher);
                arity.hash(&mut self.hasher);
            }
            ExprKind::Match { scrutinee, arms } => {
                self.tag(24);
                self.expr(scrutinee);
                arms.len().hash(&mut self.hasher);
                for arm in arms {
                    self.pattern(&arm.pattern);
                    self.expr(&arm.body);
                }
            }
        }
    }

    fn pattern(&mut self, pattern: &Pattern) {
        match &pattern.kind {
            PatternKind::Wildcard => self.tag(0),
            PatternKind::Var { binding, name: _ } => {
                self.tag(1);
                self.bind(*binding);
            }
            PatternKind::Ctor { name, args } => {
                self.tag(2);
                name.hash(&mut self.hasher);
                self.patterns(args);
            }
            PatternKind::Tuple(items) => {
                self.tag(3);
                self.patterns(items);
            }
            PatternKind::List { items, tail } => {
                self.tag(4);
                self.patterns(items);
                match tail {
                    Some(tail) => {
                        self.tag(1);
                        self.pattern(tail);
                    }
                    None => self.tag(0),
                }
            }
            PatternKind::Number(n) => {
                self.tag(5);
                n.to_bits().hash(&mut self.hasher);
            }
            PatternKind::Bool(b) => {
                self.tag(6);
                b.hash(&mut self.hasher);
            }
            PatternKind::String(s) => {
                self.tag(7);
                s.hash(&mut self.hasher);
            }
        }
    }

    fn patterns(&mut self, patterns: &[Pattern]) {
        patterns.len().hash(&mut self.hasher);
        for pattern in patterns {
            self.pattern(pattern);
        }
    }
}
