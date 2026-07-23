//! Doc-comment extraction for hover: the contiguous `//` comment block
//! sitting DIRECTLY above a definition (no blank line between), rendered as
//! prose. This is how the prelude `.funi` files' rich comments surface in
//! the editor — hover on `Scene.model` shows the interface's own
//! documentation — and user code gets the same treatment for free (a
//! comment block right above a `let` shows on hover of its references).
//!
//! Comments are discarded at lex time, so this works from the SOURCE TEXT:
//! given a definition span (a [`crate::goto::definition_span`] target), find
//! the owning file in the [`SourceMap`] and walk back over whole lines. A
//! blank line ends the block — section headers separated from the
//! declarations they group (the `.funi` house style) deliberately do not
//! attach.

use crate::project::SourceMap;
use crate::Span;

/// The doc-comment block directly above `span`, with `//` markers stripped,
/// joined by newlines. `None` when no comment line immediately precedes the
/// definition (or the span falls outside the map's files).
pub fn doc_comment(sources: &SourceMap, span: Span) -> Option<String> {
    let file = sources.file_at(span.start);
    let local = span.start.checked_sub(file.base)?;
    doc_comment_in_source(&file.src, Span::new(local, local))
}

/// The doc-comment block directly above a file-local `span`.
///
/// This is the source-only half of [`doc_comment`], shared with tooling that
/// parses standalone `.funi` text without constructing a whole [`SourceMap`].
pub fn doc_comment_in_source(source: &str, span: Span) -> Option<String> {
    comment_block_in_source(source, span, false)
}

/// The explicit public-doc block (`///`) directly above a file-local `span`.
///
/// Unlike [`doc_comment_in_source`], ordinary `//` comments are excluded:
/// generated API references use this stricter contract so section headings
/// and implementation notes cannot be misattributed to one member.
pub fn public_doc_comment_in_source(source: &str, span: Span) -> Option<String> {
    comment_block_in_source(source, span, true)
}

fn comment_block_in_source(source: &str, span: Span, public_only: bool) -> Option<String> {
    let local = span.start;
    if local > source.len() {
        return None;
    }
    // `get` rather than a slice: definition spans from the parser always sit
    // on char boundaries, but the span is caller-supplied public API.
    let above = source.get(..local)?;

    let mut lines: Vec<&str> = Vec::new();
    let mut fragments = above.lines().rev();
    // `above` usually ends mid-line: the definition line's leading text
    // (indentation, or empty for a top-level `let`). Skip that fragment —
    // it belongs to the definition, not to what precedes it — unless the
    // slice ended exactly on a line break.
    if !above.is_empty() && !above.ends_with('\n') {
        let fragment = fragments.next()?;
        if !fragment.trim().is_empty() {
            // The definition shares its line with real code — no doc block.
            return None;
        }
    }
    for line in fragments {
        let trimmed = line.trim_start();
        let rest = if public_only {
            trimmed.strip_prefix("///")
        } else {
            trimmed.strip_prefix("//")
        };
        match rest {
            Some(rest) => {
                let rest = if public_only {
                    rest
                } else {
                    // `///` is the explicit public-doc spelling. Plain `//`
                    // remains supported for existing Functor code and hovers.
                    rest.strip_prefix('/').unwrap_or(rest)
                };
                lines.push(rest.strip_prefix(' ').unwrap_or(rest));
            }
            None => break,
        }
    }
    if lines.is_empty() {
        return None;
    }
    lines.reverse();
    Some(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::{doc_comment, doc_comment_in_source, public_doc_comment_in_source};
    use crate::project::load_single_source;
    use crate::Span;

    /// The entry's own defs: a comment block right above a `let` attaches;
    /// a blank line breaks attachment.
    #[test]
    fn user_defs_attach_their_comment_block() {
        let src = "// The player's walking speed,\n\
                   // in units per second.\n\
                   let speed = 4.0\n\
                   \n\
                   // A section header, separated by a blank line.\n\
                   \n\
                   let other = 1.0\n";
        let project =
            load_single_source("game", src).unwrap_or_else(|e| panic!("loads: {}", e.render()));
        let speed = project
            .module
            .defs
            .iter()
            .find(|d| d.name == "speed")
            .expect("speed def");
        assert_eq!(
            doc_comment(&project.sources, speed.span).as_deref(),
            Some("The player's walking speed,\nin units per second.")
        );
        let other = project
            .module
            .defs
            .iter()
            .find(|d| d.name == "other")
            .expect("other def");
        assert_eq!(doc_comment(&project.sources, other.span), None);
    }

    /// Injected interface signatures: a `.funi` module's per-signature
    /// comment blocks attach through the synthetic source, and a signature
    /// without a directly-preceding block stays bare rather than stealing an
    /// earlier one.
    #[test]
    fn interface_signatures_attach_their_funi_docs() {
        let funi = "// The opaque widget handle.\n\
                    type t\n\
                    \n\
                    // Make a widget of the given size,\n\
                    // in pixels.\n\
                    let make : (float) => t\n\
                    let size : (t) => float\n";
        let project = crate::project::load_sources_with_prelude(
            vec![(
                std::path::PathBuf::from("game.fun"),
                "let x = 0.0\n".to_string(),
            )],
            &[("Widget".to_string(), funi.to_string())],
        )
        .unwrap_or_else(|e| panic!("loads: {}", e.render()));
        let sig = |name: &str| {
            project
                .module
                .signatures
                .iter()
                .find(|s| s.name == name)
                .unwrap_or_else(|| panic!("no signature {name}"))
                .span
        };
        assert_eq!(
            doc_comment(&project.sources, sig("Widget.make")).as_deref(),
            Some("Make a widget of the given size,\nin pixels.")
        );
        assert_eq!(doc_comment(&project.sources, sig("Widget.size")), None);
    }

    /// Bundled `.fun` implementations retain their own API documentation,
    /// just like project definitions and prelude signatures.
    #[test]
    fn standard_library_defs_attach_their_source_docs() {
        let project =
            load_single_source("game", "let main = () => Option.None\n")
                .unwrap_or_else(|e| panic!("loads: {}", e.render()));
        let map = project
            .module
            .defs
            .iter()
            .find(|def| def.name == "Option.map")
            .expect("Option.map definition");
        assert_eq!(
            doc_comment(&project.sources, map.span).as_deref(),
            Some("Transform a present value; leave `None` unchanged.")
        );
    }

    #[test]
    fn source_only_extraction_understands_explicit_doc_comments() {
        let src = "/// Make a widget.\nlet make : () => t\n";
        let start = src.find("let make").unwrap();
        assert_eq!(
            doc_comment_in_source(src, Span::new(start, start)).as_deref(),
            Some("Make a widget.")
        );
    }

    #[test]
    fn public_extraction_excludes_ordinary_comments() {
        let src = "// A section heading.\nlet make : () => t\n";
        let start = src.find("let make").unwrap();
        assert_eq!(
            public_doc_comment_in_source(src, Span::new(start, start)),
            None
        );
        assert_eq!(
            doc_comment_in_source(src, Span::new(start, start)).as_deref(),
            Some("A section heading.")
        );
    }
}
