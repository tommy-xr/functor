//! Map updates consume their map argument, so a uniquely owned map (a
//! pipeline temporary) is edited in place rather than path-copied. Measured
//! with a counting allocator; one test per binary keeps the count clean.

use functor_lang::{NoHost, Session};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Counting;
static BYTES: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

/// Bytes allocated per update, from the difference of 10 and 30 updates.
fn bytes_per_update(session: &Session, name: &str) -> usize {
    let measure = |n: usize| {
        let before = BYTES.load(Ordering::Relaxed);
        session
            .call(&format!("{name}{n}"), Vec::new(), &mut NoHost)
            .unwrap();
        BYTES.load(Ordering::Relaxed) - before
    };
    (measure(30) - measure(10)) / 20
}

#[test]
fn unique_temporaries_are_updated_in_place() {
    // `chained`: each insert's input is the previous call's result, owned by
    // nobody else after the first copy. `kept`: every version stays bound,
    // so each insert must copy its root-to-leaf path (4 nodes at 6.8k).
    let mut src =
        "let table = Map.fromList(List.range(6800.0) |> List.map((i) => (i, i)))\n".to_string();
    for n in [10, 30] {
        let steps: String = (0..n)
            .map(|i| format!(" |> Map.insert(5.0, {i}.0)"))
            .collect();
        src += &format!("let chained{n} = () => table{steps}\n");
        let mut kept = format!("let kept{n} = () => let m0 = table in ");
        for i in 0..n {
            kept += &format!("let m{} = Map.insert(5.0, {i}.0, m{i}) in ", i + 1);
        }
        src += &format!("{kept}m{n}\n");
    }
    let module = functor_lang::lower(functor_lang::parse(&src).unwrap()).unwrap();
    let Ok(session) = Session::load(&module, &mut NoHost) else {
        panic!("module should load");
    };
    let chained = bytes_per_update(&session, "chained");
    let kept = bytes_per_update(&session, "kept");
    assert!(
        chained * 4 < kept,
        "in-place {chained} B/update should be far below path-copying {kept} B/update"
    );
}
