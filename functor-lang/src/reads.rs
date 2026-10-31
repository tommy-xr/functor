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
use crate::types::{ExprTypes, Type};

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

/// Where a def's read of one class comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Via {
    /// An `impure[…]` built-in the def names (`File.bytes`).
    Builtin(String),
    /// A def it names that reaches the class (`World.load`).
    Def(String),
}

/// Each top-level def's inferred read classes ([`infer`]).
#[derive(Debug, Default)]
pub struct Inferred {
    defs: HashMap<String, BTreeMap<ReadClass, Via>>,
}

impl Inferred {
    /// The classes `def` may read, in [`ReadClass::ALL`] order.
    pub fn classes(&self, def: &str) -> Vec<ReadClass> {
        self.defs
            .get(def)
            .map(|reads| reads.keys().copied().collect())
            .unwrap_or_default()
    }

    /// How `def` reaches `class`, def by def to the built-in:
    /// `["World.load", "File.bytes"]`. Empty when it doesn't.
    pub fn path(&self, def: &str, class: ReadClass) -> Vec<&str> {
        let mut path = Vec::new();
        let mut at = def;
        // Each step was learned from a def that already had the class, so
        // the chain ends at a built-in.
        while let Some(via) = self.defs.get(at).and_then(|reads| reads.get(&class)) {
            match via {
                Via::Builtin(name) => {
                    path.push(name.as_str());
                    break;
                }
                Via::Def(name) => {
                    path.push(name.as_str());
                    at = name;
                }
            }
        }
        path
    }
}

/// Infer each top-level def's read classes from the `impure[…]` built-ins it
/// can reach. Report-only (hover, docs, dumps): the run-time `note_read` stays
/// the authority, because `unknown` can hide a function from this pass.
///
/// A def reaches the classes of every built-in it names anywhere (lambdas
/// included, so a function passed along is charged where it's named), plus
/// those of every def it names — unless that def's checked type is
/// closure-free (`float`, records of data, `Scene.t`, …): such a value is
/// already computed, and using it runs no code. A function, `unknown` or
/// [`CLOSURE_HOLDERS`] value keeps spreading.
pub fn infer(module: &Module, types: &ExprTypes) -> Inferred {
    let builtins: HashMap<&str, &[ReadClass]> = module
        .signatures
        .iter()
        .filter(|sig| !sig.reads.is_empty())
        .map(|sig| (sig.name.as_str(), sig.reads.as_slice()))
        .collect();
    let plain = PlainTypes::of(module);
    let mut defs: HashMap<String, BTreeMap<ReadClass, Via>> = HashMap::new();
    let mut uses: Vec<(&str, Vec<&str>)> = Vec::new();
    let mut spreads: HashSet<&str> = HashSet::new();
    for def in &module.defs {
        let names = Names::of(&def.value);
        let reads = defs.entry(def.name.clone()).or_default();
        for path in &names.externals {
            for &class in builtins.get(path.as_str()).copied().unwrap_or_default() {
                reads.entry(class).or_insert_with(|| Via::Builtin(path.clone()));
            }
        }
        uses.push((def.name.as_str(), names.globals));
        if !types.expr(def.value.id).is_some_and(|ty| plain.holds(ty)) {
            spreads.insert(def.name.as_str());
        }
    }
    // Spread to a fixed point: sets only grow, and there are few classes.
    let mut changed = true;
    while changed {
        changed = false;
        for (def, used) in &uses {
            for &used in used.iter().filter(|used| spreads.contains(*used)) {
                let gained: Vec<ReadClass> = defs
                    .get(used)
                    .into_iter()
                    .flat_map(|reads| reads.keys().copied())
                    .filter(|class| !defs[*def].contains_key(class))
                    .collect();
                let reads = defs.get_mut(*def).expect("every def has an entry");
                for class in gained {
                    reads.insert(class, Via::Def(used.to_string()));
                    changed = true;
                }
            }
        }
    }
    defs.retain(|_, reads| !reads.is_empty());
    Inferred { defs }
}

/// The externals and globals an expression names, each once, in source
/// order. A worklist, so a deep spine can't overflow the stack.
#[derive(Default)]
struct Names<'m> {
    externals: Vec<String>,
    globals: Vec<&'m str>,
}

impl<'m> Names<'m> {
    fn of(expr: &'m Expr) -> Names<'m> {
        let mut names = Names::default();
        let mut work = vec![expr];
        while let Some(expr) = work.pop() {
            match &expr.kind {
                ExprKind::External(path) => {
                    let path = path.join(".");
                    if !names.externals.contains(&path) {
                        names.externals.push(path);
                    }
                }
                ExprKind::Global(name) if !names.globals.contains(&name.as_str()) => {
                    names.globals.push(name);
                }
                _ => {}
            }
            // Reversed onto the stack, so names come out in source order.
            let next = work.len();
            crate::rebind::each_child(expr, &mut |child| work.push(child));
            work[next..].reverse();
        }
        names
    }
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
