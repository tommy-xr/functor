//! `--headless --print` against this repo's own examples.

use std::process::Command;

#[test]
fn print_marks_the_start_project_active() {
    let hello = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/hello");
    let home = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_functor-app"))
        .args(["--headless", "--print", "-d", hello])
        .env("FUNCTOR_APP_HOME", home.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("\nactive: examples/hello\n"), "{text}");
    assert!(
        text.contains("\n* examples/hello (hello): entry game.fun\n"),
        "{text}"
    );
}
