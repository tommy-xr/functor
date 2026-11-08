//! Headless parametric parts: load a project once, then evaluate a part
//! function `(params) => Solid` with many parameter records.
//!
//! ```text
//! let part = (p) => Solid.box(p.w, p.d, p.h)
//! ```
//!
//! Params are a record of finite numbers (millimetres, like export). The
//! sources are pinned at load, and a call that reads host state (a file, the
//! physics world) is rejected, so equal params always give the same mesh.

use std::collections::BTreeMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use functor_lang::project::Project;
use functor_lang::{Session, Value};
use functor_solid::{Mesh, Solid};

use crate::functor_lang_prelude::solid::FunctorLangSolid;
use crate::functor_lang_prelude::{read_counts, read_since, take_ui_handlers, FunctorHost};

/// A part function's parameters, by field name.
pub type Params = BTreeMap<String, f64>;

/// Rendered `file:line:col: message` errors.
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostics(pub Vec<String>);

impl Diagnostics {
    fn one(message: impl Into<String>) -> Self {
        Diagnostics(vec![message.into()])
    }
}

impl std::fmt::Display for Diagnostics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0.join("\n"))
    }
}

impl std::error::Error for Diagnostics {}

/// A loaded, checked project whose part functions can be evaluated. Not
/// `Send`: interpreter values are `Rc`-shared, so keep one per thread.
pub struct Evaluator {
    project: Project,
    session: Session,
}

impl Evaluator {
    /// Load, typecheck and evaluate the project whose entry is `entry` (a
    /// `.fun` file; its sibling files and the engine prelude join it).
    pub fn load(entry: &Path) -> Result<Evaluator, Diagnostics> {
        // Fonts and other project files resolve from the entry's folder.
        crate::storage::set_project_root(entry.to_path_buf());
        let project = functor_lang::project::load_with_bundled_modules(
            entry,
            &Default::default(),
            &functor_prelude::bundled_modules(),
        )
        .map_err(|e| Diagnostics::one(e.render()))?;
        let errors = project.check();
        if !errors.is_empty() {
            return Err(Diagnostics(
                errors
                    .iter()
                    .map(|e| project.sources.render(e.span.start, &e.message))
                    .collect(),
            ));
        }
        Evaluator::from_project(project)
    }

    /// Evaluate an already-checked project's top-level defs.
    pub fn from_project(project: Project) -> Result<Evaluator, Diagnostics> {
        let session = Session::load(&project.module, &mut FunctorHost);
        let _ = take_ui_handlers();
        let session = session.map_err(|failure| {
            Diagnostics::one(
                project
                    .sources
                    .render(failure.error.span.start, &failure.error.message),
            )
        })?;
        Ok(Evaluator { project, session })
    }

    /// Call `function` with `params` as a record, for its Solid.
    pub fn solid(&self, function: &str, params: &Params) -> Result<Arc<Solid>, Diagnostics> {
        if let Some((name, value)) = params.iter().find(|(_, v)| !v.is_finite()) {
            return Err(Diagnostics::one(format!(
                "param `{name}` must be a finite number, got {value}"
            )));
        }
        let record = Value::Record(Rc::new(
            params
                .iter()
                .map(|(name, value)| (name.clone(), Value::Number(*value)))
                .collect(),
        ));
        let before = read_counts();
        let result = self.session.call(function, vec![record], &mut FunctorHost);
        // `Ui.*` constructors queue handlers nothing here consumes.
        let _ = take_ui_handlers();
        let value = result.map_err(|e| {
            Diagnostics::one(self.project.sources.render(e.span.start, &e.message))
        })?;
        if let Some((class, reader)) = read_since(before) {
            return Err(Diagnostics::one(format!(
                "`{function}` read {} state through `{}` at call time; bind it at top level so it is fixed at load",
                class.name(),
                reader.unwrap_or("a built-in")
            )));
        }
        match &value {
            Value::HostData(data) => data.as_any().downcast_ref::<FunctorLangSolid>(),
            _ => None,
        }
        .map(|solid| solid.0.clone())
        .ok_or_else(|| {
            Diagnostics::one(format!(
                "`{function}` must return a Solid, got {}",
                value.kind_name()
            ))
        })
    }

    /// [`Self::solid`], meshed (millimetres).
    pub fn evaluate(&self, function: &str, params: &Params) -> Result<Mesh, Diagnostics> {
        self.solid(function, params)?
            .mesh()
            .map_err(|e| Diagnostics::one(format!("`{function}`: {e}")))
    }
}

/// Parse a JSON object of numbers, e.g. `{"w": 40, "h": 12.5}`.
pub fn parse_params(json: &str) -> Result<Params, Diagnostics> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| Diagnostics::one(format!("params: {e}")))?;
    let serde_json::Value::Object(fields) = value else {
        return Err(Diagnostics::one(format!(
            "params must be a JSON object of numbers, e.g. {{\"w\": 40}}, got {value}"
        )));
    };
    fields
        .into_iter()
        .map(|(name, value)| match value.as_f64() {
            Some(n) => Ok((name, n)),
            None => Err(Diagnostics::one(format!(
                "param `{name}` must be a number, got {value}"
            ))),
        })
        .collect()
}
