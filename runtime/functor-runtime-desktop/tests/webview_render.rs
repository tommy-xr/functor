//! Headless webview-render test: the blitz pipeline (parse → Stylo/Taffy/
//! Parley → CPU paint) turns a styled card into the expected pixels with NO
//! GL context or window — the agent-verifiable core of the native webview
//! overlay (`webview_overlay::render_html_to_rgba` is the exact code the
//! GL path uploads as a texture).
#![cfg(not(target_arch = "wasm32"))]

use functor_runtime_desktop::webview_overlay::render_html_to_rgba;

const HTML: &str = r#"
<html><head><style>
  html, body { margin: 0; background: transparent; }
  .card { width: 200px; height: 100px; margin: 50px; background: rgb(255, 0, 0); }
</style></head>
<body><div class="card"></div></body></html>
"#;

fn px(buf: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * w + x) * 4) as usize;
    [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
}

#[test]
fn styled_card_renders_to_expected_pixels() {
    let (w, h) = (400, 300);
    let buf = render_html_to_rgba(HTML, w, h, 1.0);
    assert_eq!(buf.len(), (w * h * 4) as usize);

    // Inside the card (50..250 x, 50..150 y): opaque red.
    assert_eq!(px(&buf, w, 150, 100), [255, 0, 0, 255]);
    // Outside the card: fully transparent — the 3D scene must show through.
    assert_eq!(px(&buf, w, 350, 250), [0, 0, 0, 0]);
    assert_eq!(px(&buf, w, 10, 10), [0, 0, 0, 0]);
}

#[test]
fn webview_tree_serializes_and_renders() {
    // End-to-end through the shared tree type: HtmlNode → to_html → pixels.
    use functor_runtime_common::webview::HtmlNode;
    let tree = HtmlNode::Element {
        tag: "div".into(),
        attrs: vec![(
            "style".into(),
            "width: 100px; height: 100px; background: rgb(0, 255, 0);".into(),
        )],
        click_slot: Some(0),
        input_slot: None,
        children: vec![],
    };
    let buf = render_html_to_rgba(&tree.to_html(), 200, 200, 1.0);
    // Body default margin is 8px — sample well inside the square.
    assert_eq!(px(&buf, 200, 50, 50), [0, 255, 0, 255]);
    assert_eq!(px(&buf, 200, 150, 150), [0, 0, 0, 0]);
}
