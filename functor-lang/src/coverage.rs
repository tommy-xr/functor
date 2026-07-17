//! Static execution-coverage support: the set of RUNNABLE positions.
//!
//! The paused inspector's recency gutter colors lines by when they last ran
//! (this frame / an earlier frame / a later frame). The fourth state —
//! "runnable but did NOT run in the window" (the un-taken match arm, the
//! false branch) — can't come from runtime coverage alone: a never-executed
//! expression appears in no frame's set. This walk enumerates every
//! expression span start in the module, statically, so the consumer knows
//! which positions COULD run. Mirrors the inlay/hover walk (Lambda bodies
//! and Match arms special-cased, everything else through
//! [`crate::hover::children`]) so the three walks stay in lockstep.

use crate::ir::{Expr, ExprKind, Module};

/// Every expression span start in the module's defs, sorted and deduped —
/// the static "could run" set the runtime pairs with per-frame coverage.
pub fn runnable_offsets(module: &Module) -> Vec<usize> {
    let mut out = Vec::new();
    for def in &module.defs {
        visit(&def.value, &mut out);
    }
    out.sort_unstable();
    out.dedup();
    out
}

fn visit(expr: &Expr, out: &mut Vec<usize>) {
    out.push(expr.span.start);
    match &expr.kind {
        ExprKind::Lambda { body, .. } => visit(body, out),
        ExprKind::Match { scrutinee, arms } => {
            visit(scrutinee, out);
            for arm in arms {
                visit(&arm.body, out);
            }
        }
        _ => {
            for child in crate::hover::children(expr) {
                visit(child, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Both match arms are runnable, statically — even though any single run
    // takes only one. The un-taken arm is exactly what the gutter's "dark"
    // state needs.
    #[test]
    fn match_arms_are_all_runnable() {
        let src = "let pick = (b) =>\n  match b with\n  | true => 1.0\n  | false => 2.0";
        let program = crate::parse(src).expect("parse");
        let module = crate::lower(program).expect("lower");
        let offsets = runnable_offsets(&module);

        let one = src.find("1.0").unwrap();
        let two = src.find("2.0").unwrap();
        assert!(offsets.contains(&one), "taken-arm body is runnable");
        assert!(offsets.contains(&two), "un-taken-arm body is runnable too");
        // Sorted + deduped.
        assert!(offsets.windows(2).all(|w| w[0] < w[1]));
    }

    // Lambda bodies (the walk's special case) are included.
    #[test]
    fn lambda_bodies_are_runnable() {
        let src = "let f = (x) => x + 1.0";
        let program = crate::parse(src).expect("parse");
        let module = crate::lower(program).expect("lower");
        let offsets = runnable_offsets(&module);
        assert!(offsets.contains(&src.find("x + 1.0").unwrap()));
    }
}
