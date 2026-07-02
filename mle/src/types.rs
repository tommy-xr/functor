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
//! - `List<T>`.
//! - Function types, from lambda annotations
//!   (`(a: Float, b: Float): Float => …`); an unannotated return type is the
//!   body's type when that is known.
//!
//! ## What is checked
//!
//! Arithmetic/comparison/unary-minus operand types; `==` across two
//! *different* known types (always `false` — almost certainly a bug); record
//! literals against a declared record type where one is expected (a return
//! annotation, an argument of a call with a known signature); field access on
//! a known record type; call arity and argument types where the callee's
//! function type is known — including the builtins, whose signatures live in
//! [`builtin_signature`] with generic slots as Unknown (no instantiation:
//! `List.map`'s element types simply aren't tracked); return annotations
//! against the body's type; and type-argument arity (`Position<Float>` is an
//! error, an *unknown* type name is not).
//!
//! Top-level `let`s contribute what their value's shape declares (a lambda's
//! annotations, a literal's type); everything else is Unknown. Signatures are
//! collected before bodies are checked, so forward references between
//! functions see full signatures (matching the interpreter's late binding).
//!
//! [`check`] walks the whole module and returns **every** diagnostic, sorted
//! by source position — it never stops at the first error.

use crate::ast::{BinOp, TypeName};
use crate::eval::{builtin, callee_label, Builtin};
use crate::ir::{Expr, ExprKind, Field, Module};
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
            Type::Record(name) => write!(f, "{name}"),
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
/// Unknown (at any depth) is compatible with everything, so this returning
/// `false` means the code *cannot* be well-typed at runtime.
pub fn compatible(a: &Type, b: &Type) -> bool {
    match (a, b) {
        (Type::Unknown, _) | (_, Type::Unknown) => true,
        (Type::Float, Type::Float) | (Type::String, Type::String) | (Type::Bool, Type::Bool) => {
            true
        }
        (Type::List(x), Type::List(y)) => compatible(x, y),
        (Type::Record(x), Type::Record(y)) => x == y,
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
        // List.maximum : (List<Float>) => Float
        Builtin::ListMaximum => func(vec![List(Box::new(Float))], Float),
        // Text.concat : (String, String) => String
        Builtin::TextConcat => func(vec![String, String], String),
        // Text.fromFloat : (Float) => String
        Builtin::TextFromFloat => func(vec![Float], String),
        // Text.toBullets : (List<String>) => String
        Builtin::TextToBullets => func(vec![List(Box::new(String))], String),
        // Math.clamp01 : (Float) => Float
        Builtin::MathClamp01 => func(vec![Float], Float),
    }
}

/// Check a lowered module; returns every diagnostic, sorted by position.
/// Empty means clean.
pub fn check(module: &Module) -> Vec<CheckError> {
    let mut checker = Checker {
        records: HashMap::new(),
        globals: HashMap::new(),
        locals: HashMap::new(),
        diags: Vec::new(),
    };

    // Record type names first (nominal references may be forward), then
    // resolve each declaration's field types (reporting bad type arity).
    for decl in &module.types {
        checker.records.insert(decl.name.clone(), Vec::new());
    }
    for decl in &module.types {
        let fields = decl
            .fields
            .iter()
            .map(|f| (f.name.clone(), checker.resolve_type(&f.ty, true)))
            .collect();
        checker.records.insert(decl.name.clone(), fields);
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

    for def in &module.defs {
        checker.infer(&def.value);
    }

    checker.diags.sort_by_key(|d| d.span.start);
    checker.diags
}

struct Checker {
    /// Declared record types: name → resolved fields, in declaration order.
    records: HashMap<String, Vec<(String, Type)>>,
    globals: HashMap<String, Type>,
    /// Parameter types by binding ID (IDs are unique module-wide, so entries
    /// are never shadowed or popped).
    locals: HashMap<u32, Type>,
    diags: Vec<CheckError>,
}

impl Checker {
    fn diag(&mut self, span: Span, message: String) {
        self.diags.push(CheckError { message, span });
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
            }
            (ExprKind::List(items), Type::List(elem)) => {
                for item in items {
                    self.expect(item, elem, "list element");
                }
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
                    spine.push((*op, rhs.as_ref(), leaf.span));
                    leaf = lhs;
                }
                let mut acc = self.infer(leaf);
                let mut acc_span = leaf.span;
                for (op, rhs, node_span) in spine.into_iter().rev() {
                    let rhs_ty = self.infer(rhs);
                    acc = self.binary(op, &acc, acc_span, &rhs_ty, rhs.span, node_span);
                    acc_span = node_span;
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
            // `==` across two different known types is always `false` —
            // almost certainly a bug, so it is an error, not a lint.
            BinOp::Eq => {
                if !compatible(lhs, rhs) {
                    self.diag(
                        node_span,
                        format!("`==` compares different types {lhs} and {rhs} (always false)"),
                    );
                }
                Type::Bool
            }
        }
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
