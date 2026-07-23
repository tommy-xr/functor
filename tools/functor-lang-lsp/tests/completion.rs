//! B-layer completion tests: drive `functor_lang::complete::complete` directly
//! against the **real** engine bundle (`functor_prelude::bundled_modules()`) — no
//! server spawn, no framed stdio. The A-layer unit tests in `complete.rs` use
//! inline throwaway preludes for exact-label assertions; here we pin the real
//! `scene.funi` surface so drift in the shipped prelude is caught.

use std::path::PathBuf;

use functor_lang::complete::{complete, CompletionKind};
use functor_lang::project::{load_sources_with_bundled_modules, Project};

/// A minimal, valid single-file project — the last-good parse the broken live
/// buffers complete against. The real prelude gives it `Scene.*`.
const STUB: &str = "let main = () => 1.0";

fn project() -> Project {
    let sources = vec![(PathBuf::from("game.fun"), STUB.to_string())];
    let bundled = functor_prelude::bundled_modules();
    load_sources_with_bundled_modules(sources, &bundled)
        .unwrap_or_else(|e| panic!("project loads: {}", e.render()))
}

fn labels(items: &[functor_lang::complete::CompletionItem]) -> Vec<String> {
    items.iter().map(|i| i.label.clone()).collect()
}

fn find<'a>(
    items: &'a [functor_lang::complete::CompletionItem],
    label: &str,
) -> &'a functor_lang::complete::CompletionItem {
    items
        .iter()
        .find(|i| i.label == label)
        .unwrap_or_else(|| panic!("no `{label}` in {:?}", labels(items)))
}

// Row 1. Real prelude dot-completion `Scene.` — cube/sphere/group present, and
// the exact detail the real `scene.funi` renders for `cube`.
#[test]
fn real_prelude_scene_member_completion() {
    let project = project();
    let live = "let s = Scene.";
    let items = complete(&project, "Game", live, live.len());
    let names = labels(&items);
    for member in ["cube", "sphere", "group"] {
        assert!(
            names.contains(&member.to_string()),
            "missing {member} in {names:?}"
        );
    }
    let cube = find(&items, "cube");
    assert_eq!(cube.detail.as_deref(), Some("Scene.cube : () => Scene.t"));
    assert_eq!(cube.kind, CompletionKind::Function);
}

// Row 6. Partial member `Scene.cu` → `cube` only, from the real prelude.
#[test]
fn real_prelude_partial_member_filters() {
    let project = project();
    let live = "let s = Scene.cu";
    let items = complete(&project, "Game", live, live.len());
    assert_eq!(labels(&items), ["cube"]);
}

#[test]
fn bundled_animator_member_completion() {
    let project = project();
    let live = "let pose = Animator.";
    let items = complete(&project, "Game", live, live.len());
    let names = labels(&items);
    for member in ["start", "play", "pose"] {
        assert!(
            names.contains(&member.to_string()),
            "missing {member} in {names:?}"
        );
    }
    assert!(
        !names.contains(&"smoothstep".to_string()),
        "implementation helper leaked into the public API: {names:?}"
    );
    let pose = find(&items, "pose");
    assert_eq!(pose.kind, CompletionKind::Function);
}
