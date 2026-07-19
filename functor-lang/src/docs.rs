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
    if local > file.src.len() {
        return None;
    }
    // `get` rather than a slice: definition spans from the parser always sit
    // on char boundaries, but the span is caller-supplied public API.
    let above = file.src.get(..local)?;

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
        match trimmed.strip_prefix("//") {
            Some(rest) => lines.push(rest.strip_prefix(' ').unwrap_or(rest)),
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
    use super::doc_comment;
    use crate::project::load_single_source;

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
            vec![(std::path::PathBuf::from("game.fun"), "let x = 0.0\n".to_string())],
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
}
