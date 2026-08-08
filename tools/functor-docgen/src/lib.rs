//! Structured API documentation generated from the exact Functor Lang sources
//! embedded in Functor: the host's `.funi` prelude and the language's own
//! standard library.
//!
//! The two are the same extraction — module `//!` prose, then a `///` block
//! above each type or value — over different sources, and they stay separate
//! only in the rendered output, as the [`ApiGroup`] each module carries.
//!
//! Within a group, modules are further sorted into categories ("Scene &
//! rendering", "Collections", …) by [`CATEGORIES`] — the one place the
//! reference's shape is declared.

use functor_lang::ast::{ExprKind, Item, TypeName};
use functor_lang::project::stdlib_documentation_modules;
use functor_lang::{docs::public_doc_comment_in_source, line_col, parse, parse_interface, Span};
use serde::Serialize;
use std::fmt;
use std::io;
use std::path::Path;

/// The versioned, presentation-neutral API reference consumed by the website
/// and the Markdown renderer.
#[derive(Debug, Serialize)]
pub struct ApiReference {
    pub schema_version: u32,
    pub modules: Vec<ApiModule>,
}

#[derive(Debug, Serialize)]
pub struct ApiModule {
    pub name: String,
    pub group: ApiGroup,
    /// The category heading this module renders under, within its group.
    pub category: String,
    /// The extension this module's source carries on disk, so an error about
    /// it names a plausible file. Presentation-neutral output does not need
    /// it.
    #[serde(skip)]
    pub extension: &'static str,
    pub docs: Option<String>,
    pub items: Vec<ApiItem>,
}

/// The reference's shape, declared ONCE: every documented module, in the order
/// it renders, under the group and category it belongs to.
///
/// This is the only place a module's category is stated, and it is
/// drift-proof in both directions — [`generate`] fails if an entry here names
/// a module that is not documented, if it claims the wrong group, or if a
/// documented module has no entry at all. A new prelude or standard-library
/// module therefore cannot land uncategorized.
const CATEGORIES: &[(ApiGroup, &str, &[&str])] = &[
    (
        ApiGroup::Engine,
        "Scene & rendering",
        &[
            "Scene",
            "Frame",
            "Camera3D",
            "Camera2D",
            "Sprite",
            "Light",
            "Skybox",
            "Texture",
            "Fog",
            "RenderTarget",
        ],
    ),
    (
        ApiGroup::Engine,
        "Math & geometry",
        &["Vec3", "Angle", "Color"],
    ),
    (
        ApiGroup::Engine,
        "Simulation",
        &["Physics", "Anim", "Terrain", "Time"],
    ),
    (ApiGroup::Engine, "Input", &["Input"]),
    (
        ApiGroup::Engine,
        "Effects & messaging",
        &["Effect", "Sub", "Persistence"],
    ),
    (ApiGroup::Engine, "Audio", &["AudioScene", "AudioSource"]),
    (ApiGroup::Engine, "UI", &["Ui", "Html", "Attr", "Style"]),
    (ApiGroup::Engine, "Assets", &["Asset"]),
    (ApiGroup::Stdlib, "Collections", &["List", "Map"]),
    (ApiGroup::Stdlib, "Text", &["Text"]),
    (
        ApiGroup::Stdlib,
        "Numbers & randomness",
        &["Math", "Random"],
    ),
    (ApiGroup::Stdlib, "Fallibility", &["Option", "Result"]),
    (ApiGroup::Stdlib, "Input", &["Key", "Mouse"]),
    (ApiGroup::Stdlib, "Diagnostics", &["Debug"]),
];

/// Which half of the API a module belongs to. Engine modules exist only under
/// a game runner; standard-library modules ship with the language and are
/// available everywhere Functor Lang runs, the plain CLI included.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ApiGroup {
    Engine,
    Stdlib,
}

impl ApiGroup {
    /// The heading this group renders under.
    pub fn title(self) -> &'static str {
        match self {
            ApiGroup::Engine => "Engine modules",
            ApiGroup::Stdlib => "Language standard library",
        }
    }

    /// One line of orientation, rendered under the group heading.
    pub fn summary(self) -> &'static str {
        match self {
            ApiGroup::Engine => {
                "Provided by the game runner. These resolve when Functor runs a game — \
                 natively, on the web, or in a headless test — and not under the plain \
                 `functor-lang` CLI."
            }
            ApiGroup::Stdlib => {
                "Ships with the language. These are available in every Functor Lang \
                 program, with or without a game runner."
            }
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ApiItem {
    pub name: String,
    pub qualified_name: String,
    pub kind: ApiItemKind,
    pub declaration: String,
    pub docs: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ApiItemKind {
    Type,
    Value,
    /// A `unit` declaration: the literal suffix a number may carry
    /// (`90deg` → `Angle.degrees(90.0)`).
    Unit,
    /// A `unit <suffix> (<op>)` declaration: an arithmetic operator on the
    /// brand that suffix builds (`90deg + 45deg` → `Angle.add(…)`).
    #[serde(rename = "unit-operator")]
    UnitOperator,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputFormat {
    Markdown,
    Json,
}

#[derive(Debug)]
pub struct GenerateError {
    module: String,
    /// The extension the module's source would carry on disk, so the error
    /// names a plausible file (`Option.fun`, not `Option.funi`).
    extension: &'static str,
    line: usize,
    col: usize,
    message: String,
}

impl fmt::Display for GenerateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}.{}:{}:{}: {}",
            self.module, self.extension, self.line, self.col, self.message
        )
    }
}

impl std::error::Error for GenerateError {}

impl ApiReference {
    /// Qualified names for every module or declaration missing public prose.
    pub fn undocumented(&self) -> Vec<String> {
        let mut missing = Vec::new();
        for module in &self.modules {
            if module
                .docs
                .as_deref()
                .is_none_or(|docs| docs.trim().is_empty())
            {
                missing.push(module.name.clone());
            }
            missing.extend(
                module
                    .items
                    .iter()
                    .filter(|item| {
                        item.docs
                            .as_deref()
                            .is_none_or(|docs| docs.trim().is_empty())
                    })
                    .map(|item| item.qualified_name.clone()),
            );
        }
        missing
    }
}

/// Generate the whole API from the exact sources embedded in every Functor
/// runtime and editor: the host prelude, then the language standard library.
pub fn generate() -> Result<ApiReference, GenerateError> {
    let mut modules = functor_prelude::modules()
        .into_iter()
        .map(|(name, source)| extract_module(name, source, ApiGroup::Engine, Parse::Interface))
        .collect::<Result<Vec<_>, _>>()?;
    for module in stdlib_documentation_modules() {
        let parse_as = if module.is_interface() {
            Parse::Interface
        } else {
            Parse::Implementation
        };
        modules.push(extract_module(
            module.name().to_string(),
            module.source().to_string(),
            ApiGroup::Stdlib,
            parse_as,
        )?);
    }
    // The two halves share reserved namespaces (`Debug`, `List`, … are
    // protected in both), and the page keys its anchors, nav links, and search
    // filter on the module name — so a collision would silently desync the
    // filter rather than look wrong. Refuse it here instead.
    for (index, module) in modules.iter().enumerate() {
        if let Some(earlier) = modules[..index].iter().find(|m| m.name == module.name) {
            return Err(GenerateError {
                module: module.name.clone(),
                extension: "funi",
                line: 1,
                col: 1,
                message: format!(
                    "`{}` is documented twice (once as {:?}, once as {:?})",
                    module.name, earlier.group, module.group
                ),
            });
        }
    }
    Ok(ApiReference {
        schema_version: 3,
        modules: categorize(modules, CATEGORIES)?,
    })
}

/// Sort the documented modules into [`CATEGORIES`] order, stamping each with
/// its category.
///
/// Both directions are errors, so the table and the sources cannot drift: a
/// table entry naming a module that is not documented (or documented in the
/// other group), and a documented module with no table entry at all. A group's
/// categories also have to be contiguous, since both renderers announce a
/// group once and then walk its categories.
fn categorize(
    mut modules: Vec<ApiModule>,
    categories: &[(ApiGroup, &str, &[&str])],
) -> Result<Vec<ApiModule>, GenerateError> {
    let mut ordered: Vec<ApiModule> = Vec::with_capacity(modules.len());
    let mut groups: Vec<ApiGroup> = Vec::new();
    for (group, category, names) in categories {
        if groups.last() != Some(group) {
            if groups.contains(group) {
                return Err(categorization_error(
                    names.first().unwrap_or(&""),
                    "funi",
                    format!(
                        "{group:?} categories are split around another group's — a group's \
                         categories must be contiguous in CATEGORIES (tools/functor-docgen), \
                         so \"{category}\" has to move next to the others"
                    ),
                ));
            }
            groups.push(*group);
        }
        for name in *names {
            // Modules are removed as they are placed, so a second mention would
            // otherwise report as "not documented" and point at the prelude
            // instead of at the table.
            let Some(index) = modules.iter().position(|module| module.name == *name) else {
                let message = if let Some(placed) = ordered.iter().find(|m| m.name == *name) {
                    format!(
                        "`{name}` is listed twice in CATEGORIES (tools/functor-docgen) — \
                         under \"{}\" and again under \"{category}\"",
                        placed.category
                    )
                } else {
                    format!(
                        "`{name}` is listed under \"{category}\" but no such module is \
                         documented — fix or remove its entry in CATEGORIES \
                         (tools/functor-docgen)"
                    )
                };
                return Err(categorization_error(name, "funi", message));
            };
            if modules[index].group != *group {
                return Err(categorization_error(
                    name,
                    modules[index].extension,
                    format!(
                        "`{name}` is listed under \"{category}\" as {group:?}, but it is \
                         documented as {:?} — fix its entry in CATEGORIES \
                         (tools/functor-docgen)",
                        modules[index].group
                    ),
                ));
            }
            let mut module = modules.remove(index);
            module.category = (*category).to_string();
            ordered.push(module);
        }
    }
    if let Some(module) = modules.first() {
        return Err(categorization_error(
            &module.name,
            module.extension,
            format!(
                "`{}` has no category — add it to CATEGORIES (tools/functor-docgen) so it \
                 renders under a heading",
                module.name
            ),
        ));
    }
    Ok(ordered)
}

fn categorization_error(module: &str, extension: &'static str, message: String) -> GenerateError {
    GenerateError {
        module: module.to_string(),
        extension,
        line: 1,
        col: 1,
        message,
    }
}

/// Generate a reference from `(module name, .funi source)` pairs — the
/// interface-only shape, used by tests. The synthetic modules are not part of
/// the real inventory, so they all render under one category.
pub fn generate_from_modules(
    modules: impl IntoIterator<Item = (String, String)>,
) -> Result<ApiReference, GenerateError> {
    let modules = modules
        .into_iter()
        .map(|(name, source)| {
            extract_module(name, source, ApiGroup::Engine, Parse::Interface).map(|mut module| {
                module.category = "Reference".to_string();
                module
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ApiReference {
        schema_version: 3,
        modules,
    })
}

/// Whether a documentation source is a bodyless `.funi` interface or an
/// executable `.fun` module whose signatures come from its annotations.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Parse {
    Interface,
    Implementation,
}

pub fn render(format: OutputFormat) -> Result<String, GenerateError> {
    let reference = generate()?;
    Ok(render_reference(&reference, format))
}

pub fn render_reference(reference: &ApiReference, format: OutputFormat) -> String {
    match format {
        OutputFormat::Markdown => render_markdown(reference),
        OutputFormat::Json => {
            let mut json =
                serde_json::to_string_pretty(reference).expect("API reference is serializable");
            json.push('\n');
            json
        }
    }
}

/// Write generated output, creating a non-empty parent directory when needed.
pub fn write_file(path: &Path, contents: &str) -> io::Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents)
}

/// Whether a generated file is current. Git may check text files out with
/// CRLF on Windows, so freshness is intentionally line-ending agnostic.
pub fn file_is_current(path: &Path, generated: &str) -> io::Result<bool> {
    let existing = std::fs::read_to_string(path)?;
    Ok(normalize_newlines(&existing) == normalize_newlines(generated))
}

pub fn render_markdown(reference: &ApiReference) -> String {
    let mut out = String::from(
        "<!-- Generated by `npm run generate:docs`; do not edit by hand. -->\n\n\
         # Functor API reference\n\n\
         This reference is generated from the Functor Lang sources embedded in Functor: \
         the host runtime's `.funi` prelude and the language's own standard library.\n",
    );
    let mut group = None;
    let mut category = None;
    for module in &reference.modules {
        if group != Some(module.group) {
            group = Some(module.group);
            category = None;
            out.push_str("\n## ");
            out.push_str(module.group.title());
            out.push_str("\n\n");
            out.push_str(module.group.summary());
            out.push('\n');
        }
        if category.is_none_or(|current| current != module.category.as_str()) {
            category = Some(module.category.as_str());
            out.push_str("\n### ");
            out.push_str(&module.category);
            out.push('\n');
        }
        out.push_str("\n#### ");
        out.push_str(&module.name);
        out.push('\n');
        if let Some(docs) = &module.docs {
            out.push('\n');
            out.push_str(docs);
            out.push('\n');
        }
        for item in &module.items {
            out.push_str("\n##### `");
            out.push_str(&item.qualified_name);
            out.push_str("`\n\n```functor\n");
            out.push_str(&item.declaration);
            out.push_str("\n```\n");
            if let Some(docs) = &item.docs {
                out.push('\n');
                out.push_str(docs);
                out.push('\n');
            }
        }
    }
    out
}

fn extract_module(
    name: String,
    source: String,
    group: ApiGroup,
    parse_as: Parse,
) -> Result<ApiModule, GenerateError> {
    let error_at = |span: Span, message: String| {
        let (line, col) = line_col(&source, span.start);
        GenerateError {
            module: name.clone(),
            extension: match parse_as {
                Parse::Interface => "funi",
                Parse::Implementation => "fun",
            },
            line,
            col,
            message,
        }
    };
    let parsed = match parse_as {
        Parse::Interface => parse_interface(&source),
        Parse::Implementation => parse(&source),
    };
    let program = parsed.map_err(|error| error_at(error.span, error.message))?;
    let mut items = Vec::new();
    for item in program.items {
        let (item_name, kind, span, declaration) = match item {
            Item::Type(decl) => {
                let declaration = declaration_at(&source, decl.span)
                    .ok_or_else(|| error_at(decl.span, invalid_span(&decl.name)))?;
                (decl.name, ApiItemKind::Type, decl.span, declaration)
            }
            Item::Sig(decl) => {
                let declaration = declaration_at(&source, decl.span)
                    .ok_or_else(|| error_at(decl.span, invalid_span(&decl.name)))?;
                (decl.name, ApiItemKind::Value, decl.span, declaration)
            }
            // An executable module's public surface is its `let`s; the
            // reference shows the SIGNATURE, not the implementation, so it is
            // rebuilt from the definition's own annotations.
            Item::Let(decl) if parse_as == Parse::Implementation => {
                let declaration =
                    signature_of(&source, &decl).map_err(|message| error_at(decl.span, message))?;
                (decl.name, ApiItemKind::Value, decl.span, declaration)
            }
            // A `unit` is public API: it is what makes `90deg` mean
            // `Angle.degrees(90.0)`, so it documents beside the function it
            // calls, quoted verbatim from the source.
            Item::Unit(decl) => {
                let declaration = declaration_at(&source, decl.span)
                    .ok_or_else(|| error_at(decl.span, invalid_span(&decl.suffix)))?;
                (decl.suffix, ApiItemKind::Unit, decl.span, declaration)
            }
            // An operator on a unit's brand is public API too — it is what
            // makes `90deg + 45deg` mean anything — and it documents beside
            // the implementation it names.
            Item::UnitOp(decl) => {
                let declaration = declaration_at(&source, decl.span)
                    .ok_or_else(|| error_at(decl.span, invalid_span(&decl.suffix)))?;
                let name = format!("{} ({})", decl.suffix, decl.op.symbol());
                (name, ApiItemKind::UnitOperator, decl.span, declaration)
            }
            Item::Let(_) | Item::Open(_) | Item::Expect(_) | Item::Module(_) => continue,
        };
        items.push(ApiItem {
            qualified_name: format!("{name}.{item_name}"),
            name: item_name,
            kind,
            declaration,
            docs: public_doc_comment_in_source(&source, span),
        });
    }
    Ok(ApiModule {
        name,
        group,
        // Stamped by `categorize` once the whole inventory is known.
        category: String::new(),
        extension: match parse_as {
            Parse::Interface => "funi",
            Parse::Implementation => "fun",
        },
        docs: module_doc(&source),
        items,
    })
}

fn invalid_span(name: &str) -> String {
    format!("invalid source span for `{name}`")
}

/// The `let name : Type` line for a definition in an executable module, built
/// from the annotations the definition itself carries. Each type is quoted
/// VERBATIM from the source (every `TypeName` knows its own span), so the
/// published signature cannot disagree with the code it documents.
///
/// A public definition therefore has to be fully annotated; an unannotated one
/// is an error rather than a silently vague entry.
fn signature_of(source: &str, decl: &functor_lang::ast::LetDecl) -> Result<String, String> {
    if let Some(ty) = &decl.ty {
        return Ok(format!("let {} : {}", decl.name, type_text(source, ty)?));
    }
    let ExprKind::Lambda { params, ret, .. } = &decl.value.kind else {
        return Err(format!(
            "`{}` needs a type annotation to appear in the API reference",
            decl.name
        ));
    };
    let mut rendered = Vec::with_capacity(params.len());
    for param in params {
        let ty = param.ty.as_ref().ok_or_else(|| {
            format!(
                "`{}`'s parameter `{}` needs a type annotation to appear in the \
                 API reference",
                decl.name, param.name
            )
        })?;
        rendered.push(type_text(source, ty)?);
    }
    let ret = ret.as_ref().ok_or_else(|| {
        format!(
            "`{}` needs a return type annotation to appear in the API reference",
            decl.name
        )
    })?;
    Ok(format!(
        "let {} : ({}) => {}",
        decl.name,
        rendered.join(", "),
        type_text(source, ret)?
    ))
}

fn type_text(source: &str, ty: &TypeName) -> Result<String, String> {
    source
        .get(ty.span.start..ty.span.end)
        .map(|text| normalize_newlines(text.trim()))
        .ok_or_else(|| format!("invalid source span for the type `{}`", ty.name))
}

fn declaration_at(source: &str, span: Span) -> Option<String> {
    source
        .get(span.start..span.end)
        .map(|text| normalize_newlines(text.trim()))
}

fn normalize_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn module_doc(source: &str) -> Option<String> {
    let mut lines = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("//!") {
            lines.push(rest.strip_prefix(' ').unwrap_or(rest));
        } else if trimmed.is_empty() && !lines.is_empty() {
            lines.push("");
        } else if lines.is_empty() && trimmed.is_empty() {
            continue;
        } else {
            break;
        }
    }
    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::{
        generate, generate_from_modules, render_markdown, ApiGroup, ApiItemKind, ApiModule,
    };

    #[test]
    fn extracts_module_and_item_docs_with_exact_declarations() {
        let source = "//! Widgets for tests.\n\
                      \n\
                      /// An opaque widget.\n\
                      type t\n\
                      \n\
                      /// Make one.\n\
                      let make : (float) => t\n";
        let reference =
            generate_from_modules([("Widget".to_string(), source.to_string())]).unwrap();
        let module = &reference.modules[0];
        assert_eq!(module.docs.as_deref(), Some("Widgets for tests."));
        assert_eq!(module.items.len(), 2);
        assert!(matches!(module.items[0].kind, ApiItemKind::Type));
        assert_eq!(module.items[1].qualified_name, "Widget.make");
        assert_eq!(module.items[1].declaration, "let make : (float) => t");
        assert_eq!(module.items[1].docs.as_deref(), Some("Make one."));

        let markdown = render_markdown(&reference);
        assert!(markdown.contains("#### Widget"));
        assert!(markdown.contains("##### `Widget.make`"));
    }

    /// Both halves of the embedded API are complete and fully documented. The
    /// counts are inventory pins: a module or declaration dropping out of the
    /// reference has to be a deliberate edit here, not a silent regression.
    #[test]
    fn embedded_api_is_a_complete_documented_surface() {
        let reference = generate().unwrap();
        let count = |group: ApiGroup| {
            let modules: Vec<&ApiModule> = reference
                .modules
                .iter()
                .filter(|module| module.group == group)
                .collect();
            let items: usize = modules.iter().map(|module| module.items.len()).sum();
            (modules.len(), items)
        };
        assert_eq!(count(ApiGroup::Engine), (28, 311));
        assert_eq!(count(ApiGroup::Stdlib), (10, 97));
        assert!(reference
            .modules
            .iter()
            .any(|module| module.name == "Scene" && module.group == ApiGroup::Engine));
        assert_eq!(
            reference.undocumented(),
            Vec::<String>::new(),
            "the embedded public API must stay fully documented"
        );
    }

    /// Every language-owned module reaches the reference, and the two groups
    /// stay contiguous so the rendered output has one heading each.
    #[test]
    fn the_standard_library_is_documented_beside_the_engine() {
        let reference = generate().unwrap();
        let groups: Vec<ApiGroup> = reference
            .modules
            .iter()
            .map(|module| module.group)
            .collect();
        let switches = groups.windows(2).filter(|pair| pair[0] != pair[1]).count();
        assert_eq!(switches, 1, "modules must be grouped, not interleaved");

        let stdlib: Vec<&str> = reference
            .modules
            .iter()
            .filter(|module| module.group == ApiGroup::Stdlib)
            .map(|module| module.name.as_str())
            .collect();
        assert_eq!(
            stdlib,
            [
                "List", "Map", "Text", "Math", "Random", "Option", "Result", "Key", "Mouse",
                "Debug"
            ]
        );
    }

    /// Every module renders under a category, categories stay contiguous
    /// within their group, and the taxonomy is the one in `CATEGORIES`.
    #[test]
    fn every_module_lands_in_a_category() {
        let reference = generate().unwrap();
        assert!(
            reference
                .modules
                .iter()
                .all(|module| !module.category.is_empty()),
            "every module carries a category"
        );

        let mut seen: Vec<(ApiGroup, &str)> = Vec::new();
        for module in &reference.modules {
            let key = (module.group, module.category.as_str());
            if seen.last() != Some(&key) {
                assert!(
                    !seen.contains(&key),
                    "{key:?} is split across the page — categories must be contiguous"
                );
                seen.push(key);
            }
        }
        assert_eq!(
            seen,
            super::CATEGORIES
                .iter()
                .map(|(group, category, _)| (*group, *category))
                .collect::<Vec<_>>()
        );
    }

    /// A module the taxonomy does not mention is a hard generation error, so
    /// a new module cannot silently land uncategorized.
    #[test]
    fn an_uncategorized_module_fails_generation() {
        let widget = super::extract_module(
            "Widget".to_string(),
            "//! Widgets.\n/// An opaque widget.\ntype t\n".to_string(),
            ApiGroup::Engine,
            super::Parse::Interface,
        )
        .unwrap();
        let error = super::categorize(vec![widget], &[]).expect_err("Widget has no category");
        assert!(
            error.to_string().contains("`Widget` has no category"),
            "unexpected error: {error}"
        );
    }

    /// A group announces itself once, so its categories cannot be split
    /// around another group's.
    #[test]
    fn a_group_split_across_the_table_fails_generation() {
        let error = super::categorize(
            Vec::new(),
            &[
                (ApiGroup::Engine, "Scene & rendering", &[]),
                (ApiGroup::Stdlib, "Collections", &[]),
                (ApiGroup::Engine, "Audio", &[]),
            ],
        )
        .expect_err("Engine is split around Stdlib");
        assert!(
            error.to_string().contains("must be contiguous"),
            "unexpected error: {error}"
        );
    }

    /// A module named twice says so, instead of pointing at the prelude.
    #[test]
    fn a_module_listed_twice_fails_generation() {
        let widget = super::extract_module(
            "Widget".to_string(),
            "//! Widgets.\n/// An opaque widget.\ntype t\n".to_string(),
            ApiGroup::Engine,
            super::Parse::Interface,
        )
        .unwrap();
        let error = super::categorize(
            vec![widget],
            &[
                (ApiGroup::Engine, "Scene & rendering", &["Widget"]),
                (ApiGroup::Engine, "Audio", &["Widget"]),
            ],
        )
        .expect_err("Widget is listed twice");
        assert!(
            error.to_string().contains("listed twice"),
            "unexpected error: {error}"
        );
    }

    /// And the table cannot outlive its modules either: a category naming a
    /// module that is no longer documented fails just as loudly.
    #[test]
    fn a_category_naming_a_missing_module_fails_generation() {
        let error = super::categorize(
            Vec::new(),
            &[(ApiGroup::Engine, "Scene & rendering", &["Scene"])],
        )
        .expect_err("Scene is not documented");
        assert!(
            error.to_string().contains("no such module is documented"),
            "unexpected error: {error}"
        );
    }

    /// A `.fun` module's entries are its SIGNATURES, taken from the
    /// definition's own annotations rather than its body.
    #[test]
    fn executable_modules_publish_signatures_not_bodies() {
        let reference = generate().unwrap();
        let option = reference
            .modules
            .iter()
            .find(|module| module.name == "Option")
            .expect("Option is documented");
        let map = option
            .items
            .iter()
            .find(|item| item.name == "map")
            .expect("Option.map is documented");
        assert_eq!(
            map.declaration,
            "let map : (('value) => 'mapped, t<'value>) => t<'mapped>"
        );
        assert!(matches!(map.kind, ApiItemKind::Value));
    }

    /// A public definition without annotations cannot be documented honestly,
    /// so it is a generation error rather than a vague entry.
    #[test]
    fn unannotated_definitions_fail_generation() {
        let error = super::extract_module(
            "Widget".to_string(),
            "//! Widgets.\n/// Make one.\nlet make = (size) => size\n".to_string(),
            ApiGroup::Stdlib,
            super::Parse::Implementation,
        )
        .expect_err("an unannotated parameter cannot be documented");
        assert!(
            error.to_string().contains("parameter `size`"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn reports_undocumented_modules_and_items() {
        let reference = generate_from_modules([(
            "Widget".to_string(),
            "type t\n/// Make one.\nlet make : () => t\n".to_string(),
        )])
        .unwrap();
        assert_eq!(
            reference.undocumented(),
            vec!["Widget".to_string(), "Widget.t".to_string()]
        );
    }

    #[test]
    fn empty_public_comment_markers_do_not_count_as_documentation() {
        let reference = generate_from_modules([(
            "Widget".to_string(),
            "//!   \n///\ntype t\n///    \nlet make : () => t\n".to_string(),
        )])
        .unwrap();
        assert_eq!(
            reference.undocumented(),
            vec![
                "Widget".to_string(),
                "Widget.t".to_string(),
                "Widget.make".to_string()
            ]
        );
    }

    #[test]
    fn freshness_accepts_windows_line_endings() {
        let path = std::env::temp_dir().join(format!("functor-docgen-crlf-{}", std::process::id()));
        std::fs::write(&path, "one\r\ntwo\r\n").unwrap();
        assert!(super::file_is_current(&path, "one\ntwo\n").unwrap());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn declarations_are_identical_across_source_line_endings() {
        let lf = "//! Points.\n/// A point.\ntype Point = {\n  x: float,\n  y: float\n}\n";
        let crlf = lf.replace('\n', "\r\n");
        let lf_reference =
            generate_from_modules([("Geometry".to_string(), lf.to_string())]).unwrap();
        let crlf_reference = generate_from_modules([("Geometry".to_string(), crlf)]).unwrap();
        assert_eq!(
            lf_reference.modules[0].items[0].declaration,
            crlf_reference.modules[0].items[0].declaration
        );
        assert!(!crlf_reference.modules[0].items[0]
            .declaration
            .contains('\r'));
    }
}
