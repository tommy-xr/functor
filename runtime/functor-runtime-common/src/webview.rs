//! The game-authored HTML/CSS webview overlay — the serializable tree behind
//! the `webview : model => Html.node` entry point.
//!
//! Games build this tree with the `Html.*` / `Attr.*` prelude (Elm-style:
//! `Html.div([Attr.class("hud")], [...])`); the shells render it:
//!
//! - **native** — serialized to an HTML string ([`HtmlNode::to_html`]) and fed
//!   to blitz (Stylo + Taffy + Parley), CPU-painted to an RGBA buffer, and
//!   composited as a GL texture over the 3D frame.
//! - **wasm** — the same HTML string becomes `innerHTML` of a DOM overlay div
//!   above the canvas; the browser is the renderer.
//!
//! Interaction mirrors the egui `ui` path exactly (see [`crate::ui`]):
//! `Attr.onClick(msg)` / `Attr.onInput(tagger)` register their handler in the
//! per-frame table during `webview(model)` evaluation and stamp the node with
//! the slot index, serialized as a `data-fn-click="N"` / `data-fn-input="N"`
//! attribute. The shell reports interactions as `UiEvent { slot, kind }` via
//! `GameProducer::webview_event`; the handler `Value` itself never crosses,
//! so the tree stays serializable/inspectable.

use serde::{Deserialize, Serialize};

/// A node in the webview tree. Only names, strings and slot indexes — never
/// closures or bytes — so it round-trips as JSON and survives time travel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum HtmlNode {
    /// A text node. Escaped on serialization (except inside `<style>`).
    Text(String),
    Element {
        /// Lowercase tag name (`div`, `button`, `style`, ...). Author-controlled;
        /// the webview renders only what the game emits, there is no untrusted input.
        tag: String,
        /// Plain string attributes in emission order (`class`, `style`, `id`, ...).
        attrs: Vec<(String, String)>,
        /// Handler-table slot for a click on this element (`Attr.onClick`).
        click_slot: Option<u32>,
        /// Handler-table slot for an input edit on this element (`Attr.onInput`).
        input_slot: Option<u32>,
        children: Vec<HtmlNode>,
    },
}

/// Void elements: serialized without children or a closing tag.
fn is_void(tag: &str) -> bool {
    matches!(tag, "input" | "br" | "hr" | "img")
}

fn escape_text(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
}

fn escape_attr(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            _ => out.push(c),
        }
    }
}

impl HtmlNode {
    /// Serialize the tree to an HTML string — the shared wire format for both
    /// shells. Handler slots become `data-fn-click` / `data-fn-input`
    /// attributes so events can be routed back by walking the bubble chain.
    pub fn to_html(&self) -> String {
        let mut out = String::new();
        self.write_html(&mut out, false);
        out
    }

    fn write_html(&self, out: &mut String, raw_text: bool) {
        match self {
            HtmlNode::Text(text) => {
                if raw_text {
                    // Inside <style>: CSS needs `>` selectors etc. verbatim.
                    out.push_str(text);
                } else {
                    escape_text(text, out);
                }
            }
            HtmlNode::Element {
                tag,
                attrs,
                click_slot,
                input_slot,
                children,
            } => {
                out.push('<');
                out.push_str(tag);
                for (name, value) in attrs {
                    out.push(' ');
                    out.push_str(name);
                    out.push_str("=\"");
                    escape_attr(value, out);
                    out.push('"');
                }
                if let Some(slot) = click_slot {
                    out.push_str(&format!(" data-fn-click=\"{slot}\""));
                }
                if let Some(slot) = input_slot {
                    out.push_str(&format!(" data-fn-input=\"{slot}\""));
                }
                out.push('>');
                if is_void(tag) {
                    return;
                }
                let raw = tag == "style";
                for child in children {
                    child.write_html(out, raw);
                }
                out.push_str("</");
                out.push_str(tag);
                out.push('>');
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn el(tag: &str, attrs: &[(&str, &str)], children: Vec<HtmlNode>) -> HtmlNode {
        HtmlNode::Element {
            tag: tag.to_string(),
            attrs: attrs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            click_slot: None,
            input_slot: None,
            children,
        }
    }

    #[test]
    fn serializes_nested_elements_with_attrs() {
        let tree = el(
            "div",
            &[("class", "hud")],
            vec![el("span", &[], vec![HtmlNode::Text("Score: 7".into())])],
        );
        assert_eq!(
            tree.to_html(),
            r#"<div class="hud"><span>Score: 7</span></div>"#
        );
    }

    #[test]
    fn escapes_text_and_attrs() {
        let tree = el(
            "div",
            &[("title", "a\"b<c")],
            vec![HtmlNode::Text("1 < 2 & 3".into())],
        );
        assert_eq!(
            tree.to_html(),
            r#"<div title="a&quot;b&lt;c">1 &lt; 2 &amp; 3</div>"#
        );
    }

    #[test]
    fn style_text_is_raw() {
        let tree = el(
            "style",
            &[],
            vec![HtmlNode::Text(".a > .b { color: red; }".into())],
        );
        assert_eq!(tree.to_html(), "<style>.a > .b { color: red; }</style>");
    }

    #[test]
    fn handler_slots_become_data_attributes() {
        let tree = HtmlNode::Element {
            tag: "button".into(),
            attrs: vec![("class".into(), "go".into())],
            click_slot: Some(3),
            input_slot: None,
            children: vec![HtmlNode::Text("Go".into())],
        };
        assert_eq!(
            tree.to_html(),
            r#"<button class="go" data-fn-click="3">Go</button>"#
        );
    }

    #[test]
    fn void_elements_have_no_closing_tag() {
        let tree = HtmlNode::Element {
            tag: "input".into(),
            attrs: vec![("value".into(), "hi".into())],
            click_slot: None,
            input_slot: Some(5),
            children: vec![],
        };
        assert_eq!(tree.to_html(), r#"<input value="hi" data-fn-input="5">"#);
    }
}
