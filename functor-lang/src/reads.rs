//! Read classes: the ambient host state an impure built-in reads when called.
//!
//! A `.funi` declares them above a host signature, e.g. `impure[file]` over
//! `let bytes : (Path.t) => Bytes.t`. User code is never annotated. The host
//! notes each read at run time, and that record is the authority (reload reuse
//! today; `Scene.memo` checks physics reads); the declarations are what docs
//! and hover show, and [`infer`] spreads them through user defs for hover.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;

use crate::ast::TypeName;
use crate::ir::{Expr, ExprKind, Module};
use crate::span::Span;
use crate::types::{ExprTypes, Type};
use crate::CheckError;

/// A registered read class. A `.funi` header naming anything else is a parse
/// error, so a typo can't declare a class nothing enforces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ReadClass {
    /// Project files and mounts (`File.bytes`): keyed by content hash.
    File,
    /// The stepped physics world (`Physics.position`).
    Physics,
    /// The frame's UI handler-slot table (`Ui.button`).
    Ui,
}

impl ReadClass {
    /// Every class, in declaration order.
    pub const ALL: [ReadClass; 3] = [ReadClass::File, ReadClass::Physics, ReadClass::Ui];

    /// The name a `.funi` header spells it with.
    pub fn name(self) -> &'static str {
        match self {
            ReadClass::File => "file",
            ReadClass::Physics => "physics",
            ReadClass::Ui => "ui",
        }
    }

    /// The class a header name registers, if any.
    pub fn from_name(name: &str) -> Option<ReadClass> {
        ReadClass::ALL
            .into_iter()
            .find(|class| class.name() == name)
    }
}

/// A `.funi`-style header for `classes`: `impure[file, physics]`.
pub fn header(classes: &[ReadClass]) -> String {
    let names: Vec<&str> = classes.iter().map(|class| class.name()).collect();
    format!("impure[{}]", names.join(", "))
}

impl fmt::Display for ReadClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Prelude host types whose values hold Functor Lang functions (taggers,
/// handlers): a value of one can run code that reads, so it never stops the
/// spread. Every other host type is plain data to [`infer`].
pub const CLOSURE_HOLDERS: [&str; 6] =
    ["Effect.t", "Sub.t", "Ui.view", "Html.node", "Attr.t", "Frame.t"];

/// Which of a def's read sets a step refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Set {
    /// While a value def evaluates, at load (`let level = World.load(…)`).
    Now,
    /// When a function def is called once: its body runs, but a lambda it
    /// only builds (a record field, a variant payload) doesn't.
    Call,
    /// The code a function def's result holds (`{ tick: (m) => … }`).
    Returns,
    /// Later, through code the def's value holds: a function's whole body, a
    /// value's stored lambdas. Empty for a value of data.
    Later,
}

/// Where a read comes from: a built-in, or a def's set that has it.
#[derive(Clone, Debug)]
enum Via {
    Builtin(String),
    Def(String, Set),
}

/// One step toward a read, and the span of the name that took it.
#[derive(Clone, Debug)]
struct Step {
    via: Via,
    span: Span,
}

type Reads = BTreeMap<ReadClass, Step>;

/// Each top-level def's inferred read classes ([`infer`]).
#[derive(Debug, Default)]
pub struct Inferred {
    sets: HashMap<(String, Set), Reads>,
}

impl Inferred {
    /// The classes `def` may read, at load or later, in [`ReadClass::ALL`]
    /// order.
    pub fn classes(&self, def: &str) -> Vec<ReadClass> {
        let mut classes: Vec<ReadClass> = [Set::Now, Set::Later]
            .iter()
            .filter_map(|&set| self.sets.get(&(def.to_string(), set)))
            .flat_map(|reads| reads.keys().copied())
            .collect();
        classes.sort();
        classes.dedup();
        classes
    }

    /// How `def` reaches `class`, def by def to the built-in:
    /// `["World.load", "File.bytes"]`. Empty when it doesn't.
    pub fn path(&self, def: &str, class: ReadClass) -> Vec<&str> {
        [Set::Now, Set::Later]
            .iter()
            .find_map(|&set| self.sets.get(&(def.to_string(), set))?.get(&class))
            .map(|step| self.follow(step, class))
            .unwrap_or_default()
    }

    /// The names from `step` to the built-in. Each step was learned from a
    /// set that already had the class, so the chain ends.
    fn follow<'a>(&'a self, step: &'a Step, class: ReadClass) -> Vec<&'a str> {
        let mut path = Vec::new();
        let mut at = Some(step);
        while let Some(step) = at {
            match &step.via {
                Via::Builtin(name) => {
                    path.push(name.as_str());
                    break;
                }
                Via::Def(name, set) => {
                    path.push(name.as_str());
                    at = self
                        .sets
                        .get(&(name.clone(), *set))
                        .and_then(|reads| reads.get(&class));
                }
            }
        }
        path
    }

    /// The classes a root's uses reach (an expect, a unit, a memo builder),
    /// each with its first step.
    fn resolve(&self, uses: &[Use]) -> Reads {
        let mut reads = Reads::new();
        for used in uses {
            let classes: Vec<ReadClass> = match &used.to {
                Target::Builtin(_, classes) => classes.to_vec(),
                Target::Def(name, set) => self
                    .sets
                    .get(&(name.to_string(), *set))
                    .map(|reads| reads.keys().copied().collect())
                    .unwrap_or_default(),
            };
            for class in classes {
                reads.entry(class).or_insert_with(|| used.step());
            }
        }
        reads
    }
}

/// Infer each top-level def's read classes from the `impure[…]` built-ins it
/// can reach. Diagnostics only (hover, `functor reads`, [`phase_errors`]):
/// the run-time `note_read` stays the authority, because `unknown` can hide
/// a function from this pass.
///
/// Sets per def: what a value reads while it evaluates at load, what a
/// function reads when called, and what code the def's value (or a
/// function's result) holds can read later. A use charges the set that runs: `f(x)` and `List.map(f, xs)`
/// run `f`'s body, `{ tick: f }` only holds it. A value of data (`float`,
/// records of data, `Scene.t`, …) holds no code, so using it reads nothing;
/// a function, `unknown` or [`CLOSURE_HOLDERS`] value can.
pub fn infer(module: &Module, types: &ExprTypes) -> Inferred {
    Context::of(module, types).solve()
}

/// What the walks share: built-in classes, which defs are functions, and
/// which types hold only data.
struct Context<'m> {
    module: &'m Module,
    types: &'m ExprTypes,
    builtins: HashMap<String, &'m [ReadClass]>,
    functions: HashSet<&'m str>,
    /// Each def's value, so `viewer.tick` resolves to that field's code.
    values: HashMap<&'m str, &'m Expr>,
    plain: PlainTypes,
}

impl<'m> Context<'m> {
    fn of(module: &'m Module, types: &'m ExprTypes) -> Context<'m> {
        Context {
            module,
            types,
            builtins: module
                .signatures
                .iter()
                .filter(|sig| !sig.reads.is_empty())
                .map(|sig| (sig.name.clone(), sig.reads.as_slice()))
                .collect(),
            functions: module
                .defs
                .iter()
                .filter(|def| matches!(def.value.kind, ExprKind::Lambda { .. }))
                .map(|def| def.name.as_str())
                .collect(),
            values: module
                .defs
                .iter()
                .map(|def| (def.name.as_str(), &def.value))
                .collect(),
            plain: PlainTypes::of(module),
        }
    }

    fn walker(&self) -> Walker<'_, 'm> {
        Walker {
            cx: self,
            lets: HashMap::new(),
            ran: HashSet::new(),
            kept: HashSet::new(),
            uses: Vec::new(),
        }
    }

    /// Each def's sets, spread to a fixed point (sets only grow).
    fn solve(&self) -> Inferred {
        let mut edges: Vec<((String, Set), Vec<Use<'m>>)> = Vec::new();
        for def in &self.module.defs {
            let name = def.name.clone();
            if let ExprKind::Lambda { body, .. } = &def.value.kind {
                let mut call = self.walker();
                call.now(body, false);
                edges.push(((name.clone(), Set::Call), call.uses));
                let mut returns = self.walker();
                returns.held(body);
                edges.push(((name.clone(), Set::Returns), returns.uses));
                let mut later = self.walker();
                later.code(&def.value);
                edges.push(((name, Set::Later), later.uses));
            } else {
                let mut now = self.walker();
                now.now(&def.value, false);
                edges.push(((name.clone(), Set::Now), now.uses));
                let mut later = self.walker();
                later.held(&def.value);
                edges.push(((name.clone(), Set::Later), later.uses));
                // Each field of a record literal, so calling one field
                // doesn't charge its siblings' code.
                if let ExprKind::Record(fields) = &def.value.kind {
                    for field in fields {
                        let mut held = self.walker();
                        held.held(&field.value);
                        edges.push(((format!("{name}.{}", field.name), Set::Later), held.uses));
                    }
                }
            }
        }
        let mut inferred = Inferred::default();
        for (key, uses) in &edges {
            let reads = inferred.sets.entry(key.clone()).or_default();
            for used in uses {
                if let Target::Builtin(_, classes) = &used.to {
                    for &class in *classes {
                        reads.entry(class).or_insert_with(|| used.step());
                    }
                }
            }
        }
        let mut changed = true;
        while changed {
            changed = false;
            for (key, uses) in &edges {
                for used in uses {
                    let Target::Def(name, set) = &used.to else {
                        continue;
                    };
                    let from = (name.to_string(), *set);
                    let gained: Vec<ReadClass> = inferred
                        .sets
                        .get(&from)
                        .into_iter()
                        .flat_map(|reads| reads.keys().copied())
                        .filter(|class| !inferred.sets[key].contains_key(class))
                        .collect();
                    let reads = inferred.sets.get_mut(key).expect("every set has an entry");
                    for class in gained {
                        reads.insert(class, used.step());
                        changed = true;
                    }
                }
            }
        }
        inferred.sets.retain(|_, reads| !reads.is_empty());
        inferred
    }
}

/// A name a walk found: a built-in with classes, or one of a def's sets.
#[derive(Clone, Debug)]
enum Target<'m> {
    Builtin(String, &'m [ReadClass]),
    /// A def's set; `viewer.tick` for one field of a record-literal value.
    Def(String, Set),
}

#[derive(Clone, Debug)]
struct Use<'m> {
    to: Target<'m>,
    span: Span,
}

impl Use<'_> {
    fn step(&self) -> Step {
        let via = match &self.to {
            Target::Builtin(name, _) => Via::Builtin(name.clone()),
            Target::Def(name, set) => Via::Def(name.to_string(), *set),
        };
        Step {
            via,
            span: self.span,
        }
    }
}

/// Collects the uses of one expression, under one of three readings.
struct Walker<'c, 'm> {
    cx: &'c Context<'m>,
    /// `let` values in scope, so a local lambda counts where it is called.
    lets: HashMap<u32, &'m Expr>,
    /// `let` bindings already run, so a local called twice is walked once.
    ran: HashSet<u32>,
    /// `let` bindings whose held code is already collected.
    kept: HashSet<u32>,
    uses: Vec<Use<'m>>,
}

impl<'m> Walker<'_, 'm> {
    fn builtin(&mut self, path: &[String], span: Span) {
        let path = path.join(".");
        if let Some(&classes) = self.cx.builtins.get(&path) {
            self.uses.push(Use {
                to: Target::Builtin(path, classes),
                span,
            });
        }
    }

    fn def(&mut self, name: &str, set: Set, span: Span) {
        self.uses.push(Use {
            to: Target::Def(name.to_string(), set),
            span,
        });
    }

    /// The set that runs when `name`'s value is called: a function's body,
    /// or whatever code a value holds.
    fn called(&self, name: &str) -> Set {
        if self.cx.functions.contains(name) {
            Set::Call
        } else {
            Set::Later
        }
    }

    /// `expr` evaluates now. `runs` when its value may also be called now:
    /// a callee, or a call argument (`List.map((p) => File.bytes(p), ps)`).
    /// A lambda anywhere else (a record field, a variant payload, a `let`
    /// value) is only built, and so is a value of data type (`viewer.size`).
    fn now(&mut self, expr: &'m Expr, runs: bool) {
        let runs = runs && self.callable(expr);
        match &expr.kind {
            ExprKind::External(path) => {
                if runs {
                    self.builtin(path, expr.span);
                }
            }
            ExprKind::Global(name) => {
                if runs {
                    self.def(name, self.called(name), expr.span);
                }
            }
            ExprKind::Local { binding, .. } => {
                if let Some(value) = self.lets.get(&binding.0).copied() {
                    if runs && self.ran.insert(binding.0) {
                        self.now(value, true);
                    }
                }
            }
            ExprKind::Lambda { body, .. } => {
                if runs {
                    self.now(body, false);
                }
            }
            ExprKind::Call { callee, args } => {
                // A variant's payload is stored, not called; so is an
                // argument to a call whose result holds code (`Physics.events(
                // tagger)`, `probe("x", () => …)`), as far as we can tell.
                let ctor = matches!(callee.kind, ExprKind::Ctor { .. });
                let stores = ctor || self.holds_code(expr);
                self.now(callee, true);
                for arg in args {
                    self.now(arg, !stores);
                }
                // `make(a)(b)` runs code `make` returned.
                if runs && !ctor {
                    self.held_root(callee, expr.span);
                }
            }
            ExprKind::Let {
                binding,
                value,
                body,
                ..
            } => {
                self.now(value, false);
                self.lets.insert(binding.0, value);
                self.now(body, runs);
            }
            ExprKind::Assign { value, rest, .. } => {
                self.now(value, false);
                self.now(rest, runs);
            }
            ExprKind::If {
                cond,
                then_branch,
                else_branch,
            } => {
                self.now(cond, false);
                self.now(then_branch, runs);
                self.now(else_branch, runs);
            }
            ExprKind::Match { scrutinee, arms } => {
                self.now(scrutinee, false);
                for arm in arms {
                    self.now(&arm.body, runs);
                }
            }
            _ => {
                // `api.has(p)`: a field of a def's value may be its code.
                if runs {
                    self.held_root(expr, expr.span);
                }
                crate::rebind::each_child(expr, &mut |child| self.now(child, false));
            }
        }
    }

    /// The code the def at the root of `expr` holds (`api` in `api.has`,
    /// `make`'s result in `make(a)`), which calling `expr`'s value may run.
    fn held_root(&mut self, mut expr: &'m Expr, span: Span) {
        // `viewer.tick` on a record literal: just that field's code.
        if let ExprKind::FieldAccess { object, field } = &expr.kind {
            if let ExprKind::Global(name) = &object.kind {
                // Through aliases (`let alias = api`) to the literal.
                let mut name = name.as_str();
                for _ in 0..8 {
                    match self.cx.values.get(name).map(|value| &value.kind) {
                        Some(ExprKind::Global(next)) => name = next,
                        _ => break,
                    }
                }
                if let Some(ExprKind::Record(fields)) =
                    self.cx.values.get(name).map(|value| &value.kind)
                {
                    if fields.iter().any(|f| &f.name == field) {
                        self.def(&format!("{name}.{field}"), Set::Later, span);
                    }
                    return;
                }
            }
        }
        let mut called = false;
        loop {
            match &expr.kind {
                ExprKind::FieldAccess { object, .. } => expr = object,
                ExprKind::Call { callee, .. } => {
                    called = true;
                    expr = callee;
                }
                ExprKind::Global(name) => return self.def(name, self.result(name, called), span),
                _ => return,
            }
        }
    }

    /// The set holding `name`'s code: its result's when `called`.
    fn result(&self, name: &str, called: bool) -> Set {
        if called && self.cx.functions.contains(name) {
            Set::Returns
        } else {
            Set::Later
        }
    }

    /// Whether `expr`'s value may be a function (or `unknown`, or generic).
    fn callable(&self, expr: &Expr) -> bool {
        !matches!(
            self.cx.types.expr(expr.id),
            Some(Type::Float | Type::String | Type::Bool | Type::List(_) | Type::Map(..))
                | Some(Type::Tuple(_) | Type::Record(..) | Type::Variant(..))
        )
    }

    /// Whether `expr`'s value may hold code: a function, `unknown`, or a
    /// type that holds one.
    fn holds_code(&self, expr: &Expr) -> bool {
        self.cx
            .types
            .expr(expr.id)
            .is_some_and(|ty| !self.cx.plain.holds(ty))
    }

    /// The code a lambda holds: what its body reads when called, and the
    /// code its result holds.
    fn code(&mut self, lambda: &'m Expr) {
        if let ExprKind::Lambda { body, .. } = &lambda.kind {
            self.now(body, false);
            self.held(body);
        }
    }

    /// The code an evaluated value holds: its lambdas, stored functions and
    /// what the calls building it return. A part of data type (`mesh:
    /// loadMesh(p)`) holds none.
    fn held(&mut self, expr: &'m Expr) {
        if self
            .cx
            .types
            .expr(expr.id)
            .is_some_and(|ty| self.cx.plain.holds(ty))
        {
            return;
        }
        match &expr.kind {
            ExprKind::Lambda { .. } => self.code(expr),
            ExprKind::External(path) => self.builtin(path, expr.span),
            ExprKind::Global(name) => self.def(name, Set::Later, expr.span),
            ExprKind::Call { callee, args } => {
                // A built-in's read ran while the value was built.
                match &callee.kind {
                    ExprKind::Global(name) => self.def(name, self.result(name, true), callee.span),
                    ExprKind::External(_) => {}
                    // A local factory: only the code its result holds.
                    ExprKind::Local { binding, .. } => {
                        match self.lets.get(&binding.0).map(|value| &value.kind) {
                            Some(ExprKind::Lambda { body, .. }) => self.held(body),
                            _ => self.held(callee),
                        }
                    }
                    _ => self.held(callee),
                }
                // `Job.run`'s function runs on a worker, not where its
                // effect lands ([`phase_errors`] checks it on its own).
                let job = matches!(&callee.kind, ExprKind::External(path) if path == &["Job", "run"]);
                for (i, arg) in args.iter().enumerate() {
                    if !(job && i == 1) {
                        self.held(arg);
                    }
                }
            }
            // Only what the result keeps: a local used just to build it
            // (`let load = … in { size: load() }`) holds nothing.
            ExprKind::Let {
                binding,
                value,
                body,
                ..
            } => {
                self.lets.insert(binding.0, value);
                self.held(body);
            }
            ExprKind::Local { binding, .. } => {
                if let Some(value) = self.lets.get(&binding.0).copied() {
                    if self.kept.insert(binding.0) {
                        self.held(value);
                    }
                }
            }
            ExprKind::Assign { rest, .. } => self.held(rest),
            // The arms' results; the scrutinee only if an arm can keep a part.
            ExprKind::Match { scrutinee, arms } => {
                if arms.iter().any(keeps_part) {
                    self.held(scrutinee);
                }
                for arm in arms {
                    self.held(&arm.body);
                }
            }
            ExprKind::FieldAccess { .. } => self.held_root(expr, expr.span),
            _ => crate::rebind::each_child(expr, &mut |child| self.held(child)),
        }
    }
}

/// Whether a match arm's body uses a name its pattern binds, so its result
/// can keep part of the scrutinee (`| Boxed(read) => read`).
fn keeps_part(arm: &crate::ir::MatchArm) -> bool {
    use crate::ir::PatternKind;
    fn bound(pattern: &crate::ir::Pattern, out: &mut HashSet<u32>) {
        match &pattern.kind {
            PatternKind::Var { binding, .. } => {
                out.insert(binding.0);
            }
            PatternKind::Ctor { args: items, .. } | PatternKind::Tuple(items) => {
                items.iter().for_each(|item| bound(item, out))
            }
            PatternKind::List { items, tail } => {
                items.iter().for_each(|item| bound(item, out));
                if let Some(tail) = tail {
                    bound(tail, out);
                }
            }
            _ => {}
        }
    }
    fn uses(expr: &Expr, names: &HashSet<u32>) -> bool {
        if let ExprKind::Local { binding, .. } = &expr.kind {
            return names.contains(&binding.0);
        }
        let mut found = false;
        crate::rebind::each_child(expr, &mut |child| found = found || uses(child, names));
        found
    }
    let mut names = HashSet::new();
    bound(&arm.pattern, &mut names);
    !names.is_empty() && uses(&arm.body, &names)
}

/// A role's entry bindings, which [`phase_errors`] keys on.
pub struct Entry<'a> {
    /// Called by the runtime each frame or event (`tick`, `draw`, `ui`, …).
    pub hooks: &'a [&'a str],
    /// The first model, evaluated at load.
    pub init: &'a str,
    /// Read before anything else runs.
    pub config: &'a str,
    /// Draws the loading screen while the top-level values load.
    pub loading: &'a str,
}

/// Where each read class may run, as check errors:
///
/// - `file` only at load: top-level values and `expect`s. A hook (and
///   anything it calls) can't reach one; a file read there isn't pinned to
///   the load's snapshot.
/// - `physics` and `ui` only in hooks: no world is stepped at load, and UI
///   handler slots belong to one frame.
/// - `init`, `config`, `loading`, unit constructors and operators, and
///   `Scene.memo` / `Physics.memo` builders read nothing.
/// - A `Job.run` function reads like a top-level value: files, not
///   `physics` or `ui`.
///
/// Each error names the path to the built-in (`tick → loadClip →
/// File.bytes`). Inference can't see through `unknown`; the run-time checks
/// stay.
pub fn phase_errors(module: &Module, types: &ExprTypes, entry: &Entry) -> Vec<CheckError> {
    let cx = Context::of(module, types);
    let inferred = cx.solve();
    let mut errors: Vec<CheckError> = Vec::new();
    let mut seen: HashSet<(usize, ReadClass)> = HashSet::new();
    let mut report = |span: Span, class: ReadClass, message: String| {
        // One read can break two rules (a file read in a hook's memo
        // builder); the first, more specific, error is enough.
        if seen.insert((span.start, class)) {
            errors.push(CheckError { message, span });
        }
    };
    let path = |name: &str, step: &Step, class: ReadClass| {
        let mut path = vec![name];
        path.extend(inferred.follow(step, class));
        path.join(" → ")
    };

    for (callee, args) in memo_calls(module) {
        let Some(builder) = args.get(2) else { continue };
        let mut walk = cx.walker();
        walk.now(builder, true);
        for (class, step) in inferred.resolve(&walk.uses) {
            let fix = match class {
                ReadClass::File => {
                    "read the file in a top-level value and use that value in the builder"
                }
                ReadClass::Physics => {
                    "make that read outside the memo and pass the values the builder needs in deps"
                }
                ReadClass::Ui => "build UI widgets outside the memo",
            };
            report(
                step.span,
                class,
                format!(
                    "a {callee} builder can't read {} ({}): deps can't capture it, so the \
cached result would freeze — {fix}",
                    what(class),
                    path("builder", &step, class)
                ),
            );
        }
    }

    // A job's function runs off the frame, like a top-level value: files
    // yes, live state no.
    for (_, args) in calls_to(module, &["Job.run"]) {
        let Some(f) = args.get(1) else { continue };
        let mut walk = cx.walker();
        walk.now(f, true);
        for (class, step) in inferred.resolve(&walk.uses) {
            if class == ReadClass::File {
                continue;
            }
            report(
                step.span,
                class,
                format!(
                    "a `Job.run` function can't read {} ({}): it runs on a worker, off the \
frame — read it in a hook and put it in the input",
                    what(class),
                    path("job", &step, class)
                ),
            );
        }
    }

    for unit in &module.units {
        let mut walk = cx.walker();
        walk.now(&unit.target, true);
        for (class, step) in inferred.resolve(&walk.uses) {
            report(
                step.span,
                class,
                format!(
                    "the `{}` unit can't read {} ({}): it runs wherever a `1{}` literal \
appears, so it must be pure",
                    unit.suffix,
                    what(class),
                    path(&unit.suffix, &step, class),
                    unit.suffix
                ),
            );
        }
    }
    for op in &module.unit_ops {
        let mut walk = cx.walker();
        walk.now(&op.target, true);
        let op_name = format!("({})", op.op.symbol());
        for (class, step) in inferred.resolve(&walk.uses) {
            report(
                step.span,
                class,
                format!(
                    "the `{}` unit's {op_name} can't read {} ({}): it runs wherever {op_name} \
meets a `{}` value, so it must be pure",
                    op.suffix,
                    what(class),
                    path(&op_name, &step, class),
                    op.suffix
                ),
            );
        }
    }

    let loading_deps = crate::reuse::CodeIndex::new(module)
        .deps_of(entry.loading)
        .unwrap_or_default();
    for def in &module.defs {
        let name = def.name.as_str();
        let at = |set: Set| inferred.sets.get(&(def.name.clone(), set));
        let now = if cx.functions.contains(name) {
            // `init`/`config` as functions run once, at load.
            if name == entry.init || name == entry.config {
                at(Set::Call)
            } else {
                None
            }
        } else {
            at(Set::Now)
        };
        for (&class, step) in now.into_iter().flatten() {
            let message = match class {
                ReadClass::File if name == entry.config => format!(
                    "`{name}` can't read project files ({}): it's read first, to learn which \
paths are external — read them in a top-level value `{name}` doesn't use",
                    path(name, step, class)
                ),
                ReadClass::File if name == entry.init => format!(
                    "`{name}` can't read project files ({}): read them in a top-level value \
(`let level = …`) and use that value in `{name}`",
                    path(name, step, class)
                ),
                ReadClass::File => continue,
                ReadClass::Physics => format!(
                    "`{name}` reads the live physics world at load ({}), but no world has \
been stepped yet — read it in a hook (`tick`, `draw`, …), or compute it from data",
                    path(name, step, class)
                ),
                ReadClass::Ui => format!(
                    "`{name}` builds UI handlers at load ({}), but their slots belong to one \
frame — build the view in `ui`",
                    path(name, step, class)
                ),
            };
            report(step.span, class, message);
        }
        // `loading` draws before the load it shows has finished, so neither
        // it nor a value it uses may read.
        if name != entry.loading && !cx.functions.contains(name) && loading_deps.contains(name) {
            for (&class, step) in at(Set::Now).into_iter().flatten() {
                report(
                    step.span,
                    class,
                    format!(
                        "`{}` uses `{name}`, which reads {} at load ({}): the loading screen \
draws before the top-level values have loaded — draw from its `progress`, `tts` and plain values",
                        entry.loading,
                        what(class),
                        path(name, step, class)
                    ),
                );
            }
        }
        if name == entry.loading {
            for (&class, step) in at(Set::Later).into_iter().flatten() {
                report(
                    step.span,
                    class,
                    format!(
                        "`{name}` can't read {} ({}): it draws the loading screen before the \
top-level values have loaded — draw from its `progress`, `tts` and plain values",
                        what(class),
                        path(name, step, class)
                    ),
                );
            }
        }
        if entry.hooks.contains(&name) {
            if let Some(step) = at(Set::Later).and_then(|reads| reads.get(&ReadClass::File)) {
                report(
                    step.span,
                    ReadClass::File,
                    format!(
                        "`{name}` reads project files ({}), but files are read only at load, \
so the answer isn't pinned for replay — read it in a top-level value and use that value \
in `{name}`, or ask for it with `File.load` (or compute with it in `Job.run`): the result \
arrives through `update`",
                        path(name, step, ReadClass::File)
                    ),
                );
            }
        }
    }

    for expect in &module.expects {
        let mut walk = cx.walker();
        walk.now(&expect.expr, false);
        for (class, step) in inferred.resolve(&walk.uses) {
            if class == ReadClass::File {
                continue;
            }
            report(
                step.span,
                class,
                format!(
                    "this `expect` can't read {} ({}): expects run at load, outside any frame",
                    what(class),
                    path("expect", &step, class)
                ),
            );
        }
    }
    errors.sort_by_key(|error| error.span.start);
    errors
}

/// A class's state, for errors: "project files".
fn what(class: ReadClass) -> &'static str {
    match class {
        ReadClass::File => "project files",
        ReadClass::Physics => "the live physics world",
        ReadClass::Ui => "UI handler slots",
    }
}

/// Every `Scene.memo` / `Physics.memo` call in the module, with its args.
fn memo_calls(module: &Module) -> Vec<(String, &[Expr])> {
    calls_to(module, &["Scene.memo", "Physics.memo"])
}

/// Every call of a built-in in `paths`, with its arguments.
fn calls_to<'m>(module: &'m Module, paths: &[&str]) -> Vec<(String, &'m [Expr])> {
    let mut calls = Vec::new();
    let mut work: Vec<&Expr> = module
        .defs
        .iter()
        .map(|def| &def.value)
        .chain(module.expects.iter().map(|expect| &expect.expr))
        .collect();
    while let Some(expr) = work.pop() {
        if let ExprKind::Call { callee, args } = &expr.kind {
            if let ExprKind::External(path) = &callee.kind {
                let path = path.join(".");
                if paths.contains(&path.as_str()) {
                    calls.push((path, args.as_slice()));
                }
            }
        }
        crate::rebind::each_child(expr, &mut |child| work.push(child));
    }
    calls
}

/// Which declared types hold only data: no function, `unknown` or
/// [`CLOSURE_HOLDERS`] field, nominal fields followed
/// (recursive types included).
struct PlainTypes {
    plain: HashMap<String, bool>,
}

impl PlainTypes {
    fn of(module: &Module) -> PlainTypes {
        let mut plain: HashMap<String, bool> = module
            .types
            .iter()
            .map(|decl| (decl.name.clone(), !CLOSURE_HOLDERS.contains(&decl.name.as_str())))
            .collect();
        // Greatest fixed point: assume plain, demote until nothing changes,
        // so a recursive type is plain unless some field isn't.
        let mut changed = true;
        while changed {
            changed = false;
            for decl in &module.types {
                if !plain[&decl.name] {
                    continue;
                }
                let fields = crate::goto::type_body_fields(&decl.body);
                if !fields.iter().all(|ty| name_is_plain(ty, &plain)) {
                    plain.insert(decl.name.clone(), false);
                    changed = true;
                }
            }
        }
        PlainTypes { plain }
    }

    /// Whether a value of `ty` holds only data (so using it runs no code).
    fn holds(&self, ty: &Type) -> bool {
        match ty {
            Type::Float | Type::String | Type::Bool => true,
            Type::List(elem) => self.holds(elem),
            Type::Map(key, value) => self.holds(key) && self.holds(value),
            Type::Tuple(elems) => elems.iter().all(|elem| self.holds(elem)),
            Type::Record(name, args) | Type::Variant(name, args) => {
                self.plain.get(name) == Some(&true) && args.iter().all(|arg| self.holds(arg))
            }
            // A variable still free in a checked top-level value is
            // unconstrained (`Ok(1.0) : Result.t<float, 'e>`, `[] : List<'a>`),
            // so nothing but `unknown` can put a function there.
            Type::Var(_) => true,
            Type::Fn(..) | Type::Unknown => false,
        }
    }
}

/// [`PlainTypes::holds`] for a declaration's field annotation. A type
/// parameter counts as plain here; the use site's argument is checked.
fn name_is_plain(ty: &TypeName, plain: &HashMap<String, bool>) -> bool {
    let args = || ty.args.iter().all(|arg| name_is_plain(arg, plain));
    match ty.name.as_str() {
        "float" | "string" | "bool" => true,
        "List" | "Map" | "*" => args(),
        "=>" | "unknown" => false,
        name if name.starts_with('\'') => true,
        name => plain.get(name) == Some(&true) && args(),
    }
}

/// The inferred classes of each def in the project's own files (bundled
/// modules left out), one line each in def order:
///
/// ```text
/// level : impure[file] — file: World.load → File.bytes
/// tick : pure
/// ```
pub fn report(project: &crate::project::Project, inferred: &Inferred) -> String {
    let mut out = String::new();
    for def in &project.module.defs {
        let file = project.sources.file_at(def.span.start);
        if file.path.to_string_lossy().starts_with('<') {
            continue;
        }
        let classes = inferred.classes(&def.name);
        if classes.is_empty() {
            out.push_str(&format!("{} : pure\n", def.name));
            continue;
        }
        let paths: Vec<String> = classes
            .iter()
            .map(|&class| format!("{class}: {}", inferred.path(&def.name, class).join(" → ")))
            .collect();
        out.push_str(&format!(
            "{} : {} — {}\n",
            def.name,
            header(&classes),
            paths.join("; ")
        ));
    }
    out
}
