//! A command line with no subcommand opens the desktop app when one is
//! installed, and prints the usual help when none is.
#![cfg(unix)]

use std::process::Command;

fn functor() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_functor"));
    c.env("PATH", "/nonexistent");
    c
}

#[test]
fn no_subcommand_runs_the_app_with_the_project() {
    // `echo` stands in for the app: it prints the arguments it was given.
    let out = functor()
        .env("FUNCTOR_APP_BIN", "/bin/echo")
        .args(["-d", "some/game"])
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{said}");
    assert!(said.starts_with("--functor "), "{said}");
    assert!(said.trim_end().ends_with("-d some/game"), "{said}");
}

#[test]
fn no_subcommand_without_the_app_prints_help() {
    let out = functor().env_remove("FUNCTOR_APP_BIN").output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("Usage: functor"));
}

#[test]
fn a_bad_flag_is_still_an_error_with_the_app_installed() {
    let out = functor()
        .env("FUNCTOR_APP_BIN", "/bin/echo")
        .arg("--no-such-flag")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
}
