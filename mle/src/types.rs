//! Typechecking over the core IR — B4's gradual checker upgraded to real
//! **Hindley–Milner inference** (Track B7 of `docs/mle.md`).
//!
//! Unannotated code gets genuine types: fresh inference variables, solved
//! by unification, with **let-polymorphism** — each top-level def
//! generalizes as its dependency group finishes (strongly-connected
//! components of the call graph, so mutual recursion is monomorphic inside
//! its group and forward references still work), and every use
//! instantiates fresh (`id` at Float and String in one module is fine).
//! Builtins carry generic schemes (`List.map : (List<'a>, ('a) => 'b) =>
//! List<'b>`), so element types flow through pipelines. Lowercase names in
//! annotations are scoped type variables (`(xs: List<a>, f: (a) => b)`).
//!
//! **Gradualness survives at the seams**: [`Type::Unknown`] remains for
//! host values and unrecognized UPPERCASE type names, absorbs anything in
//! unification, and never binds a variable — dynamic where the world is
//! dynamic, inferred everywhere else. Record literals resolve NOMINALLY,
//! F#-style: the unique declared type with exactly that field set (no
//! match → anonymous data, still gradual; several → ambiguity error
//! asking for an annotation).
//!
//! ## The type language
//!
//! - Primitives `Float`, `String`, `Bool` (numbers are all Float).
//! - Declared record types (`type Position = { x: Float, y: Float }`) —
//!   nominal, by name.
//! - Declared variant types (`type Shape = | Circle(radius: Float) | Point`)
//!   — nominal, by name, like records.
//! - `List<T>`.
//! - Function types, from lambda annotations
//!   (`(a: Float, b: Float): Float => …`); an unannotated return type is the
//!   body's type when that is known (inferred in a single quiet enrichment
//!   pass — a *chain* of unannotated-return functions may stay Unknown).
//!
//! Note that nominality exists **only in annotations**: the runtime's record
//! equality and field access are structural (`value_eq` has no type tags), so
//! nominal diagnostics catch annotation-level intent, not runtime crashes.
//!
//! ## What is checked
//!
//! Arithmetic/comparison/unary-minus operand types; `==` across known types
//! that *cannot* be equal at runtime (different primitive/list/function
//! kinds, or record types whose declared field shapes differ — equality is
//! structural, so same-shaped nominal types may legitimately compare) and on
//! two functions (always a runtime error); record literals against a declared
//! record type where one is expected (a return annotation, an argument of a
//! call with a known signature) and against any *non*-record expectation (a
//! record literal is never a Float); field access on a known record type;
//! call arity and argument types where the callee's function type is known —
//! including the builtins, whose generic schemes live in
//! [`builtin_signature`] and instantiate fresh at every use (element types
//! flow through `List.map`); return annotations against the body's type;
//! and type-argument arity (`Position<Float>` is an error, an *unknown* type
//! name is not). Constructors carry function types from their declarations
//! (`Circle : (Float) => Shape`; nullary constructors are the variant type
//! itself), so construction checks like any call. `match` checks pattern
//! compatibility against a known scrutinee type (a foreign constructor, a
//! literal of the wrong type), binds pattern variables to the declared field
//! types (a bare-variable arm binds the scrutinee's type), requires
//! **exhaustiveness** when the scrutinee's type is known — every constructor
//! of a variant type, both `true` and `false` for Bool, and always a
//! catch-all for Float/String literal matches, unless a catch-all arm
//! exists — and joins the arm result types (compatible where known; the
//! match's type is Unknown unless all arms agree exactly).
//!
//! Top-level `let`s get placeholder signatures (annotations, with fresh
//! variables for whatever is unannotated) before bodies are checked, so
//! forward references see full signatures (matching the interpreter's late
//! binding); bodies then infer in dependency (SCC) order and generalize.
//!
//! [`check`] walks the whole module and returns **every** diagnostic, sorted
//! by source position — it never stops at the first error.

use crate::ast::{BinOp, TypeBody, TypeName};
use crate::eval::{builtin, callee_label, Builtin};
use crate::ir::{BindingId, Expr, ExprId, ExprKind, Field, MatchArm, Module, Pattern, PatternKind};
use crate::span::Span;
use crate::CheckError;
use std::collections::HashMap;
use std::fmt;

#[derive(Clone, PartialEq)]
pub enum Type {
    /// Not known statically; compatible with everything (see module docs).
    /// After B7 this is the GRADUAL seam only (host values, unknown
    /// uppercase type names) — unannotated code gets real [`Type::Var`]s.
    Unknown,
    /// An inference variable (B7). Solved through the checker's
    /// substitution; a variable still free after a def's inference is
    /// GENERALIZED there (let-polymorphism) and instantiated fresh at every
    /// use. Displays as `'a`, `'b`, … (normalized per top-level def).
    Var(u32),
    Float,
    String,
    Bool,
    List(Box<Type>),
    /// A product type: `Float * Float` in annotations. Structural, like the
    /// runtime.
    Tuple(Vec<Type>),
    /// A declared record type, nominal by name, with its type arguments
    /// (`Pair<Float, String>`; empty for non-generic declarations).
    Record(String, Vec<Type>),
    /// A declared variant type, nominal by name, with its type arguments
    /// (`Box<Float>`).
    Variant(String, Vec<Type>),
    Fn(Vec<Type>, Box<Type>),
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Unknown => write!(f, "Unknown"),
            // Raw display; hover/errors normalize vars to 'a, 'b first
            // (see `Checker::zonk_normalized`).
            Type::Var(v) => write!(f, "'{}", var_name(*v)),
            Type::Float => write!(f, "Float"),
            Type::String => write!(f, "String"),
            Type::Bool => write!(f, "Bool"),
            Type::List(elem) => write!(f, "List<{elem}>"),
            Type::Tuple(elems) => {
                for (i, elem) in elems.iter().enumerate() {
                    if i > 0 {
                        write!(f, " * ")?;
                    }
                    write!(f, "{elem}")?;
                }
                Ok(())
            }
            Type::Record(name, args) | Type::Variant(name, args) => {
                write!(f, "{name}")?;
                if args.is_empty() {
                    return Ok(());
                }
                write!(f, "<")?;
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{arg}")?;
                }
                write!(f, ">")
            }
            Type::Fn(params, ret) => {
                write!(f, "(")?;
                for (i, param) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{param}")?;
                }
                write!(f, ") => {ret}")
            }
        }
    }
}

/// Spell an inference variable: 0 → a, 1 → b, …, 26 → a1, ….
fn var_name(v: u32) -> String {
    let letter = (b'a' + (v % 26) as u8) as char;
    if v < 26 {
        letter.to_string()
    } else {
        format!("{letter}{}", v / 26)
    }
}

/// A polymorphic type: universally quantified variables plus a body. What
/// a top-level def (or generalized `let`-bound lambda) contributes to the
/// environment; instantiated with fresh variables at every use site.
#[derive(Clone)]
pub struct Scheme {
    pub vars: Vec<u32>,
    pub ty: Type,
}

/// Gradual compatibility: true unless the two types are known to disagree.
/// Unknown (at any depth) is compatible with everything. (Incompatibility is
/// an annotation-level claim, not a runtime guarantee — nominality only
/// exists in annotations; see the module doc.)
pub fn compatible(a: &Type, b: &Type) -> bool {
    match (a, b) {
        (Type::Unknown, _) | (_, Type::Unknown) => true,
        // An unsolved variable is compatible with anything at this level of
        // scrutiny (the unifier is where variables get COMMITTED).
        (Type::Var(_), _) | (_, Type::Var(_)) => true,
        (Type::Float, Type::Float) | (Type::String, Type::String) | (Type::Bool, Type::Bool) => {
            true
        }
        (Type::List(x), Type::List(y)) => compatible(x, y),
        (Type::Tuple(xs), Type::Tuple(ys)) => {
            xs.len() == ys.len() && xs.iter().zip(ys).all(|(x, y)| compatible(x, y))
        }
        (Type::Record(x, xa), Type::Record(y, ya))
        | (Type::Variant(x, xa), Type::Variant(y, ya)) => {
            x == y && xa.len() == ya.len() && xa.iter().zip(ya).all(|(a, b)| compatible(a, b))
        }
        (Type::Fn(p1, r1), Type::Fn(p2, r2)) => {
            p1.len() == p2.len()
                && p1.iter().zip(p2).all(|(x, y)| compatible(x, y))
                && compatible(r1, r2)
        }
        _ => false,
    }
}

/// Does this type (or, for products, any element) denote a function?
/// Runtime `==` errors on functions at any depth it actually compares, so a
/// known function anywhere in a compared tuple is a certain runtime error.
fn contains_fn(ty: &Type) -> bool {
    match ty {
        Type::Fn(..) => true,
        Type::Tuple(elems) => elems.iter().any(contains_fn),
        // A generic nominal can carry a function in its ARGUMENTS
        // (`Box<(Float) => Float>`) even when the declaration's own fields
        // are just parameters — recurse. [Codex H — generics review]
        Type::Record(_, args) | Type::Variant(_, args) => args.iter().any(contains_fn),
        _ => false,
    }
}

/// Every variable binding a pattern introduces (shallow — the pattern
/// language has no nesting beyond ctor/tuple args).
fn pattern_var_bindings(pattern: &Pattern, f: &mut impl FnMut(u32)) {
    match &pattern.kind {
        PatternKind::Var { binding, .. } => f(binding.0),
        PatternKind::Ctor { args, .. } | PatternKind::Tuple(args) => {
            for arg in args {
                pattern_var_bindings(arg, f);
            }
        }
        PatternKind::List { items, tail } => {
            for arg in items {
                pattern_var_bindings(arg, f);
            }
            if let Some(tail) = tail {
                pattern_var_bindings(tail, f);
            }
        }
        PatternKind::Wildcard
        | PatternKind::Number(_)
        | PatternKind::Bool(_)
        | PatternKind::String(_) => {}
    }
}

/// Substitute declaration parameter placeholders (`Var(i)`, i < args.len())
/// with concrete type arguments — how generic record/variant field types
/// meet their use sites. Non-generic declarations contain no placeholders,
/// so this is the identity for them.
fn subst_params(ty: &Type, args: &[Type]) -> Type {
    if args.is_empty() {
        return ty.clone();
    }
    match ty {
        Type::Var(v) => args
            .get(*v as usize)
            .cloned()
            .unwrap_or_else(|| Type::Var(*v)),
        Type::List(e) => Type::List(Box::new(subst_params(e, args))),
        Type::Tuple(es) => Type::Tuple(es.iter().map(|e| subst_params(e, args)).collect()),
        Type::Fn(ps, r) => Type::Fn(
            ps.iter().map(|p| subst_params(p, args)).collect(),
            Box::new(subst_params(r, args)),
        ),
        Type::Record(n, a) => {
            Type::Record(n.clone(), a.iter().map(|t| subst_params(t, args)).collect())
        }
        Type::Variant(n, a) => {
            Type::Variant(n.clone(), a.iter().map(|t| subst_params(t, args)).collect())
        }
        other => other.clone(),
    }
}

/// Rewrite variables to their position in `order` (display normalization).
fn renumber_with(ty: &Type, order: &[u32]) -> Type {
    match ty {
        Type::Var(v) => {
            let idx = order.iter().position(|o| o == v).expect("collected") as u32;
            Type::Var(idx)
        }
        Type::List(e) => Type::List(Box::new(renumber_with(e, order))),
        Type::Tuple(es) => Type::Tuple(es.iter().map(|e| renumber_with(e, order)).collect()),
        Type::Fn(ps, r) => Type::Fn(
            ps.iter().map(|p| renumber_with(p, order)).collect(),
            Box::new(renumber_with(r, order)),
        ),
        Type::Record(n, args) => Type::Record(
            n.clone(),
            args.iter().map(|a| renumber_with(a, order)).collect(),
        ),
        Type::Variant(n, args) => Type::Variant(
            n.clone(),
            args.iter().map(|a| renumber_with(a, order)).collect(),
        ),
        other => other.clone(),
    }
}

/// Free inference variables of `ty`, appended to `out` (deduplicated).
fn free_vars_of(ty: &Type, out: &mut Vec<u32>) {
    match ty {
        Type::Var(v) => {
            if !out.contains(v) {
                out.push(*v);
            }
        }
        Type::List(elem) => free_vars_of(elem, out),
        Type::Tuple(elems) => {
            for elem in elems {
                free_vars_of(elem, out);
            }
        }
        Type::Fn(params, ret) => {
            for param in params {
                free_vars_of(param, out);
            }
            free_vars_of(ret, out);
        }
        Type::Record(_, args) | Type::Variant(_, args) => {
            for arg in args {
                free_vars_of(arg, out);
            }
        }
        Type::Unknown | Type::Float | Type::String | Type::Bool => {}
    }
}

/// The signature of a builtin (kept in sync with [`crate::eval`]'s registry
/// by matching on [`Builtin`]). Generic slots are Unknown rather than
/// instantiated type variables — e.g. `List.map : (List<T>, (T) => U) =>
/// List<U>` is `(List<Unknown>, (Unknown) => Unknown) => List<Unknown>` — so
/// element types don't flow through, but arity and the known parts (like
/// `List.filter`'s Bool-returning predicate) still check.
pub fn builtin_signature(b: Builtin) -> Type {
    use Type::*;
    fn func(params: Vec<Type>, ret: Type) -> Type {
        Fn(params, Box::new(ret))
    }
    match b {
        // Generic slots are Var(0)/Var(1); every use site instantiates
        // them fresh (B7), so element types genuinely flow through.
        // List.map : (List<'a>, ('a) => 'b) => List<'b>
        Builtin::ListMap => func(
            vec![List(Box::new(Var(0))), func(vec![Var(0)], Var(1))],
            List(Box::new(Var(1))),
        ),
        // List.filter : (List<'a>, ('a) => Bool) => List<'a>
        Builtin::ListFilter => func(
            vec![List(Box::new(Var(0))), func(vec![Var(0)], Bool)],
            List(Box::new(Var(0))),
        ),
        // List.fold : (List<'a>, ('b, 'a) => 'b, 'b) => 'b
        Builtin::ListFold => func(
            vec![
                List(Box::new(Var(0))),
                func(vec![Var(1), Var(0)], Var(1)),
                Var(1),
            ],
            Var(1),
        ),
        // List.range : (Float) => List<Float>
        Builtin::ListRange => func(vec![Float], List(Box::new(Float))),
        // List.maximum : (List<Float>) => Float
        Builtin::ListMaximum => func(vec![List(Box::new(Float))], Float),
        // Text.concat : (String, String) => String
        Builtin::TextConcat => func(vec![String, String], String),
        // Text.fromFloat : (Float) => String
        Builtin::TextFromFloat => func(vec![Float], String),
        // Text.fixed : (Float, Float) => String — n at a fixed decimal count
        Builtin::TextFixed => func(vec![Float, Float], String),
        // Text.toBullets : (List<String>) => String
        Builtin::TextToBullets => func(vec![List(Box::new(String))], String),
        // Text.split : (String, String) => List<String>
        Builtin::TextSplit => func(vec![String, String], List(Box::new(String))),
        // Text.join : (List<String>, String) => String
        Builtin::TextJoin => func(vec![List(Box::new(String)), String], String),
        // Text.parseFloat : (String) => Float
        Builtin::TextParseFloat => func(vec![String], Float),
        // Math.clamp01 / sin / cos : (Float) => Float
        Builtin::MathClamp01 | Builtin::MathSin | Builtin::MathCos => func(vec![Float], Float),
    }
}

/// The checker's best-known types — the substrate for editor hover (see
/// [`crate::hover`]): one table per expression node ([`ExprId`]) and one per
/// value binding ([`BindingId`] — lambda params, `let … in`s, and pattern
/// variables, whose types have no expression node of their own).
pub struct ExprTypes {
    exprs: HashMap<u32, Type>,
    bindings: HashMap<u32, Type>,
}

impl ExprTypes {
    pub fn expr(&self, id: ExprId) -> Option<&Type> {
        self.exprs.get(&id.raw())
    }

    pub fn binding(&self, id: BindingId) -> Option<&Type> {
        self.bindings.get(&id.0)
    }
}

/// Which record types are visible UNQUALIFIED per module — a multi-file
/// project (B8, [`crate::project`]) hands this to [`check_with_scopes`] so
/// a bare record literal only resolves nominally against types in scope
/// where it is written (its own module's plus `open`ed ones). Without it,
/// an unreferenced sibling declaring a same-shaped type would make an
/// existing literal ambiguous — an implicit cross-module dependency the
/// project's dependency graph never sees. Annotated positions are
/// unaffected (a qualified annotation reaches any module's type).
#[derive(Default)]
pub struct RecordLiteralScopes {
    /// Def-name prefix (the module: `"Utils"`; `""` for the entry, whose
    /// canonical names are bare) → visible canonical record-type names.
    pub by_module: HashMap<String, std::collections::HashSet<String>>,
}

/// Check a lowered module; returns every diagnostic, sorted by position.
/// Empty means clean. Single-module: every declared record type is in
/// scope for literal resolution (see [`RecordLiteralScopes`]).
pub fn check(module: &Module) -> Vec<CheckError> {
    check_impl(module, None).0
}

/// [`check`] for a merged multi-file module, with per-module record-literal
/// scopes (see [`RecordLiteralScopes`]; `crate::project::Project::check`
/// is the caller-facing seam).
pub fn check_with_scopes(module: &Module, scopes: &RecordLiteralScopes) -> Vec<CheckError> {
    check_impl(module, Some(scopes)).0
}

/// [`check`], also returning the per-expression types recorded during the
/// (final, loud) inference pass.
pub fn check_with_types(module: &Module) -> (Vec<CheckError>, ExprTypes) {
    check_impl(module, None)
}

fn check_impl(
    module: &Module,
    scopes: Option<&RecordLiteralScopes>,
) -> (Vec<CheckError>, ExprTypes) {
    let mut checker = Checker {
        scopes,
        current_module: String::new(),
        subst: HashMap::new(),
        next_var: 0,
        schemes: HashMap::new(),
        in_type_decl: false,
        annot_vars: HashMap::new(),
        records: HashMap::new(),
        variants: HashMap::new(),
        ctors: HashMap::new(),
        globals: HashMap::new(),
        locals: HashMap::new(),
        diags: Vec::new(),
        expr_types: HashMap::new(),
    };

    // Record type names first (nominal references may be forward), then
    // resolve each declaration's field types (reporting bad type arity).
    for decl in &module.types {
        match &decl.body {
            TypeBody::Record(_) => {
                checker
                    .records
                    .insert(decl.name.clone(), (decl.params.len(), Vec::new()));
            }
            TypeBody::Variants(variants) => {
                checker.variants.insert(
                    decl.name.clone(),
                    (
                        decl.params.len(),
                        variants.iter().map(|v| v.name.clone()).collect(),
                    ),
                );
            }
        }
    }
    checker.in_type_decl = true;
    for decl in &module.types {
        // Declared params resolve to out-of-band placeholders Var(0..n); an
        // UNDECLARED lowercase name in a declaration is still the teaching
        // error (see resolve_type).
        checker.annot_vars.clear();
        for (i, param) in decl.params.iter().enumerate() {
            checker
                .annot_vars
                .insert(param.clone(), Type::Var(i as u32));
        }
        match &decl.body {
            TypeBody::Record(decl_fields) => {
                let fields = decl_fields
                    .iter()
                    .map(|f| (f.name.clone(), checker.resolve_type(&f.ty, true)))
                    .collect();
                checker
                    .records
                    .insert(decl.name.clone(), (decl.params.len(), fields));
            }
            TypeBody::Variants(variants) => {
                for variant in variants {
                    let fields = variant
                        .fields
                        .iter()
                        .map(|f| checker.resolve_type(&f.ty, true))
                        .collect();
                    checker.ctors.insert(
                        variant.name.clone(),
                        (decl.name.clone(), decl.params.len(), fields),
                    );
                }
            }
        }
    }

    checker.in_type_decl = false;
    checker.annot_vars.clear();

    // Placeholder signatures: annotation-derived, with FRESH inference
    // variables where nothing is annotated (B7 — this is what makes
    // unannotated code inferable instead of Unknown). Resolution is silent;
    // the body pass resolves the same annotations again and reports once.
    for def in &module.defs {
        checker.annot_vars.clear();
        let ty = match &def.value.kind {
            ExprKind::Lambda { params, ret, .. } => {
                let params = params
                    .iter()
                    .map(|p| checker.resolve_annotation(p.ty.as_ref(), false))
                    .collect();
                let ret = checker.resolve_annotation(ret.as_ref(), false);
                Type::Fn(params, Box::new(ret))
            }
            ExprKind::Number(_) => Type::Float,
            ExprKind::String(_) => Type::String,
            ExprKind::Bool(_) => Type::Bool,
            _ => checker.fresh(),
        };
        checker.globals.insert(def.name.clone(), ty);
    }

    // Infer in dependency order, one strongly-connected component at a
    // time: within a group (mutual recursion) uses are monomorphic — the
    // standard HM letrec rule — and each def GENERALIZES as its group
    // finishes, so later defs instantiate real schemes (`id` used at Float
    // and String in one module works).
    for group in scc_groups(module) {
        for &i in &group {
            let def = &module.defs[i];
            checker.annot_vars.clear();
            // The def's module, from its canonical name ("Utils.foo" →
            // "Utils"; bare → the entry) — scopes bare record literals.
            checker.current_module = def
                .name
                .split_once('.')
                .map(|(module, _)| module.to_string())
                .unwrap_or_default();
            let placeholder = checker
                .globals
                .get(&def.name)
                .cloned()
                .unwrap_or(Type::Unknown);
            let inferred = checker.infer(&def.value);
            checker.unify(
                &inferred,
                &placeholder,
                def.value.span,
                &format!("`{}`", def.name),
            );
        }
        for &i in &group {
            let def = &module.defs[i];
            let ty = checker
                .globals
                .get(&def.name)
                .cloned()
                .unwrap_or(Type::Unknown);
            let scheme = checker.generalize(&ty);
            checker.schemes.insert(def.name.clone(), scheme);
        }
    }

    checker.diags.sort_by_key(|d| d.span.start);
    // ONE display order for the whole table: the same variable must hover
    // as the same 'a everywhere (per-entry renumbering showed `q : 'a`
    // while the signature said 'b — review F5c).
    let mut order: Vec<u32> = Vec::new();
    let mut expr_items: Vec<(u32, Type)> = checker
        .expr_types
        .iter()
        .map(|(id, ty)| (*id, checker.zonk(ty)))
        .collect();
    expr_items.sort_by_key(|(id, _)| *id);
    let mut binding_items: Vec<(u32, Type)> = checker
        .locals
        .iter()
        .map(|(id, ty)| (*id, checker.zonk(ty)))
        .collect();
    binding_items.sort_by_key(|(id, _)| *id);
    for (_, ty) in expr_items.iter().chain(binding_items.iter()) {
        free_vars_of(ty, &mut order);
    }
    let exprs = expr_items
        .into_iter()
        .map(|(id, ty)| (id, renumber_with(&ty, &order)))
        .collect();
    let bindings = binding_items
        .into_iter()
        .map(|(id, ty)| (id, renumber_with(&ty, &order)))
        .collect();
    (checker.diags, ExprTypes { exprs, bindings })
}

/// Strongly-connected components of the def call graph (edges = `Global`
/// references), in dependency order — the generalization boundaries.
/// Iterative Tarjan; module-sized inputs, no recursion depth concerns.
fn scc_groups(module: &Module) -> Vec<Vec<usize>> {
    let index_of: HashMap<&str, usize> = module
        .defs
        .iter()
        .enumerate()
        .map(|(i, d)| (d.name.as_str(), i))
        .collect();
    let n = module.defs.len();
    let mut edges: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, def) in module.defs.iter().enumerate() {
        // Iterative worklist — the only whole-tree walk in `check`, and a
        // deep binary spine must not overflow the stack (the lowerer and
        // eval walk spines iteratively for the same reason).
        let mut work: Vec<&Expr> = vec![&def.value];
        while let Some(expr) = work.pop() {
            if let ExprKind::Global(name) = &expr.kind {
                if let Some(&j) = index_of.get(name.as_str()) {
                    if !edges[i].contains(&j) {
                        edges[i].push(j);
                    }
                }
            }
            crate::rebind::each_child(expr, &mut |child| work.push(child));
        }
    }
    // Tarjan, iterative.
    let mut index = vec![usize::MAX; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut next_index = 0usize;
    let mut components: Vec<Vec<usize>> = Vec::new();
    for start in 0..n {
        if index[start] != usize::MAX {
            continue;
        }
        // (node, next child position)
        let mut call: Vec<(usize, usize)> = vec![(start, 0)];
        while let Some(&mut (v, ref mut ci)) = call.last_mut() {
            if *ci == 0 {
                index[v] = next_index;
                low[v] = next_index;
                next_index += 1;
                stack.push(v);
                on_stack[v] = true;
            }
            if *ci < edges[v].len() {
                let w = edges[v][*ci];
                *ci += 1;
                if index[w] == usize::MAX {
                    call.push((w, 0));
                } else if on_stack[w] {
                    low[v] = low[v].min(index[w]);
                }
            } else {
                if low[v] == index[v] {
                    let mut component = Vec::new();
                    loop {
                        let w = stack.pop().expect("tarjan stack");
                        on_stack[w] = false;
                        component.push(w);
                        if w == v {
                            break;
                        }
                    }
                    components.push(component);
                }
                call.pop();
                if let Some(&mut (parent, _)) = call.last_mut() {
                    low[parent] = low[parent].min(low[v]);
                }
            }
        }
    }
    components
}

struct Checker<'s> {
    /// Per-module record-literal visibility (multi-file projects only —
    /// `None` checks a single module, where everything is visible).
    scopes: Option<&'s RecordLiteralScopes>,
    /// The module owning the def currently being inferred (`""` = entry).
    current_module: String,
    /// The substitution: solved inference variables (B7). Types are read
    /// through it via [`Checker::zonk`].
    subst: HashMap<u32, Type>,
    next_var: u32,
    /// Generalized top-level defs (populated as each dependency group
    /// finishes inference); instantiated fresh at every later use.
    schemes: HashMap<String, Scheme>,
    /// Resolving a TYPE DECLARATION's field annotations (lowercase names
    /// are refused there — see `resolve_type`).
    in_type_decl: bool,
    /// Lowercase annotation names in the CURRENT def's signature are scoped
    /// type variables (`(xs: List<a>, f: (a) => b): List<b>`); this maps
    /// them to their variable for the def's duration.
    annot_vars: HashMap<String, Type>,
    /// Declared record types: name → resolved fields, in declaration order.
    /// name → (type-parameter count, fields). Field types hold parameter
    /// PLACEHOLDERS as `Type::Var(i)` with i < the param count — an
    /// out-of-band namespace like the builtins': never unified raw, always
    /// [`subst_params`]-substituted first.
    records: HashMap<String, (usize, Vec<(String, Type)>)>,
    /// Declared variant types: name → (type-parameter count, constructor
    /// names in declaration order — the exhaustiveness universe). The param
    /// count is pre-seeded before field resolution, so recursive and
    /// forward generic references (`type L<a> = | Cons(h: a, t: L<a>)`)
    /// resolve at the right arity. [Codex H — generics review]
    variants: HashMap<String, (usize, Vec<String>)>,
    /// Declared constructors: name → (owning variant type, field types in
    /// declaration order). Names are module-unique (lowering enforces it).
    /// ctor name → (owning type, its param count, field types with the
    /// same placeholder convention as `records`).
    ctors: HashMap<String, (String, usize, Vec<Type>)>,
    globals: HashMap<String, Type>,
    /// Parameter types by binding ID (IDs are unique module-wide, so entries
    /// are never shadowed or popped).
    locals: HashMap<u32, Type>,
    diags: Vec<CheckError>,
    /// Best-known type per expression, recorded by [`Checker::infer`]. The
    /// loud pass runs last, so its (better-informed) types win.
    expr_types: HashMap<u32, Type>,
}

impl Checker<'_> {
    fn diag(&mut self, span: Span, message: String) {
        self.diags.push(CheckError { message, span });
    }

    /// May a bare record literal in the CURRENT def resolve to `name`?
    /// Only types in scope unqualified there (see [`RecordLiteralScopes`]);
    /// without scopes (single module) everything is a candidate.
    fn literal_candidate(&self, name: &str) -> bool {
        match self.scopes {
            Some(scopes) => scopes
                .by_module
                .get(&self.current_module)
                .is_some_and(|visible| visible.contains(name)),
            None => true,
        }
    }

    fn fresh(&mut self) -> Type {
        let v = self.next_var;
        self.next_var += 1;
        Type::Var(v)
    }

    /// Apply the substitution deeply — the type with everything solved so
    /// far written through.
    fn zonk(&self, ty: &Type) -> Type {
        match ty {
            Type::Var(v) => match self.subst.get(v) {
                Some(solved) => self.zonk(solved),
                None => Type::Var(*v),
            },
            Type::List(elem) => Type::List(Box::new(self.zonk(elem))),
            Type::Tuple(elems) => Type::Tuple(elems.iter().map(|e| self.zonk(e)).collect()),
            Type::Fn(params, ret) => Type::Fn(
                params.iter().map(|p| self.zonk(p)).collect(),
                Box::new(self.zonk(ret)),
            ),
            Type::Record(n, args) => {
                Type::Record(n.clone(), args.iter().map(|a| self.zonk(a)).collect())
            }
            Type::Variant(n, args) => {
                Type::Variant(n.clone(), args.iter().map(|a| self.zonk(a)).collect())
            }
            other => other.clone(),
        }
    }

    /// Unify two types, committing variable solutions. On a real conflict,
    /// reports ONE diagnostic at `span` showing the full zonked types
    /// (`got` = `a`, `expected` = `b`) — component mismatches inside lists,
    /// tuples, or functions surface as the whole types, which is what the
    /// source position actually shows. `Unknown` absorbs anything (the gradual
    /// seam); `origin` cites where the expected type came from.
    fn unify(&mut self, a: &Type, b: &Type, span: Span, what: &str) -> bool {
        if self.unify_rec(a, b, span, what) {
            return true;
        }
        let (got, expected) = self.normalize_pair(a, b);
        self.mismatch(&expected, &got, span, what);
        false
    }

    /// The recursive worker: solves what it can, returns false on conflict
    /// WITHOUT reporting (the wrapper reports once, with full types).
    fn unify_rec(&mut self, a: &Type, b: &Type, span: Span, what: &str) -> bool {
        let a = self.zonk(a);
        let b = self.zonk(b);
        match (&a, &b) {
            (Type::Unknown, _) | (_, Type::Unknown) => true,
            (Type::Var(v), _) => self.bind(*v, &b, span, what),
            (_, Type::Var(v)) => self.bind(*v, &a, span, what),
            (Type::Float, Type::Float)
            | (Type::String, Type::String)
            | (Type::Bool, Type::Bool) => true,
            (Type::Record(x, xa), Type::Record(y, ya))
            | (Type::Variant(x, xa), Type::Variant(y, ya))
                if x == y && xa.len() == ya.len() =>
            {
                let (xa, ya) = (xa.clone(), ya.clone());
                let mut ok = true;
                for (a, b) in xa.iter().zip(ya.iter()) {
                    ok &= self.unify_rec(a, b, span, what);
                }
                ok
            }
            (Type::List(x), Type::List(y)) => self.unify_rec(x, y, span, what),
            (Type::Tuple(xs), Type::Tuple(ys)) if xs.len() == ys.len() => {
                let mut ok = true;
                for (x, y) in xs.clone().iter().zip(ys.clone().iter()) {
                    ok &= self.unify_rec(x, y, span, what);
                }
                ok
            }
            (Type::Fn(p1, r1), Type::Fn(p2, r2)) if p1.len() == p2.len() => {
                let (p1, r1, p2, r2) = (p1.clone(), r1.clone(), p2.clone(), r2.clone());
                let mut ok = true;
                for (x, y) in p1.iter().zip(p2.iter()) {
                    ok &= self.unify_rec(x, y, span, what);
                }
                ok & self.unify_rec(&r1, &r2, span, what)
            }
            _ => false,
        }
    }

    /// Solve variable `v` as `ty` (occurs-checked: `'a = List<'a>` is an
    /// infinite type, reported rather than looped on).
    fn bind(&mut self, v: u32, ty: &Type, span: Span, what: &str) -> bool {
        if let Type::Var(w) = ty {
            if *w == v {
                return true;
            }
        }
        let mut free = Vec::new();
        free_vars_of(ty, &mut free);
        if free.contains(&v) {
            let (var, ty) = self.normalize_pair(&Type::Var(v), ty);
            self.diag(
                span,
                format!("{what}: cannot construct the infinite type {var} = {ty}"),
            );
            // Reported here; treat as absorbed so the wrapper doesn't add a
            // second, vaguer mismatch for the same conflict.
            return true;
        }
        self.subst.insert(v, ty.clone());
        true
    }

    /// Report a unification conflict. The `what` label names where the
    /// expectation came from ("argument 2 of `f`", "return value", "list
    /// element") — the legible-error contract.
    fn mismatch(&mut self, expected: &Type, got: &Type, span: Span, what: &str) {
        self.diag(span, format!("{what}: expected {expected}, got {got}"));
    }

    /// Instantiate a scheme: quantified variables become fresh ones.
    fn instantiate(&mut self, scheme: &Scheme) -> Type {
        if scheme.vars.is_empty() {
            return scheme.ty.clone();
        }
        let mapping: HashMap<u32, Type> = scheme.vars.iter().map(|v| (*v, self.fresh())).collect();
        fn walk(ty: &Type, mapping: &HashMap<u32, Type>) -> Type {
            match ty {
                Type::Var(v) => mapping.get(v).cloned().unwrap_or(Type::Var(*v)),
                Type::List(e) => Type::List(Box::new(walk(e, mapping))),
                Type::Tuple(es) => Type::Tuple(es.iter().map(|e| walk(e, mapping)).collect()),
                Type::Fn(ps, r) => Type::Fn(
                    ps.iter().map(|p| walk(p, mapping)).collect(),
                    Box::new(walk(r, mapping)),
                ),
                Type::Record(n, args) => {
                    Type::Record(n.clone(), args.iter().map(|a| walk(a, mapping)).collect())
                }
                Type::Variant(n, args) => {
                    Type::Variant(n.clone(), args.iter().map(|a| walk(a, mapping)).collect())
                }
                other => other.clone(),
            }
        }
        // NOT zonked — the load-bearing invariant: a real scheme's body is
        // zonked at generalization and its quantified vars never re-enter
        // unification (same-group uses go through the monomorphic
        // placeholder, later uses through fresh instantiations), while a
        // builtin signature's literal Var(0)/Var(1) ids DO collide with
        // live checker variables — the no-zonk rule is exactly what keeps
        // that collision inert (zonking here read unrelated solutions: the
        // var-collision bug the `functions.mle` golden caught, List.map's
        // 'a arriving pre-solved as Float).
        walk(&scheme.ty, &mapping)
    }

    /// Generalize a def's zonked type: every still-free variable is
    /// quantified (top-level defs close over nothing that could pin one).
    fn generalize(&self, ty: &Type) -> Scheme {
        let ty = self.zonk(ty);
        let mut vars = Vec::new();
        free_vars_of(&ty, &mut vars);
        Scheme { vars, ty }
    }

    /// Normalize TWO types with ONE shared variable order — a diagnostic
    /// showing both sides must name the same variable the same way (and
    /// different variables differently).
    fn normalize_pair(&self, a: &Type, b: &Type) -> (Type, Type) {
        let (a, b) = (self.zonk(a), self.zonk(b));
        let mut order = Vec::new();
        free_vars_of(&a, &mut order);
        free_vars_of(&b, &mut order);
        (renumber_with(&a, &order), renumber_with(&b, &order))
    }

    /// Resolve an annotation to a [`Type`]. Unknown type *names* are not
    /// errors (they may be generics like `T`); a recognized name applied at
    /// the wrong arity is. `report: false` resolves silently (the signature
    /// pre-pass, whose annotations the body pass resolves again).
    fn resolve_type(&mut self, ty: &TypeName, report: bool) -> Type {
        let arity_error = |checker: &mut Checker, takes: usize| {
            if report {
                checker.diag(
                    ty.span,
                    format!(
                        "`{}` takes {takes} type argument(s), got {}",
                        ty.name,
                        ty.args.len()
                    ),
                );
            }
            Type::Unknown
        };
        match ty.name.as_str() {
            "Float" | "String" | "Bool" => {
                if !ty.args.is_empty() {
                    return arity_error(self, 0);
                }
                match ty.name.as_str() {
                    "Float" => Type::Float,
                    "String" => Type::String,
                    _ => Type::Bool,
                }
            }
            "List" => {
                if ty.args.len() != 1 {
                    return arity_error(self, 1);
                }
                Type::List(Box::new(self.resolve_type(&ty.args[0], report)))
            }
            // The parser encodes a product annotation (`Float * Float`) as
            // the reserved name `*` with the elements as args.
            "*" => Type::Tuple(
                ty.args
                    .iter()
                    .map(|arg| self.resolve_type(arg, report))
                    .collect(),
            ),
            name if self.records.contains_key(name) => {
                let params = self.records.get(name).map(|(p, _)| *p).unwrap_or(0);
                if ty.args.len() != params {
                    return arity_error(self, params);
                }
                let args = ty
                    .args
                    .iter()
                    .map(|arg| self.resolve_type(arg, report))
                    .collect();
                Type::Record(name.to_string(), args)
            }
            name if self.variants.contains_key(name) => {
                let params = self.variants.get(name).map(|(p, _)| *p).unwrap_or(0);
                if ty.args.len() != params {
                    return arity_error(self, params);
                }
                let args = ty
                    .args
                    .iter()
                    .map(|arg| self.resolve_type(arg, report))
                    .collect();
                Type::Variant(name.to_string(), args)
            }
            // A lowercase name is a TYPE VARIABLE, scoped to the enclosing
            // def's signature: `(xs: List<a>, f: (a) => b): List<b>`. The
            // same name maps to the same variable within one def. In TYPE
            // DECLARATIONS they are refused — generic type declarations
            // aren't designed yet, and a declaration-held variable would be
            // module-global (first use pins it for everyone; both review
            // engines' probe). [F4 — B7 review]
            name if name.chars().next().is_some_and(char::is_lowercase) => {
                if !ty.args.is_empty() {
                    return arity_error(self, 0);
                }
                if let Some(var) = self.annot_vars.get(name) {
                    return var.clone();
                }
                if self.in_type_decl {
                    if report {
                        self.diag(
                            ty.span,
                            format!(
                                "undeclared type parameter `{name}` — declare it on \
the type: `type Name<{name}> = …`"
                            ),
                        );
                    }
                    return Type::Unknown;
                }
                let var = self.fresh();
                self.annot_vars.insert(name.to_string(), var.clone());
                var
            }
            // Unrecognized UPPERCASE names stay Unknown — the gradual seam
            // for host-side types this module doesn't declare. Still resolve
            // any arguments so nested annotations get their diagnostics.
            _ => {
                for arg in &ty.args {
                    self.resolve_type(arg, report);
                }
                Type::Unknown
            }
        }
    }

    fn resolve_annotation(&mut self, ty: Option<&TypeName>, report: bool) -> Type {
        match ty {
            Some(ty) => self.resolve_type(ty, report),
            // Unannotated: a real inference variable, not Unknown — this is
            // what B7 buys.
            None => self.fresh(),
        }
    }

    /// Check `expr` against a known expected type. Record and list literals
    /// are checked structurally against the expectation (this is where record
    /// literals meet their declared types); anything else is inferred and
    /// tested for compatibility. `what` names the expectation for the
    /// diagnostic ("argument 2 of `move`", "field `x` of `Position`").
    fn expect(&mut self, expr: &Expr, expected: &Type, what: &str) {
        let expected = &self.zonk(expected);
        if *expected == Type::Unknown {
            self.infer(expr);
            return;
        }
        // An unsolved variable has no structure to check literals against —
        // infer and commit it.
        if let Type::Var(_) = expected {
            let got = self.infer(expr);
            self.unify(&got, expected, expr.span, what);
            return;
        }
        match (&expr.kind, expected) {
            (ExprKind::Record(fields), Type::Record(name, targs)) => {
                let targs = targs.clone();
                self.check_record_literal(fields, name, &targs, expr.span);
                // Structural paths bypass `infer`, so record the checked
                // type here or hover would honestly-but-wrongly say Unknown
                // (and a stale quiet-pass entry could linger).
                self.expr_types.insert(expr.id.raw(), expected.clone());
            }
            // A record literal can never be a primitive/list/function,
            // whatever nominal type it might otherwise satisfy.
            (ExprKind::Record(fields), _) => {
                self.diag(
                    expr.span,
                    format!("{what}: expected {expected}, got a record literal"),
                );
                for field in fields {
                    self.infer(&field.value);
                }
            }
            (ExprKind::List(items), Type::List(elem)) => {
                for item in items {
                    self.expect(item, elem, "list element");
                }
                self.expr_types.insert(expr.id.raw(), expected.clone());
            }
            // A tuple literal against a product expectation: check each
            // element against its slot (so record/list elements meet their
            // declared types instead of hiding behind Unknown).
            (ExprKind::Tuple(items), Type::Tuple(elems)) => {
                if items.len() != elems.len() {
                    self.diag(
                        expr.span,
                        format!(
                            "{what}: expected {expected}, got a tuple of {} element(s)",
                            items.len()
                        ),
                    );
                    for item in items {
                        self.infer(item);
                    }
                    return;
                }
                for (i, (item, elem)) in items.iter().zip(elems.iter()).enumerate() {
                    self.expect(item, elem, &format!("tuple element {}", i + 1));
                }
                self.expr_types.insert(expr.id.raw(), expected.clone());
            }
            _ => {
                let got = self.infer(expr);
                let expected = self.zonk(expected);
                self.unify(&got, &expected, expr.span, what);
            }
        }
    }

    /// Check a record literal against declared record type `name`: every
    /// literal field must exist in the declaration and match its type, and
    /// every declared field must be present.
    fn check_record_literal(&mut self, fields: &[Field], name: &str, args: &[Type], span: Span) {
        let (_, decl) = self
            .records
            .get(name)
            .cloned()
            .expect("Type::Record names a declaration");
        for field in fields {
            match decl.iter().find(|(n, _)| n == &field.name) {
                Some((_, field_ty)) => {
                    let what = format!("field `{}` of `{name}`", field.name);
                    let field_ty = subst_params(field_ty, args);
                    self.expect(&field.value, &field_ty, &what);
                }
                None => {
                    self.diag(
                        field.span,
                        format!("`{name}` has no field `{}`", field.name),
                    );
                    self.infer(&field.value);
                }
            }
        }
        for (declared, _) in &decl {
            if !fields.iter().any(|f| &f.name == declared) {
                self.diag(
                    span,
                    format!("record literal for `{name}` is missing field `{declared}`"),
                );
            }
        }
    }

    fn infer(&mut self, expr: &Expr) -> Type {
        let ty = self.infer_inner(expr);
        self.expr_types.insert(expr.id.raw(), ty.clone());
        ty
    }

    fn infer_inner(&mut self, expr: &Expr) -> Type {
        match &expr.kind {
            ExprKind::Number(_) => Type::Float,
            ExprKind::Tuple(items) => {
                Type::Tuple(items.iter().map(|item| self.infer(item)).collect())
            }
            ExprKind::String(_) => Type::String,
            ExprKind::Bool(_) => Type::Bool,
            ExprKind::Local { binding, .. } => self
                .locals
                .get(&binding.0)
                .cloned()
                .unwrap_or(Type::Unknown),
            ExprKind::Global(name) => match self.schemes.get(name).cloned() {
                Some(scheme) => self.instantiate(&scheme),
                // Same dependency group: monomorphic placeholder (the HM
                // letrec rule).
                None => self.globals.get(name).cloned().unwrap_or(Type::Unknown),
            },
            // An unregistered external is a runtime concern (the module set
            // may grow); the checker only knows the builtins' signatures.
            ExprKind::External(path) => match builtin(path) {
                Some(b) => {
                    let sig = builtin_signature(b);
                    let mut vars = Vec::new();
                    free_vars_of(&sig, &mut vars);
                    self.instantiate(&Scheme { vars, ty: sig })
                }
                None => Type::Unknown,
            },
            // A record literal resolves NOMINALLY, F#-style (B7, user
            // decision): the unique declared type with exactly this field
            // set. No match → anonymous record data, still gradual
            // (Unknown); several matches → ambiguous, ask for an
            // annotation. (`expect` handles annotated positions with
            // tailored diagnostics before this is reached.)
            ExprKind::Record(fields) => {
                let mut names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
                names.sort_unstable();
                let matches: Vec<String> = self
                    .records
                    .iter()
                    .filter(|(name, _)| self.literal_candidate(name))
                    .filter(|(_, (_, decl))| {
                        let mut declared: Vec<&str> =
                            decl.iter().map(|(n, _)| n.as_str()).collect();
                        declared.sort_unstable();
                        declared == names
                    })
                    .map(|(name, _)| name.clone())
                    .collect();
                match matches.as_slice() {
                    [name] => {
                        let name = name.clone();
                        // Generic declarations get fresh arguments, solved
                        // by the literal's field types (Pair<Float, String>
                        // from { first: 1.0, second: "s" }).
                        let params = self.records.get(&name).map(|(p, _)| *p).unwrap_or(0);
                        let args: Vec<Type> = (0..params).map(|_| self.fresh()).collect();
                        self.check_record_literal(fields, &name, &args, expr.span);
                        Type::Record(name, args)
                    }
                    [] => {
                        for field in fields {
                            self.infer(&field.value);
                        }
                        Type::Unknown
                    }
                    several => {
                        let mut several: Vec<&str> = several.iter().map(|s| s.as_str()).collect();
                        several.sort_unstable();
                        self.diag(
                            expr.span,
                            format!(
                                "ambiguous record literal: fields match {} — annotate which one is meant",
                                several.join(" and ")
                            ),
                        );
                        for field in fields {
                            self.infer(&field.value);
                        }
                        Type::Unknown
                    }
                }
            }
            ExprKind::List(items) => {
                // One element type, unified across all items — mixed lists
                // are now real errors (inference with teeth).
                let elem = self.fresh();
                for item in items {
                    let ty = self.infer(item);
                    self.unify(&ty, &elem, item.span, "list element");
                }
                Type::List(Box::new(elem))
            }
            ExprKind::ListCons { items, tail } => {
                let elem = self.fresh();
                for item in items {
                    let ty = self.infer(item);
                    self.unify(&ty, &elem, item.span, "list element");
                }
                let tail_ty = self.infer(tail);
                self.unify(
                    &tail_ty,
                    &Type::List(Box::new(elem.clone())),
                    tail.span,
                    "`..` tail",
                );
                Type::List(Box::new(elem))
            }
            ExprKind::RecordUpdate { base, fields } => {
                let base_ty = self.infer(base);
                let base_ty = self.zonk(&base_ty);
                match &base_ty {
                    Type::Record(name, targs) => {
                        let name = name.clone();
                        let targs = targs.clone();
                        for field in fields {
                            let decl_ty = self
                                .records
                                .get(&name)
                                .and_then(|(_, decl)| decl.iter().find(|(n, _)| n == &field.name))
                                .map(|(_, ty)| subst_params(ty, &targs));
                            match decl_ty {
                                Some(ty) => {
                                    let what = format!("field `{}` of `{name}`", field.name);
                                    self.expect(&field.value, &ty, &what);
                                }
                                None => {
                                    self.diag(
                                        field.span,
                                        format!("`{name}` has no field `{}`", field.name),
                                    );
                                    self.infer(&field.value);
                                }
                            }
                        }
                        base_ty
                    }
                    Type::Unknown | Type::Var(_) => {
                        for field in fields {
                            self.infer(&field.value);
                        }
                        Type::Unknown
                    }
                    other => {
                        self.diag(base.span, format!("`with` update on {other}, not a record"));
                        for field in fields {
                            self.infer(&field.value);
                        }
                        Type::Unknown
                    }
                }
            }
            ExprKind::LocalMut { binding, .. } => self
                .locals
                .get(&binding.0)
                .cloned()
                .unwrap_or(Type::Unknown),
            ExprKind::Let {
                binding,
                value,
                body,
                ..
            } => {
                let value_ty = self.infer(value);
                self.locals.insert(binding.0, value_ty);
                self.infer(body)
            }
            ExprKind::Assign {
                binding,
                name,
                value,
                rest,
            } => {
                // The slot's type is fixed by its initializer: a `mut Float`
                // stays a Float across assignments.
                let slot_ty = self
                    .locals
                    .get(&binding.0)
                    .cloned()
                    .unwrap_or(Type::Unknown);
                let what = format!("assignment to `{name}`");
                self.expect(value, &slot_ty, &what);
                self.infer(rest)
            }
            ExprKind::FieldAccess { object, field } => {
                let object_ty = self.infer(object);
                let object_ty = self.zonk(&object_ty);
                match &object_ty {
                    Type::Record(name, targs) => {
                        let field_ty = self
                            .records
                            .get(name)
                            .and_then(|(_, decl)| decl.iter().find(|(n, _)| n == field))
                            .map(|(_, ty)| subst_params(ty, targs));
                        match field_ty {
                            Some(ty) => ty,
                            None => {
                                self.diag(expr.span, format!("`{name}` has no field `{field}`"));
                                Type::Unknown
                            }
                        }
                    }
                    // No row polymorphism: an unsolved object stays
                    // gradual rather than guessing a nominal type.
                    Type::Unknown | Type::Var(_) => Type::Unknown,
                    other => {
                        self.diag(expr.span, format!("`.{field}` on {other}, not a record"));
                        Type::Unknown
                    }
                }
            }
            ExprKind::Lambda { params, ret, body } => {
                let param_tys: Vec<Type> = params
                    .iter()
                    .map(|p| self.resolve_annotation(p.ty.as_ref(), true))
                    .collect();
                for (param, ty) in params.iter().zip(&param_tys) {
                    self.locals.insert(param.binding.0, ty.clone());
                }
                let annotated = ret.is_some();
                let ret_ty = self.resolve_annotation(ret.as_ref(), true);
                if annotated {
                    // expect() keeps the tailored record/list/tuple literal
                    // diagnostics for annotated returns.
                    self.expect(body, &ret_ty, "return value");
                } else {
                    let body_ty = self.infer(body);
                    self.unify(&body_ty, &ret_ty, body.span, "return value");
                }
                Type::Fn(param_tys, Box::new(ret_ty))
            }
            ExprKind::Call { callee, args } => {
                let callee_ty = self.infer(callee);
                match callee_ty {
                    Type::Fn(params, ret) => {
                        if params.len() != args.len() {
                            self.diag(
                                expr.span,
                                format!(
                                    "`{}` takes {} argument(s), got {}",
                                    callee_label(callee),
                                    params.len(),
                                    args.len()
                                ),
                            );
                            for arg in args {
                                self.infer(arg);
                            }
                        } else {
                            for (i, (arg, param_ty)) in args.iter().zip(&params).enumerate() {
                                let what =
                                    format!("argument {} of `{}`", i + 1, callee_label(callee));
                                self.expect(arg, param_ty, &what);
                            }
                        }
                        *ret
                    }
                    Type::Var(_) => {
                        let arg_tys: Vec<Type> = args.iter().map(|arg| self.infer(arg)).collect();
                        let ret = self.fresh();
                        let wanted = Type::Fn(arg_tys, Box::new(ret.clone()));
                        let what = format!("call of `{}`", callee_label(callee));
                        self.unify(&callee_ty, &wanted, expr.span, &what);
                        ret
                    }
                    Type::Unknown => {
                        for arg in args {
                            self.infer(arg);
                        }
                        Type::Unknown
                    }
                    other => {
                        self.diag(expr.span, format!("cannot call {other}, not a function"));
                        for arg in args {
                            self.infer(arg);
                        }
                        Type::Unknown
                    }
                }
            }
            ExprKind::Binary { .. } => {
                // Left-assoc chains nest down the lhs with no parser depth
                // guard — walk the spine iteratively, like lower and eval.
                let mut spine = Vec::new();
                let mut leaf = expr;
                while let ExprKind::Binary { op, lhs, rhs } = &leaf.kind {
                    spine.push((*op, rhs.as_ref(), leaf.span, leaf.id));
                    leaf = lhs;
                }
                let mut acc = self.infer(leaf);
                let mut acc_span = leaf.span;
                for (op, rhs, node_span, node_id) in spine.into_iter().rev() {
                    let rhs_ty = self.infer(rhs);
                    acc = self.binary(op, &acc, acc_span, &rhs_ty, rhs.span, node_span);
                    acc_span = node_span;
                    // Spine nodes never pass through the recording `infer`
                    // wrapper (the walk is iterative) — record each here.
                    self.expr_types.insert(node_id.raw(), acc.clone());
                }
                acc
            }
            ExprKind::Neg(inner) => {
                let ty = self.infer(inner);
                let ty = self.zonk(&ty);
                if let Type::Var(_) = ty {
                    self.unify(&ty, &Type::Float, inner.span, "unary `-` operand");
                } else if !compatible(&ty, &Type::Float) {
                    self.diag(
                        inner.span,
                        format!("unary `-` needs a Float operand, got {ty}"),
                    );
                }
                Type::Float
            }
            // A constructor reference: nullary is the variant value itself,
            // parameterful is a function from its declared field types.
            ExprKind::Ctor { name, .. } => match self.ctors.get(name).cloned() {
                Some((type_name, params, fields)) => {
                    // Fresh arguments per USE — `Full(1.0)` and `Full("s")`
                    // coexist as Box<Float> and Box<String>.
                    let args: Vec<Type> = (0..params).map(|_| self.fresh()).collect();
                    if fields.is_empty() {
                        Type::Variant(type_name, args)
                    } else {
                        let fields = fields.iter().map(|f| subst_params(f, &args)).collect();
                        Type::Fn(fields, Box::new(Type::Variant(type_name, args)))
                    }
                }
                // Unreachable (lowering rejects unknown constructors) —
                // stay gradual rather than panic.
                None => Type::Unknown,
            },
            ExprKind::Match { scrutinee, arms } => self.check_match(expr, scrutinee, arms),
        }
    }

    /// Check a `match` (see the module doc): pattern compatibility against
    /// the scrutinee's type, pattern-variable binding types, exhaustiveness
    /// where the scrutinee's type is known, and the arm-result join.
    fn check_match(&mut self, expr: &Expr, scrutinee: &Expr, arms: &[MatchArm]) -> Type {
        let scrutinee_ty = self.infer(scrutinee);
        let scrutinee_ty = self.zonk(&scrutinee_ty);
        let mut has_catch_all = false;
        let mut saw_true = false;
        let mut saw_false = false;
        let mut covered_ctors: Vec<&str> = Vec::new();
        // List coverage: exact-length patterns (`[a, b]` → 2) and min-length
        // tail patterns (`[h, ..t]` → 1, matches len >= 1).
        let mut list_exact: Vec<usize> = Vec::new();
        let mut list_tail_mins: Vec<usize> = Vec::new();
        let mut result: Option<Type> = None;
        for arm in arms {
            // Arms after a catch-all are unreachable at runtime — they are
            // still CHECKED (garbage draws diagnostics) but must not
            // CONSTRAIN the scrutinee (an unreachable `"s"` arm must not
            // pin an inferred scrutinee to String). [Codex M — B7 review]
            self.check_pattern_constraining(&arm.pattern, &scrutinee_ty, !has_catch_all);
            match &arm.pattern.kind {
                PatternKind::Wildcard | PatternKind::Var { .. } => has_catch_all = true,
                PatternKind::List { items, tail } => {
                    if tail.is_some() {
                        list_tail_mins.push(items.len());
                    } else {
                        list_exact.push(items.len());
                    }
                }
                // Sub-patterns are irrefutable (names/`_`), so a tuple arm
                // catches every tuple of its arity — but only if that arity
                // CAN match the scrutinee (the mismatch itself is diagnosed
                // in check_pattern; it must not also count as exhaustive).
                PatternKind::Tuple(args) => match &self.zonk(&scrutinee_ty) {
                    Type::Tuple(elems) if elems.len() != args.len() => {}
                    _ => has_catch_all = true,
                },
                PatternKind::Ctor { name, .. } => covered_ctors.push(name),
                PatternKind::Bool(true) => saw_true = true,
                PatternKind::Bool(false) => saw_false = true,
                PatternKind::Number(_) | PatternKind::String(_) => {}
            }
            // Arms UNIFY into one result type — a var arm is constrained by
            // its siblings instead of collapsing the match to Unknown.
            // [BOTH engines — B7 review]
            let body_ty = self.infer(&arm.body);
            result = Some(match result {
                None => body_ty,
                Some(prev) => self.join_arms(prev, body_ty, arm.body.span),
            });
        }
        // Exhaustiveness fires only where the scrutinee's type is known —
        // RE-ZONKED: the arms' patterns may have just SOLVED it (the
        // stale-zonk hole both engines found: an inferred-scrutinee match
        // silently skipped exhaustiveness). [BOTH engines, High]
        let scrutinee_ty = self.zonk(&scrutinee_ty);
        if !has_catch_all {
            match &scrutinee_ty {
                Type::Variant(name, _) => {
                    let missing: Vec<String> = self
                        .variants
                        .get(name)
                        .map(|(_, declared)| {
                            declared
                                .iter()
                                .filter(|c| !covered_ctors.contains(&c.as_str()))
                                .map(|c| format!("`{c}`"))
                                .collect()
                        })
                        .unwrap_or_default();
                    if !missing.is_empty() {
                        self.diag(
                            expr.span,
                            format!(
                                "match on `{name}` is not exhaustive: missing {}",
                                missing.join(", ")
                            ),
                        );
                    }
                }
                Type::Bool => {
                    if !(saw_true && saw_false) {
                        let missing = match (saw_true, saw_false) {
                            (false, true) => "`true`",
                            (true, false) => "`false`",
                            _ => "`true`, `false`",
                        };
                        self.diag(
                            expr.span,
                            format!("match on Bool is not exhaustive: missing {missing}"),
                        );
                    }
                }
                // Literal patterns can never cover all numbers or strings.
                Type::Float | Type::String => {
                    self.diag(
                        expr.span,
                        format!(
                            "match on {scrutinee_ty} is not exhaustive: literal patterns need \
a catch-all arm (`_` or a name)"
                        ),
                    );
                }
                // A list match is exhaustive iff SOME `[x1..xn, ..t]`
                // pattern bounds the tail (covering len >= n) AND every
                // shorter length 0..n is covered by an exact pattern (or a
                // shorter tail pattern). `[]` + `[h, ..t]` is the canonical
                // exhaustive recursion.
                Type::List(_) => {
                    let covered = list_tail_mins.iter().min().is_some_and(|&min| {
                        (0..min).all(|len| {
                            list_exact.contains(&len) || list_tail_mins.iter().any(|&m| m <= len)
                        })
                    });
                    if !covered {
                        self.diag(
                            expr.span,
                            format!(
                                "match on {scrutinee_ty} is not exhaustive: add `[..rest]`, a catch-all (`_`), or arms covering the remaining lengths"
                            ),
                        );
                    }
                }
                // A known product with no arity-matching arm can never be
                // handled (the per-arm mismatch diags say why each arm
                // fails; this says the MATCH as a whole is uncovered).
                Type::Tuple(elems) => {
                    self.diag(
                        expr.span,
                        format!(
                            "match on {scrutinee_ty} is not exhaustive: no arm matches a \
{}-element tuple",
                            elems.len()
                        ),
                    );
                }
                // Unknown stays gradual; List/Record/Fn scrutinees already
                // drew per-pattern compatibility diagnostics above.
                _ => {}
            }
        }
        result.unwrap_or(Type::Unknown)
    }

    /// Join two match arms' types. Plain unification, with ONE contract
    /// lift first: the B6 producer treats a bare model as
    /// `(model, Effect.none())`, so an arm returning `m` beside an arm
    /// returning `(m, effect)` joins as the PAIR — otherwise the unifier
    /// is asked for the infinite type `'a = 'a * Unknown` and every
    /// effect-returning game fails `functor build`. The lift keys on the
    /// pair's second element being the host seam (Unknown), so real tuple
    /// mismatches (`(m, 1.0)` vs `m`) still error.
    fn join_arms(&mut self, prev: Type, body: Type, span: Span) -> Type {
        let (p, b) = (self.zonk(&prev), self.zonk(&body));
        for (pair, bare) in [(&p, &b), (&b, &p)] {
            if let Type::Tuple(items) = pair {
                if items.len() == 2
                    && items[1] == Type::Unknown
                    && !matches!(bare, Type::Tuple(_))
                    && self.unify_rec(bare, &items[0], span, "match arm")
                {
                    return Type::Tuple(vec![self.zonk(&items[0]), Type::Unknown]);
                }
            }
        }
        if !self.unify_rec(&p, &b, span, "match arm") {
            let (prev_n, body_n) = self.normalize_pair(&p, &b);
            self.diag(
                span,
                format!("match arms have incompatible types {prev_n} and {body_n}"),
            );
            Type::Unknown
        } else {
            self.zonk(&p)
        }
    }

    /// Check one pattern against the scrutinee's type and record its
    /// variables' binding types (a bare variable binds the scrutinee's type;
    /// constructor sub-patterns bind the declared field types).
    fn check_pattern(&mut self, pattern: &Pattern, scrutinee: &Type) {
        self.check_pattern_constraining(pattern, scrutinee, true);
    }

    fn check_pattern_constraining(&mut self, pattern: &Pattern, scrutinee: &Type, constrain: bool) {
        let scrutinee = &self.zonk(scrutinee);
        // A variable scrutinee is CONSTRAINED by the pattern: a ctor pattern
        // makes it the owning variant type, a tuple pattern a product of
        // fresh elements, a literal its primitive — then check proceeds with
        // the solved structure. (Unreachable arms pass constrain: false —
        // they are checked but must not solve the scrutinee.)
        if let Type::Var(_) = scrutinee {
            if !constrain {
                // Bind this arm's variables gradually and stop.
                pattern_var_bindings(pattern, &mut |binding| {
                    self.locals.insert(binding, Type::Unknown);
                });
                return;
            }
            let ctor_info = match &pattern.kind {
                PatternKind::Ctor { name, .. } => self.ctors.get(name).cloned(),
                _ => None,
            };
            let constrained: Option<Type> = match &pattern.kind {
                PatternKind::Ctor { .. } => ctor_info.map(|(type_name, params, _)| {
                    let targs = (0..params).map(|_| self.fresh()).collect();
                    Type::Variant(type_name, targs)
                }),
                PatternKind::Tuple(args) => {
                    Some(Type::Tuple((0..args.len()).map(|_| self.fresh()).collect()))
                }
                PatternKind::List { .. } => Some(Type::List(Box::new(self.fresh()))),
                PatternKind::Number(_) => Some(Type::Float),
                PatternKind::Bool(_) => Some(Type::Bool),
                PatternKind::String(_) => Some(Type::String),
                PatternKind::Wildcard | PatternKind::Var { .. } => None,
            };
            if let Some(constrained) = constrained {
                self.unify(scrutinee, &constrained, pattern.span, "match pattern");
                return self.check_pattern(pattern, &constrained);
            }
        }
        match &pattern.kind {
            PatternKind::Wildcard => {}
            PatternKind::Var { binding, .. } => {
                self.locals.insert(binding.0, scrutinee.clone());
            }
            PatternKind::Tuple(args) => match scrutinee {
                Type::Tuple(elems) => {
                    if elems.len() != args.len() {
                        self.diag(
                            pattern.span,
                            format!(
                                "this pattern names {} element(s), but the matched \
value is {scrutinee} — it can never match",
                                args.len()
                            ),
                        );
                    }
                    for (arg, elem) in args.iter().zip(elems.iter()) {
                        self.check_pattern(arg, elem);
                    }
                }
                Type::Unknown => {
                    for arg in args {
                        self.check_pattern(arg, &Type::Unknown);
                    }
                }
                other => {
                    self.diag(
                        pattern.span,
                        format!("a tuple pattern cannot match {other} — it can never match"),
                    );
                    for arg in args {
                        self.check_pattern(arg, &Type::Unknown);
                    }
                }
            },
            PatternKind::List { items, tail } => {
                // Element type from the scrutinee (List<elem>); each item and
                // the tail check against it (tail binds List<elem>).
                let elem = match scrutinee {
                    Type::List(elem) => (**elem).clone(),
                    Type::Unknown | Type::Var(_) => Type::Unknown,
                    other => {
                        self.diag(
                            pattern.span,
                            format!("a list pattern cannot match {other} — it can never match"),
                        );
                        Type::Unknown
                    }
                };
                for item in items {
                    self.check_pattern(item, &elem);
                }
                if let Some(tail) = tail {
                    self.check_pattern(tail, &Type::List(Box::new(elem)));
                }
            }
            PatternKind::Ctor { name, args } => match self.ctors.get(name).cloned() {
                Some((type_name, _, field_tys)) => {
                    let mut field_tys = field_tys;
                    match scrutinee {
                        Type::Variant(s, _) if *s != type_name => {
                            self.diag(
                                pattern.span,
                                format!("`{name}` is not a constructor of `{s}`"),
                            );
                        }
                        Type::Variant(_, targs) => {
                            // The scrutinee's arguments give the pattern's
                            // field types (Full(v) on Box<Float> binds
                            // v: Float).
                            field_tys = field_tys.iter().map(|f| subst_params(f, targs)).collect();
                        }
                        Type::Unknown => {}
                        other => {
                            self.diag(
                                pattern.span,
                                format!(
                                    "pattern `{name}` matches `{type_name}`, but the scrutinee \
is {other}"
                                ),
                            );
                        }
                    }
                    // Lowering fixed the pattern's arity to the declaration.
                    for (sub, field_ty) in args.iter().zip(&field_tys) {
                        self.check_pattern(sub, field_ty);
                    }
                }
                // Unreachable (lowering rejects unknown constructors) —
                // stay gradual rather than panic.
                None => {
                    for sub in args {
                        self.check_pattern(sub, &Type::Unknown);
                    }
                }
            },
            PatternKind::Number(_) => self.literal_pattern(scrutinee, Type::Float, pattern.span),
            PatternKind::Bool(_) => self.literal_pattern(scrutinee, Type::Bool, pattern.span),
            PatternKind::String(_) => self.literal_pattern(scrutinee, Type::String, pattern.span),
        }
    }

    fn literal_pattern(&mut self, scrutinee: &Type, literal: Type, span: Span) {
        if !compatible(scrutinee, &literal) {
            self.diag(
                span,
                format!("pattern matches {literal}, but the scrutinee is {scrutinee}"),
            );
        }
    }

    fn binary(
        &mut self,
        op: BinOp,
        lhs: &Type,
        lhs_span: Span,
        rhs: &Type,
        rhs_span: Span,
        node_span: Span,
    ) -> Type {
        match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => {
                self.require_float(op, lhs, lhs_span);
                self.require_float(op, rhs, rhs_span);
                Type::Float
            }
            BinOp::Lt | BinOp::Gt => {
                self.require_float(op, lhs, lhs_span);
                self.require_float(op, rhs, rhs_span);
                Type::Bool
            }
            // `==` is an error only where the runtime outcome is certain:
            // comparing functions always fails at runtime, and operands whose
            // known types cannot be equal always yield `false`. Runtime
            // equality is STRUCTURAL, so two same-shaped nominal record types
            // may legitimately compare — only differing declared shapes are
            // errors.
            BinOp::Eq => {
                let (lhs, rhs) = (&self.zonk(lhs), &self.zonk(rhs));
                // A known function on EITHER side is a certain runtime error
                // regardless of the other side — check before variables get
                // a chance to unify with it.
                if contains_fn(lhs) || contains_fn(rhs) {
                    self.diag(
                        node_span,
                        "functions cannot be compared with `==`".to_string(),
                    );
                    return Type::Bool;
                }
                // Otherwise equality constrains unsolved sides together —
                // vars at ANY depth, not just top level (`(x, 1.0) ==
                // (1.0, 1.0)` must pin x). [Codex H — B7 review]
                let mut vars = Vec::new();
                free_vars_of(lhs, &mut vars);
                free_vars_of(rhs, &mut vars);
                if !vars.is_empty() {
                    self.unify(lhs, rhs, node_span, "`==` operands");
                    return Type::Bool;
                }
                match (lhs, rhs) {
                    // One known function operand is enough: the runtime
                    // rejects `==` whenever EITHER side is a function
                    // (closure, builtin, or unapplied constructor), so the
                    // other operand's type — even Unknown — cannot save it.
                    // Runtime equality recurses, so a tuple with a known
                    // function ELEMENT is just as certain to error.
                    _ if contains_fn(lhs) || contains_fn(rhs) => {
                        self.diag(
                            node_span,
                            "functions cannot be compared with `==`".to_string(),
                        );
                    }
                    (Type::Record(x, xargs), Type::Record(y, yargs)) => {
                        // Same declaration, incompatible arguments: the
                        // substituted fields cannot be equal — certainly
                        // false. [Codex H — generics review]
                        if x == y
                            && (xargs.len() != yargs.len()
                                || xargs.iter().zip(yargs).any(|(a, b)| !compatible(a, b)))
                        {
                            self.diag(
                                node_span,
                                format!(
                                    "`==` compares records with different shapes \
                                     ({lhs} and {rhs}) — always false"
                                ),
                            );
                        }
                        if x != y && !self.same_record_shape(x, xargs, y, yargs) {
                            self.diag(
                                node_span,
                                format!(
                                    "`==` compares records with different shapes \
                                     ({x} and {y}) — always false"
                                ),
                            );
                        }
                    }
                    _ => {
                        if !compatible(lhs, rhs) {
                            self.diag(
                                node_span,
                                format!(
                                    "`==` compares different types {lhs} and {rhs} (always false)"
                                ),
                            );
                        }
                    }
                }
                Type::Bool
            }
        }
    }

    /// Whether two declared record types have the same field-name set with
    /// pairwise-compatible types — i.e. their values can be structurally
    /// equal at runtime.
    /// Compare two record types' SUBSTITUTED field shapes — raw declaration
    /// fields hold parameter placeholders whose `compatible` is trivially
    /// true, hiding concrete disagreements. [Codex H — generics review]
    fn same_record_shape(&self, x: &str, xargs: &[Type], y: &str, yargs: &[Type]) -> bool {
        let (Some((_, xf)), Some((_, yf))) = (self.records.get(x), self.records.get(y)) else {
            return true; // unknown decl: stay gradual
        };
        xf.len() == yf.len()
            && xf.iter().all(|(name, ty)| {
                yf.iter().any(|(n, t)| {
                    n == name && compatible(&subst_params(ty, xargs), &subst_params(t, yargs))
                })
            })
    }

    fn require_float(&mut self, op: BinOp, ty: &Type, span: Span) {
        let ty = self.zonk(ty);
        if let Type::Var(_) = ty {
            self.unify(
                &ty,
                &Type::Float,
                span,
                &format!("`{}` operand", op_str(op)),
            );
            return;
        }
        if !compatible(&ty, &Type::Float) {
            self.diag(
                span,
                format!("`{}` needs Float operands, got {ty}", op_str(op)),
            );
        }
    }
}

fn op_str(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Lt => "<",
        BinOp::Gt => ">",
        BinOp::Eq => "==",
    }
}
