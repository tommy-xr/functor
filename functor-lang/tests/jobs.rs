//! What a job (`Job.run`) needs from the language: a closure's top-level
//! def ([`Roots`]), that def's dependencies and code hash ([`CodeIndex`]),
//! evaluation that stops on a flag, and the checker's top-level rule.

use functor_lang::project::{load_sources_with_bundled_modules, BundledModule, Project};
use functor_lang::reuse::{CodeIndex, Roots};
use functor_lang::{NoHost, Session, Value};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

const JOB: &str = "let run : (string, ('i) => 'o, 'i, ('r) => 'msg) => float\n";

fn project(src: &str) -> Project {
    let sources = vec![(PathBuf::from("game.fun"), src.to_string())];
    load_sources_with_bundled_modules(sources, &[BundledModule::interface("Job", JOB)])
        .unwrap_or_else(|e| panic!("{}", e.render()))
}

fn sorted(deps: Option<std::collections::HashSet<String>>) -> Vec<String> {
    let mut deps: Vec<String> = deps.expect("a def").into_iter().collect();
    deps.sort();
    deps
}

#[test]
fn deps_are_transitive_and_skip_unrelated_defs() {
    let p = project(
        "let table = [1.0, 2.0]
let helper = (x) => x + List.length(table)
let f = (x) => helper(x) * 2.0
let unrelated = List.range(1000.0)
",
    );
    let index = CodeIndex::new(&p.module);
    assert_eq!(sorted(index.deps_of("f")), ["f", "helper", "table"]);
    assert!(index.deps_of("missing").is_none());
}

#[test]
fn the_code_key_changes_only_with_the_code_a_function_reaches() {
    let base = "let k = 2.0\nlet f = (x) => x * k\nlet other = 1.0\n";
    let key = |src: &str| CodeIndex::new(&project(src).module).code_key("f").unwrap();
    assert_eq!(key(base), key(&base.replace("other = 1.0", "other = 5.0")));
    assert_ne!(key(base), key(&base.replace("k = 2.0", "k = 3.0")));
    assert_ne!(key(base), key(&base.replace("x * k", "x + k")));
}

#[test]
fn only_a_top_level_lambda_names_a_def() {
    let p = project(
        "let f = (x) => x + 1.0
let alias = f
let add = (a, b) => a + b
let partial = add(1.0)
let make = (n) => (x) => x + n
let made = make(2.0)
",
    );
    let session = Session::load(&p.module, &mut NoHost).ok().unwrap();
    let roots = Roots::of(&p.module);
    let name = |def: &str| roots.name_of(&session.global(def).unwrap()).map(str::to_string);
    assert_eq!(name("f").as_deref(), Ok("f"));
    assert_eq!(name("alias").as_deref(), Ok("f"));
    assert!(name("partial").unwrap_err().contains("partial application"));
    assert!(name("made").unwrap_err().contains("inside another one"));
    assert!(roots.name_of(&Value::Number(1.0)).unwrap_err().contains("not a function"));
    // A closure from another load of the same code isn't this module's.
    let other = project("let f = (x) => x + 1.0\n");
    let other = Session::load(&other.module, &mut NoHost).ok().unwrap();
    assert!(roots.name_of(&other.global("f").unwrap()).is_err());
}

#[test]
fn a_set_stop_flag_ends_a_load_and_a_call() {
    let src = "let count = (n) => if n <= 0.0 then 0.0 else 1.0 + count(n - 1.0)
let spin = (n) => List.fold((acc, x) => acc + x, 0.0, List.range(n))
let slow = spin(100000.0)
";
    let p = project(src);
    let stop = AtomicBool::new(true);
    let err = Session::load_cancellable(&p.module, &|_| true, &mut NoHost, &stop).err().unwrap();
    assert_eq!(err.error.message, functor_lang::eval::CANCELLED);
    let keep = |name: &str| name != "slow";
    let session = Session::load_cancellable(&p.module, &keep, &mut NoHost, &stop).ok().unwrap();
    let err = session
        .call_cancellable("spin", vec![Value::Number(100000.0)], &mut NoHost, &stop)
        .err()
        .unwrap();
    assert_eq!(err.message, functor_lang::eval::CANCELLED);
    // Unset, the same call runs to the end.
    let go = AtomicBool::new(false);
    let sum = session.call_cancellable("spin", vec![Value::Number(10.0)], &mut NoHost, &go);
    assert_eq!(sum.ok().unwrap().to_string(), "45");
}

#[test]
fn the_checker_refuses_a_job_function_that_isnt_top_level() {
    let errors = |body: &str| -> Vec<String> {
        let src = format!(
            "let f = (x: float): float => x * 2.0\nlet alias = f\nlet made = ((n) => (x) => x + n)(1.0)\n\
let go = (m) => {body}\n"
        );
        project(&src).check().into_iter().map(|e| e.message).collect()
    };
    let job = |errs: Vec<String>| errs.into_iter().filter(|e| e.contains("Job.run")).count();
    assert_eq!(job(errors("Job.run(\"k\", f, 1.0, (r) => r)")), 0);
    assert_eq!(job(errors("Job.run(\"k\", alias, 1.0, (r) => r)")), 0);
    assert_eq!(job(errors("Job.run(\"k\", (x) => x + m, 1.0, (r) => r)")), 1);
    assert_eq!(job(errors("Job.run(\"k\", made, 1.0, (r) => r)")), 1);
}
