//! `webview_bench` — a headless benchmark of the native webview overlay's
//! CPU costs: blitz parse/resolve (per model-driven re-render) and CPU paint
//! (per repaint: hover transitions, presses, content changes).
//!
//! The `frame_bench` philosophy (see functor-runtime-common/examples): no GL,
//! no window — the exact blitz calls `webview_overlay` makes, timed
//! back-to-back; min + median over a FIXED sample count so both sides of an
//! A/B draw from the same distribution. Report-only, no CI gate.
//!
//! The workload is a hermetic copy of `examples/webview`'s counter card
//! (deliberately NOT the live example file — it can change under the bench),
//! serialized exactly as `HtmlNode::to_html` emits it (data-fn-* slots
//! included).
//!
//! Reference numbers (M-series Mac, release): parse+resolve ~1ms with the
//! shared FontContext (~30ms with a fresh one — the regression this bench
//! exists to catch), paint ~1ms at 800x600 and ~3ms at a 2048x1536 retina
//! framebuffer. Debug paint is ~200x slower (~0.6s at retina size), which is
//! why interactive native testing uses release builds.
//!
//! ```sh
//! cargo run -q --release -p functor-runtime-desktop --example webview_bench
//! ```

use anyrender::ImageRenderer;
use anyrender_vello_cpu::VelloCpuImageRenderer;
use blitz_dom::{BaseDocument, DocumentConfig, FontContext};
use blitz_html::HtmlDocument;
use blitz_traits::shell::{ColorScheme, Viewport};
use std::time::Instant;

/// The counter card from examples/webview, as `HtmlNode::to_html` serializes
/// it (handler slots as data attributes).
const HTML: &str = r#"<div><style>
  .hud { display: flex; flex-direction: column; gap: 12px; width: 300px;
         margin: 24px; padding: 20px;
         background: linear-gradient(135deg, rgba(24, 26, 44, 0.92), rgba(52, 24, 64, 0.92));
         border: 2px solid #8be9fd; border-radius: 14px;
         font-family: sans-serif; color: #f8f8f2; }
  .hud h1 { margin: 0; font-size: 22px; color: #8be9fd; }
  .count { font-size: 40px; font-weight: bold; text-align: center; }
  .row { display: flex; gap: 10px; justify-content: center; }
  button { padding: 8px 18px; font-size: 18px; font-weight: bold;
           background: #50fa7b; color: #1e1e3c; border: none; border-radius: 8px; }
  button:hover { background: #f1fa8c; }
  button.ghost { background: transparent; color: #8be9fd; border: 1px solid #8be9fd; }
</style><div class="hud"><h1>Functor webview</h1><div class="count">7</div><div class="row"><button data-fn-click="0">-</button><button data-fn-click="1">+</button><button class="ghost" data-fn-click="2">Reset</button></div></div></div>"#;

/// (framebuffer size, hidpi scale) — a small window, a common 1x window, and
/// a retina-sized framebuffer (the case that made debug builds unusable).
const SIZES: &[(u32, u32, f32)] = &[
    (800, 600, 1.0),
    (1280, 720, 1.0),
    (2048, 1536, 2.0),
];

const PARSE_SAMPLES: usize = 100;
const PAINT_SAMPLES: usize = 40;
const HIT_SAMPLES: usize = 2000;

fn build_doc(w: u32, h: u32, scale: f32, font_ctx: &FontContext) -> BaseDocument {
    let mut doc = HtmlDocument::from_html(
        HTML,
        DocumentConfig {
            viewport: Some(Viewport::new(w, h, scale, ColorScheme::Dark)),
            font_ctx: Some(font_ctx.clone()),
            ..Default::default()
        },
    )
    .into_inner();
    doc.resolve(0.0);
    doc
}

/// min + median of `samples` timed runs of `f`, in microseconds.
fn time_us(samples: usize, mut f: impl FnMut()) -> (f64, f64) {
    let mut us: Vec<f64> = (0..samples)
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed().as_secs_f64() * 1e6
        })
        .collect();
    us.sort_by(|a, b| a.total_cmp(b));
    (us[0], us[samples / 2])
}

fn main() {
    #[cfg(debug_assertions)]
    eprintln!(
        "warning: webview_bench in a DEBUG build — paint is ~200x release; \
numbers are not comparable. Re-run with --release."
    );

    let font_ctx = FontContext::default();
    // Warm the shared font context once (first parse pays font enumeration,
    // like the overlay's first frame).
    let _ = build_doc(800, 600, 1.0, &font_ctx);

    println!(
        "{:<18} {:>22} {:>22}",
        "scenario", "min", "median"
    );
    for &(w, h, scale) in SIZES {
        let label = format!("{w}x{h}@{scale}");

        // Per re-render cost: parse + resolve with the SHARED font context
        // (the shipping config — a fresh context re-scans system fonts).
        let (min, med) = time_us(PARSE_SAMPLES, || {
            let _ = build_doc(w, h, scale, &font_ctx);
        });
        println!("{label:<18} {:>14.0} us parse {:>14.0} us", min, med);

        // Per repaint cost: rasterize the resolved doc (hover transitions,
        // presses, and content changes each pay one of these).
        let mut doc = build_doc(w, h, scale, &font_ctx);
        let mut renderer = VelloCpuImageRenderer::new(w, h);
        let mut buf = Vec::new();
        let (min, med) = time_us(PAINT_SAMPLES, || {
            renderer.render_to_vec(
                |scene| {
                    blitz_paint::paint_scene(scene, &mut doc, scale as f64, w, h, 0, 0)
                },
                &mut buf,
            );
        });
        println!("{label:<18} {:>14.0} us paint {:>14.0} us", min, med);
    }

    // Per-press hit-test cost (the run loop's synchronous click arbitration).
    let doc = build_doc(1280, 720, 1.0, &font_ctx);
    let (min, med) = time_us(HIT_SAMPLES, || {
        // Over the "+" button and over empty space.
        std::hint::black_box(doc.hit(180.0, 180.0));
        std::hint::black_box(doc.hit(900.0, 500.0));
    });
    println!("{:<18} {:>14.2} us hit   {:>14.2} us", "hit-test x2", min, med);
}
