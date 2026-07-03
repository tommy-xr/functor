//! Basic typechecking over the core IR — Track B4 of `docs/mle.md`.
//!
//! This is **gradual checking with annotations, not inference**: anything
//! unannotated — and any type name that isn't a primitive, `List`, or a
//! declared record type (e.g. a generic parameter like `T`) — is
//! [`Type::Unknown`], and Unknown is compatible with everything. A check only
//! fires where **both** sides are known, so unannotated code never produces a
//! false positive; annotations buy diagnostics.
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
//! including the builtins, whose signatures live in [`builtin_signature`]
//! with generic slots as Unknown (no instantiation: `List.map`'s element
//! types simply aren't tracked); return annotations against the body's type;
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
//! Top-level `let`s contribute what their value's shape declares (a lambda's
//! annotations, a literal's type), then a quiet single-pass inference
//! upgrades what it can (an unannotated lambda return, a list literal's
//! element type). Signatures are collected before bodies are checked, so
//! forward references between functions see full signatures (matching the
//! interpreter's late binding).
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
    Unknown,
    Float,
    String,
    Bool,
    List(Box<Type>),
    /// A declared record type, nominal by name.
    Record(String),
    /// A declared variant type, nominal by name.
    Variant(String),
    Fn(Vec<Type>, Box<Type>),
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Unknown => write!(f, "Unknown"),
            Type::Float => write!(f, "Float"),
            Type::String => write!(f, "String"),
            Type::Bool => write!(f, "Bool"),
            Type::List(elem) => write!(f, "List<{elem}>"),
            Type::Record(name) | Type::Variant(name) => write!(f, "{name}"),
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

/// Gradual compatibility: true unless the two types are known to disagree.
/// Unknown (at any depth) is compatible with everything. (Incompatibility is
/// an annotation-level claim, not a runtime guarantee — nominality only
/// exists in annotations; see the module doc.)
pub fn compatible(a: &Type, b: &Type) -> bool {
    match (a, b) {
        (Type::Unknown, _) | (_, Type::Unknown) => true,
        (Type::Float, Type::Float) | (Type::String, Type::String) | (Type::Bool, Type::Bool) => {
            true
        }
        (Type::List(x), Type::List(y)) => compatible(x, y),
        (Type::Record(x), Type::Record(y)) => x == y,
        (Type::Variant(x), Type::Variant(y)) => x == y,
        (Type::Fn(p1, r1), Type::Fn(p2, r2)) => {
            p1.len() == p2.len()
                && p1.iter().zip(p2).all(|(x, y)| compatible(x, y))
                && compatible(r1, r2)
        }
        _ => false,
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
        // List.map : (List<T>, (T) => U) => List<U>
        Builtin::ListMap => func(
            vec![List(Box::new(Unknown)), func(vec![Unknown], Unknown)],
            List(Box::new(Unknown)),
        ),
        // List.filter : (List<T>, (T) => Bool) => List<T>
        Builtin::ListFilter => func(
            vec![List(Box::new(Unknown)), func(vec![Unknown], Bool)],
            List(Box::new(Unknown)),
        ),
        // List.fold : (List<T>, (U, T) => U, U) => U
        Builtin::ListFold => func(
            vec![
                List(Box::new(Unknown)),
                func(vec![Unknown, Unknown], Unknown),
                Unknown,
            ],
            Unknown,
        ),
        // List.range : (Float) => List<Float>
        Builtin::ListRange => func(vec![Float], List(Box::new(Float))),
        // List.maximum : (List<Float>) => Float
        Builtin::ListMaximum => func(vec![List(Box::new(Float))], Float),
        // Text.concat : (String, String) => String
        Builtin::TextConcat => func(vec![String, String], String),
        // Text.fromFloat : (Float) => String
        Builtin::TextFromFloat => func(vec![Float], String),
        // Text.toBullets : (List<String>) => String
        Builtin::TextToBullets => func(vec![List(Box::new(String))], String),
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

/// Check a lowered module; returns every diagnostic, sorted by position.
/// Empty means clean.
pub fn check(module: &Module) -> Vec<CheckError> {
    check_with_types(module).0
}

/// [`check`], also returning the per-expression types recorded during the
/// (final, loud) inference pass.
pub fn check_with_types(module: &Module) -> (Vec<CheckError>, ExprTypes) {
    let mut checker = Checker {
        records: HashMap::new(),
        variants: HashMap::new(),
        ctors: HashMap::new(),
        globals: HashMap::new(),
        locals: HashMap::new(),
        diags: Vec::new(),
        quiet: false,
        expr_types: HashMap::new(),
    };

    // Record type names first (nominal references may be forward), then
    // resolve each declaration's field types (reporting bad type arity).
    for decl in &module.types {
        match &decl.body {
            TypeBody::Record(_) => {
                checker.records.insert(decl.name.clone(), Vec::new());
            }
            TypeBody::Variants(variants) => {
                checker.variants.insert(
                    decl.name.clone(),
                    variants.iter().map(|v| v.name.clone()).collect(),
                );
            }
        }
    }
    for decl in &module.types {
        match &decl.body {
            TypeBody::Record(decl_fields) => {
                let fields = decl_fields
                    .iter()
                    .map(|f| (f.name.clone(), checker.resolve_type(&f.ty, true)))
                    .collect();
                checker.records.insert(decl.name.clone(), fields);
            }
            TypeBody::Variants(variants) => {
                for variant in variants {
                    let fields = variant
                        .fields
                        .iter()
                        .map(|f| checker.resolve_type(&f.ty, true))
                        .collect();
                    checker
                        .ctors
                        .insert(variant.name.clone(), (decl.name.clone(), fields));
                }
            }
        }
    }

    // Global signatures before bodies, so forward references between
    // functions see full signatures (the interpreter late-binds globals the
    // same way). Resolution is silent here — the body pass resolves the same
    // annotations again and reports once.
    for def in &module.defs {
        let ty = match &def.value.kind {
            ExprKind::Lambda { params, ret, .. } => Type::Fn(
                params
                    .iter()
                    .map(|p| checker.resolve_annotation(p.ty.as_ref(), false))
                    .collect(),
                Box::new(checker.resolve_annotation(ret.as_ref(), false)),
            ),
            ExprKind::Number(_) => Type::Float,
            ExprKind::String(_) => Type::String,
            ExprKind::Bool(_) => Type::Bool,
            _ => Type::Unknown,
        };
        checker.globals.insert(def.name.clone(), ty);
    }

    // Quiet enrichment: one inference pass upgrades what annotations alone
    // couldn't say (an unannotated lambda's return from its body, a list
    // literal's element type), so the diagnostic pass below checks against
    // the best-known signatures. Single pass by design — a chain of
    // unannotated-return functions stays Unknown (gradual, no fixed point).
    checker.quiet = true;
    for def in &module.defs {
        let inferred = checker.infer(&def.value);
        let entry = checker
            .globals
            .get_mut(&def.name)
            .expect("inserted in the pre-pass");
        *entry = merge_known(entry.clone(), inferred);
    }
    checker.quiet = false;

    for def in &module.defs {
        checker.infer(&def.value);
    }

    checker.diags.sort_by_key(|d| d.span.start);
    (
        checker.diags,
        ExprTypes {
            exprs: checker.expr_types,
            bindings: checker.locals,
        },
    )
}

struct Checker {
    /// Declared record types: name → resolved fields, in declaration order.
    records: HashMap<String, Vec<(String, Type)>>,
    /// Declared variant types: name → constructor names, in declaration
    /// order (the exhaustiveness universe).
    variants: HashMap<String, Vec<String>>,
    /// Declared constructors: name → (owning variant type, field types in
    /// declaration order). Names are module-unique (lowering enforces it).
    ctors: HashMap<String, (String, Vec<Type>)>,
    globals: HashMap<String, Type>,
    /// Parameter types by binding ID (IDs are unique module-wide, so entries
    /// are never shadowed or popped).
    locals: HashMap<u32, Type>,
    diags: Vec<CheckError>,
    /// Suppress diagnostics (the quiet enrichment pass — the loud pass walks
    /// the same nodes again and reports once).
    quiet: bool,
    /// Best-known type per expression, recorded by [`Checker::infer`]. The
    /// loud pass runs last, so its (better-informed) types win.
    expr_types: HashMap<u32, Type>,
}

/// Prefer the known parts of two views of the same definition's type: the
/// annotation-derived signature, upgraded by inference where the annotation
/// said Unknown.
fn merge_known(stored: Type, inferred: Type) -> Type {
    match (stored, inferred) {
        (Type::Unknown, inferred) => inferred,
        (Type::Fn(params, ret), Type::Fn(inferred_params, inferred_ret))
            if params.len() == inferred_params.len() =>
        {
            let params = params
                .into_iter()
                .zip(inferred_params)
                .map(|(s, i)| merge_known(s, i))
                .collect();
            Type::Fn(params, Box::new(merge_known(*ret, *inferred_ret)))
        }
        (stored, _) => stored,
    }
}

impl Checker {
    fn diag(&mut self, span: Span, message: String) {
        if !self.quiet {
            self.diags.push(CheckError { message, span });
        }
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
            name if self.records.contains_key(name) => {
                if !ty.args.is_empty() {
                    return arity_error(self, 0);
                }
                Type::Record(name.to_string())
            }
            name if self.variants.contains_key(name) => {
                if !ty.args.is_empty() {
                    return arity_error(self, 0);
                }
                Type::Variant(name.to_string())
            }
            // Unrecognized (a generic like `T`, or a type this module doesn't
            // declare): Unknown, not an error. Still resolve any arguments so
            // nested annotations (`T<Position<Float>>`) get their diagnostics.
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
            None => Type::Unknown,
        }
    }

    /// Check `expr` against a known expected type. Record and list literals
    /// are checked structurally against the expectation (this is where record
    /// literals meet their declared types); anything else is inferred and
    /// tested for compatibility. `what` names the expectation for the
    /// diagnostic ("argument 2 of `move`", "field `x` of `Position`").
    fn expect(&mut self, expr: &Expr, expected: &Type, what: &str) {
        if *expected == Type::Unknown {
            self.infer(expr);
            return;
        }
        match (&expr.kind, expected) {
            (ExprKind::Record(fields), Type::Record(name)) => {
                self.check_record_literal(fields, name, expr.span);
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
            _ => {
                let got = self.infer(expr);
                if !compatible(&got, expected) {
                    self.diag(expr.span, format!("{what}: expected {expected}, got {got}"));
                }
            }
        }
    }

    /// Check a record literal against declared record type `name`: every
    /// literal field must exist in the declaration and match its type, and
    /// every declared field must be present.
    fn check_record_literal(&mut self, fields: &[Field], name: &str, span: Span) {
        let decl = self
            .records
            .get(name)
            .cloned()
            .expect("Type::Record names a declaration");
        for field in fields {
            match decl.iter().find(|(n, _)| n == &field.name) {
                Some((_, field_ty)) => {
                    let what = format!("field `{}` of `{name}`", field.name);
                    let field_ty = field_ty.clone();
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
            ExprKind::String(_) => Type::String,
            ExprKind::Bool(_) => Type::Bool,
            ExprKind::Local { binding, .. } => self
                .locals
                .get(&binding.0)
                .cloned()
                .unwrap_or(Type::Unknown),
            ExprKind::Global(name) => self.globals.get(name).cloned().unwrap_or(Type::Unknown),
            // An unregistered external is a runtime concern (the module set
            // may grow); the checker only knows the builtins' signatures.
            ExprKind::External(path) => match builtin(path) {
                Some(b) => builtin_signature(b),
                None => Type::Unknown,
            },
            // A record literal with no expected type stays Unknown — record
            // types are nominal, and nothing here names one (see `expect` for
            // the checked positions).
            ExprKind::Record(fields) => {
                for field in fields {
                    self.infer(&field.value);
                }
                Type::Unknown
            }
            ExprKind::List(items) => {
                let mut elem: Option<Type> = None;
                for item in items {
                    let ty = self.infer(item);
                    elem = match elem {
                        None => Some(ty),
                        Some(prev) if prev == ty => Some(prev),
                        Some(_) => Some(Type::Unknown),
                    };
                }
                Type::List(Box::new(elem.unwrap_or(Type::Unknown)))
            }
            ExprKind::RecordUpdate { base, fields } => {
                let base_ty = self.infer(base);
                match &base_ty {
                    Type::Record(name) => {
                        let name = name.clone();
                        for field in fields {
                            let decl_ty = self
                                .records
                                .get(&name)
                                .and_then(|decl| decl.iter().find(|(n, _)| n == &field.name))
                                .map(|(_, ty)| ty.clone());
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
                    Type::Unknown => {
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
                match &object_ty {
                    Type::Record(name) => {
                        let field_ty = self
                            .records
                            .get(name)
                            .and_then(|decl| decl.iter().find(|(n, _)| n == field))
                            .map(|(_, ty)| ty.clone());
                        match field_ty {
                            Some(ty) => ty,
                            None => {
                                self.diag(expr.span, format!("`{name}` has no field `{field}`"));
                                Type::Unknown
                            }
                        }
                    }
                    Type::Unknown => Type::Unknown,
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
                let ret_ty = self.resolve_annotation(ret.as_ref(), true);
                let body_ty = if ret_ty == Type::Unknown {
                    self.infer(body)
                } else {
                    self.expect(body, &ret_ty, "return value");
                    ret_ty
                };
                Type::Fn(param_tys, Box::new(body_ty))
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
                if !compatible(&ty, &Type::Float) {
                    self.diag(
                        inner.span,
                        format!("unary `-` needs a Float operand, got {ty}"),
                    );
                }
                Type::Float
            }
            // A constructor reference: nullary is the variant value itself,
            // parameterful is a function from its declared field types.
            ExprKind::Ctor { name, .. } => match self.ctors.get(name) {
                Some((type_name, fields)) => {
                    if fields.is_empty() {
                        Type::Variant(type_name.clone())
                    } else {
                        Type::Fn(fields.clone(), Box::new(Type::Variant(type_name.clone())))
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
        let mut has_catch_all = false;
        let mut saw_true = false;
        let mut saw_false = false;
        let mut covered_ctors: Vec<&str> = Vec::new();
        let mut result: Option<Type> = None;
        for arm in arms {
            self.check_pattern(&arm.pattern, &scrutinee_ty);
            match &arm.pattern.kind {
                PatternKind::Wildcard | PatternKind::Var { .. } => has_catch_all = true,
                PatternKind::Ctor { name, .. } => covered_ctors.push(name),
                PatternKind::Bool(true) => saw_true = true,
                PatternKind::Bool(false) => saw_false = true,
                PatternKind::Number(_) | PatternKind::String(_) => {}
            }
            // All arms must agree where known; the match's type is the join
            // (Unknown as soon as the arms aren't literally the same type).
            let body_ty = self.infer(&arm.body);
            result = Some(match result {
                None => body_ty,
                Some(prev) => {
                    if !compatible(&prev, &body_ty) {
                        self.diag(
                            arm.body.span,
                            format!("match arms have incompatible types {prev} and {body_ty}"),
                        );
                    }
                    if prev == body_ty {
                        prev
                    } else {
                        Type::Unknown
                    }
                }
            });
        }
        // Exhaustiveness fires only where the scrutinee's type is known —
        // gradual, like every other check.
        if !has_catch_all {
            match &scrutinee_ty {
                Type::Variant(name) => {
                    let missing: Vec<String> = self
                        .variants
                        .get(name)
                        .map(|declared| {
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
                // Unknown stays gradual; List/Record/Fn scrutinees already
                // drew per-pattern compatibility diagnostics above.
                _ => {}
            }
        }
        result.unwrap_or(Type::Unknown)
    }

    /// Check one pattern against the scrutinee's type and record its
    /// variables' binding types (a bare variable binds the scrutinee's type;
    /// constructor sub-patterns bind the declared field types).
    fn check_pattern(&mut self, pattern: &Pattern, scrutinee: &Type) {
        match &pattern.kind {
            PatternKind::Wildcard => {}
            PatternKind::Var { binding, .. } => {
                self.locals.insert(binding.0, scrutinee.clone());
            }
            PatternKind::Ctor { name, args } => match self.ctors.get(name).cloned() {
                Some((type_name, field_tys)) => {
                    match scrutinee {
                        Type::Variant(s) if *s != type_name => {
                            self.diag(
                                pattern.span,
                                format!("`{name}` is not a constructor of `{s}`"),
                            );
                        }
                        Type::Unknown | Type::Variant(_) => {}
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
                match (lhs, rhs) {
                    (Type::Fn(..), Type::Fn(..)) => {
                        self.diag(
                            node_span,
                            "functions cannot be compared with `==`".to_string(),
                        );
                    }
                    (Type::Record(x), Type::Record(y)) => {
                        if x != y && !self.same_record_shape(x, y) {
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
    fn same_record_shape(&self, x: &str, y: &str) -> bool {
        let (Some(xf), Some(yf)) = (self.records.get(x), self.records.get(y)) else {
            return true; // unknown decl: stay gradual
        };
        xf.len() == yf.len()
            && xf
                .iter()
                .all(|(name, ty)| yf.iter().any(|(n, t)| n == name && compatible(ty, t)))
    }

    fn require_float(&mut self, op: BinOp, ty: &Type, span: Span) {
        if !compatible(ty, &Type::Float) {
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
