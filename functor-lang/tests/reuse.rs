//! Reload reuse (`Session::load_reusing`): a reload keeps a top-level value
//! whose code, dependencies, type and host inputs are unchanged, and
//! evaluates everything else. `T.slow` counts its calls, so a reused value is
//! one that didn't call it again.

use functor_lang::project::{load_sources_with_bundled_modules, BundledModule};
use functor_lang::reuse::{def_types, HostInput, ReuseStats};
use functor_lang::{Host, HostData, HostReturn, RunError, Session, Span, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

const T: &str = "type handle = host
let slow : (float) => float
let read : (string) => string
let live : () => float
let handle : () => handle
";

/// `T.slow(x)` = `x * 2` (counted), `T.read(path)` reads `files` (an input),
/// `T.live()` reads live host state, `T.handle()` is an opaque value.
#[derive(Default)]
struct TestHost {
    files: HashMap<String, String>,
    slow_calls: usize,
    live_calls: usize,
    reads: Vec<HostInput>,
    volatile: bool,
}

struct Opaque;

impl HostData for Opaque {
    fn type_name(&self) -> &'static str {
        "Opaque"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

fn identity(text: Option<&String>) -> u64 {
    text.map_or(0, |text| text.len() as u64 * 1000 + text.bytes().map(u64::from).sum::<u64>())
}

impl Host for TestHost {
    fn provides(&self, path: &str) -> bool {
        matches!(path, "T.slow" | "T.read" | "T.live" | "T.handle")
    }

    fn call(&mut self, path: &str, args: Vec<Value>, _span: Span) -> Result<HostReturn, RunError> {
        let value = match (path, args.as_slice()) {
            ("T.slow", [Value::Number(x)]) => {
                self.slow_calls += 1;
                Value::Number(x * 2.0)
            }
            ("T.read", [Value::String(path)]) => {
                let text = self.files.get(path.as_ref());
                self.reads.push(HostInput {
                    key: path.to_string(),
                    identity: identity(text),
                });
                Value::String(text.cloned().unwrap_or_default().into())
            }
            ("T.live", []) => {
                self.live_calls += 1;
                self.volatile = true;
                Value::Number(self.live_calls as f64)
            }
            ("T.handle", []) => Value::HostData(Rc::new(Opaque)),
            _ => panic!("unexpected call {path}"),
        };
        Ok(HostReturn::Value(value))
    }

    fn begin_inputs(&mut self) {
        self.reads.clear();
        self.volatile = false;
    }

    fn end_inputs(&mut self) -> Option<Vec<HostInput>> {
        (!self.volatile).then(|| std::mem::take(&mut self.reads))
    }

    fn inputs_current(&mut self, inputs: &[HostInput]) -> bool {
        inputs
            .iter()
            .all(|input| identity(self.files.get(&input.key)) == input.identity)
    }
}

/// Load `files` (entry first) reusing `previous`; `retype` edits the checked
/// types first (a stand-in for a type-only change).
fn load_with(
    files: &[(&str, &str)],
    host: &mut TestHost,
    previous: Option<&Session>,
    retype: impl FnOnce(&mut HashMap<String, String>),
) -> (Session, ReuseStats) {
    let sources = files
        .iter()
        .map(|(path, src)| (PathBuf::from(path), src.to_string()))
        .collect();
    let project = load_sources_with_bundled_modules(sources, &[BundledModule::interface("T", T)])
        .unwrap_or_else(|e| panic!("{}", e.render()));
    let (errors, types) = project.check_with_types();
    assert!(errors.is_empty(), "{:?}", errors);
    let mut types = def_types(&project.module, &types);
    retype(&mut types);
    Session::load_reusing(&project.module, host, previous, types)
        .unwrap_or_else(|f| panic!("{}", f.error.message))
}

fn load(files: &[(&str, &str)], host: &mut TestHost, previous: Option<&Session>) -> (Session, ReuseStats) {
    load_with(files, host, previous, |_| {})
}

/// `(reused, values)` over every module.
fn all(stats: &ReuseStats) -> (usize, usize) {
    stats.count(|_| true)
}

fn num(session: &Session, name: &str) -> f64 {
    match session.global(name) {
        Some(Value::Number(n)) => n,
        other => panic!("{name}: expected a number, got {:?}", other.map(|v| v.to_string())),
    }
}

const LEVEL: &str = "let data = Build.make(21.0)\n";
const BUILD: &str = "let make = (x) => T.slow(x) + 1.0\n";

fn game(speed: f64) -> String {
    format!("let speed = {speed:?}\nlet level = Level.data\nlet move = (dt) => speed * dt\n")
}

#[test]
fn editing_a_constant_reuses_an_expensive_value() {
    let mut host = TestHost::default();
    let (first, cold) = load(
        &[("game.fun", &game(1.0)), ("level.fun", LEVEL), ("build.fun", BUILD)],
        &mut host,
        None,
    );
    assert_eq!(host.slow_calls, 1);
    assert_eq!(all(&cold), (0, 3));
    let (second, stats) = load(
        &[("game.fun", &game(2.0)), ("level.fun", LEVEL), ("build.fun", BUILD)],
        &mut host,
        Some(&first),
    );
    assert_eq!(host.slow_calls, 1, "the expensive value was evaluated again");
    assert_eq!(num(&second, "level"), 43.0);
    assert_eq!(num(&second, "speed"), 2.0);
    // `Level.data` and `level`; `speed` changed; functions never count.
    assert_eq!(all(&stats), (2, 3));
    // A third load reuses from a session that itself reused.
    let (_, again) = load(
        &[("game.fun", &game(3.0)), ("level.fun", LEVEL), ("build.fun", BUILD)],
        &mut host,
        Some(&second),
    );
    assert_eq!(host.slow_calls, 1);
    assert_eq!(all(&again).0, 2);
}

#[test]
fn editing_the_value_or_a_function_it_calls_evaluates_it() {
    let mut host = TestHost::default();
    let game = game(1.0);
    let (first, _) = load(
        &[("game.fun", &game), ("level.fun", LEVEL), ("build.fun", BUILD)],
        &mut host,
        None,
    );
    let level = "let data = Build.make(10.0)\n";
    let (second, _) = load(
        &[("game.fun", &game), ("level.fun", level), ("build.fun", BUILD)],
        &mut host,
        Some(&first),
    );
    assert_eq!(host.slow_calls, 2);
    assert_eq!(num(&second, "level"), 21.0, "a dependent picks up the new value");
    let build = "let make = (x) => T.slow(x) + 2.0\n";
    let (third, _) = load(
        &[("game.fun", &game), ("level.fun", level), ("build.fun", build)],
        &mut host,
        Some(&second),
    );
    assert_eq!(host.slow_calls, 3, "editing a called function in another module");
    assert_eq!(num(&third, "level"), 22.0);
}

#[test]
fn a_moved_definition_that_keeps_its_code_is_reused() {
    // Spans and ids shift (a comment and a def above), the code doesn't.
    let mut host = TestHost::default();
    let level = "let data = T.slow(1.0)\n";
    let (first, _) = load(&[("game.fun", "let level = Level.data\n"), ("level.fun", level)], &mut host, None);
    let moved = "// the level\nlet unrelated = (a, b) => a + b\n\nlet data = T.slow(1.0)\n";
    let (_, stats) = load(&[("game.fun", "let level = Level.data\n"), ("level.fun", moved)], &mut host, Some(&first));
    assert_eq!(host.slow_calls, 1);
    assert_eq!(all(&stats).0, 2);
}

#[test]
fn a_changed_input_file_evaluates_its_readers_only() {
    let level = "let a = T.slow(Text.length(T.read(\"a.txt\")))\nlet b = T.slow(Text.length(T.read(\"b.txt\")))\n";
    let files = [("game.fun", "let x = 1.0\n"), ("level.fun", level)];
    let mut host = TestHost::default();
    host.files.insert("a.txt".into(), "aaa".into());
    host.files.insert("b.txt".into(), "bb".into());
    let (first, _) = load(&files, &mut host, None);
    assert_eq!(host.slow_calls, 2);
    host.files.insert("a.txt".into(), "aaaaa".into());
    host.files.insert("unrelated.txt".into(), "z".into());
    let (second, _) = load(&files, &mut host, Some(&first));
    assert_eq!(host.slow_calls, 3, "only `a` re-reads");
    assert_eq!(num(&second, "Level.a"), 10.0);
    assert_eq!(num(&second, "Level.b"), 4.0);
    // A file that appears counts as a change too.
    let reader = [("game.fun", "let x = 1.0\n"), ("level.fun", "let c = T.read(\"c.txt\")\n")];
    let (third, _) = load(&reader, &mut host, Some(&second));
    host.files.insert("c.txt".into(), "new".into());
    let (fourth, _) = load(&reader, &mut host, Some(&third));
    assert!(matches!(fourth.global("Level.c"), Some(Value::String(s)) if &*s == "new"));
}

#[test]
fn values_reading_live_state_are_never_reused() {
    let files = [("game.fun", "let pose = T.live()\nlet derived = pose + 1.0\n")];
    let mut host = TestHost::default();
    let (first, _) = load(&files, &mut host, None);
    let (second, stats) = load(&files, &mut host, Some(&first));
    assert_eq!(host.live_calls, 2);
    assert_eq!(num(&second, "derived"), 3.0, "a dependent of a live read re-evaluates");
    assert_eq!(all(&stats).0, 0);
}

#[test]
fn opaque_host_values_are_never_reused() {
    let files = [("game.fun", "let h = T.handle()\nlet n = T.slow(1.0)\n")];
    let mut host = TestHost::default();
    let (first, _) = load(&files, &mut host, None);
    let (_, stats) = load(&files, &mut host, Some(&first));
    assert_eq!(all(&stats), (1, 2), "only `n`");
}

#[test]
fn a_changed_type_evaluates_the_value() {
    let files = [("game.fun", "let n = T.slow(1.0)\nlet m = n + 1.0\n")];
    let mut host = TestHost::default();
    let (first, _) = load(&files, &mut host, None);
    let (_, stats) = load_with(&files, &mut host, Some(&first), |types| {
        types.insert("n".into(), "string".into());
    });
    assert_eq!(host.slow_calls, 2);
    assert_eq!(all(&stats).0, 0, "`m` depends on `n`");
    // Without a known type there's no reuse either.
    let (third, _) = load(&files, &mut host, None);
    let (_, stats) = load_with(&files, &mut host, Some(&third), |types| {
        types.remove("n");
    });
    assert_eq!(all(&stats).0, 0);
}

#[test]
fn a_dependency_moved_below_its_user_evaluates_it() {
    // Eager evaluation can only demand defs above, so the reload must fail
    // the same way a cold load does rather than keep the old value.
    let mut host = TestHost::default();
    let files = [("game.fun", "let n = T.slow(1.0)\nlet m = n + 1.0\n")];
    let (first, _) = load(&files, &mut host, None);
    let project = load_sources_with_bundled_modules(
        vec![(PathBuf::from("game.fun"), "let m = n + 1.0\nlet n = T.slow(1.0)\n".into())],
        &[BundledModule::interface("T", T)],
    )
    .ok()
    .unwrap();
    let (_, types) = project.check_with_types();
    let types = def_types(&project.module, &types);
    let reloaded = Session::load_reusing(&project.module, &mut host, Some(&first), types);
    assert!(reloaded.is_err(), "used before its definition");
}

#[test]
fn a_session_from_plain_load_reuses_nothing() {
    let files = [("game.fun", "let n = T.slow(1.0)\n")];
    let project = load_sources_with_bundled_modules(
        vec![(PathBuf::from("game.fun"), files[0].1.into())],
        &[BundledModule::interface("T", T)],
    )
    .ok()
    .unwrap();
    let mut host = TestHost::default();
    let plain = Session::load(&project.module, &mut host).ok().unwrap();
    let (_, stats) = load(&files, &mut host, Some(&plain));
    assert_eq!(all(&stats).0, 0);
    assert_eq!(host.slow_calls, 2);
}

#[test]
fn a_host_that_records_nothing_reuses_only_values_that_never_called_it() {
    // `NoInputs` keeps the trait defaults: a host call makes the value
    // non-reusable, pure code still reuses.
    struct NoInputs(TestHost);
    impl Host for NoInputs {
        fn provides(&self, path: &str) -> bool {
            self.0.provides(path)
        }
        fn call(&mut self, path: &str, args: Vec<Value>, span: Span) -> Result<HostReturn, RunError> {
            self.0.call(path, args, span)
        }
    }
    let src = "let n = T.slow(1.0)\nlet table = List.map((i) => i * i, List.range(10.0))\n";
    let project = load_sources_with_bundled_modules(
        vec![(PathBuf::from("game.fun"), src.into())],
        &[BundledModule::interface("T", T)],
    )
    .ok()
    .unwrap();
    let (_, types) = project.check_with_types();
    let types = def_types(&project.module, &types);
    let mut host = NoInputs(TestHost::default());
    let (first, _) = Session::load_reusing(&project.module, &mut host, None, types.clone()).ok().unwrap();
    let (_, stats) = Session::load_reusing(&project.module, &mut host, Some(&first), types).ok().unwrap();
    assert_eq!(all(&stats), (1, 2));
    assert_eq!(host.0.slow_calls, 2);
}
