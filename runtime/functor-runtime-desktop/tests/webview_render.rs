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

#[test]
fn css_animation_ticks_under_the_clock() {
    // A red square sliding right under an infinite @keyframes animation,
    // driven exactly the way the render worker drives it: ONE retained doc,
    // `resolve(t)` advancing the clock, repaint while `doc.is_animating()`
    // (the animation's start anchors at the doc's first resolve, so a fresh
    // doc per t would always render the from-pose).
    use anyrender::ImageRenderer;
    use anyrender_vello_cpu::VelloCpuImageRenderer;
    use blitz_dom::DocumentConfig;
    use blitz_html::HtmlDocument;
    use blitz_traits::shell::{ColorScheme, Viewport};

    const ANIM_HTML: &str = r#"
<html><head><style>
  html, body { margin: 0; background: transparent; }
  @keyframes slide { from { margin-left: 0px; } to { margin-left: 200px; } }
  .box { width: 100px; height: 100px; background: rgb(255, 0, 0);
         animation: slide 1s linear infinite; }
</style></head>
<body><div class="box"></div></body></html>
"#;

    let (w, h) = (400, 200);
    let mut doc = HtmlDocument::from_html(
        ANIM_HTML,
        DocumentConfig {
            viewport: Some(Viewport::new(w, h, 1.0, ColorScheme::Dark)),
            ..Default::default()
        },
    )
    .into_inner();
    let mut renderer = VelloCpuImageRenderer::new(w, h);
    let mut paint_at = |doc: &mut blitz_dom::BaseDocument, t: f64| {
        doc.resolve(t);
        // The render context accumulates across paints — reset, like the
        // worker does, or the moved box ghosts its old position.
        renderer.reset();
        let mut buf = Vec::new();
        renderer.render_to_vec(
            |scene| blitz_paint::paint_scene(scene, doc, 1.0, w, h, 0, 0),
            &mut buf,
        );
        buf
    };

    // t=0: the box sits at the left edge; t=0.5: it slid 100px right.
    let at0 = paint_at(&mut doc, 0.0);
    // The worker's repaint-while-animating condition.
    assert!(doc.is_animating(), "infinite @keyframes must report animating");
    let at_half = paint_at(&mut doc, 0.5);
    assert_ne!(at0, at_half, "clock advance must change the rendered pixels");
    assert_eq!(px(&at0, w, 50, 50), [255, 0, 0, 255]);
    assert_eq!(px(&at_half, w, 50, 50), [0, 0, 0, 0]);
    assert_eq!(px(&at_half, w, 150, 50), [255, 0, 0, 255]);
}

#[test]
fn fresh_doc_anchored_at_zero_renders_the_clock_pose() {
    // The worker's reparse path resolves a fresh document at 0.0 BEFORE the
    // current clock (webview_overlay::run_cycle): blitz anchors an animation
    // at the first resolve that sees it, so without the zero anchor a doc
    // (re)parsed at clock T would render the from-pose (elapsed 0) — exactly
    // what `--fixed-time T` captures hit, and why a model-driven re-render
    // used to restart running @keyframes. This pins the anchoring semantics
    // the fix relies on across blitz upgrades.
    use anyrender::ImageRenderer;
    use anyrender_vello_cpu::VelloCpuImageRenderer;
    use blitz_dom::DocumentConfig;
    use blitz_html::HtmlDocument;
    use blitz_traits::shell::{ColorScheme, Viewport};

    const ANIM_HTML: &str = r#"
<html><head><style>
  html, body { margin: 0; background: transparent; }
  @keyframes slide { from { margin-left: 0px; } to { margin-left: 200px; } }
  .box { width: 100px; height: 100px; background: rgb(255, 0, 0);
         animation: slide 1s linear infinite; }
</style></head>
<body><div class="box"></div></body></html>
"#;

    let (w, h) = (400, 200);
    let fresh_doc = || {
        HtmlDocument::from_html(
            ANIM_HTML,
            DocumentConfig {
                viewport: Some(Viewport::new(w, h, 1.0, ColorScheme::Dark)),
                ..Default::default()
            },
        )
        .into_inner()
    };
    let paint = |doc: &mut blitz_dom::BaseDocument| {
        let mut renderer = VelloCpuImageRenderer::new(w, h);
        let mut buf = Vec::new();
        renderer.render_to_vec(
            |scene| blitz_paint::paint_scene(scene, doc, 1.0, w, h, 0, 0),
            &mut buf,
        );
        buf
    };

    // First resolve AT the clock (the old behavior): anchored at 0.5, so the
    // pose is the from-pose — the box sits at the left edge.
    let mut unanchored = fresh_doc();
    unanchored.resolve(0.5);
    let at_anchor = paint(&mut unanchored);
    assert_eq!(px(&at_anchor, w, 50, 50), [255, 0, 0, 255]);

    // Zero-anchored then resolved at the same clock (the worker's sequence):
    // the pose is 0.5s into the animation — the box slid 100px right.
    let mut anchored = fresh_doc();
    anchored.resolve(0.0);
    anchored.resolve(0.5);
    let at_half = paint(&mut anchored);
    assert_eq!(px(&at_half, w, 50, 50), [0, 0, 0, 0]);
    assert_eq!(px(&at_half, w, 150, 50), [255, 0, 0, 255]);
}
