//! A command line with no subcommand opens the desktop app when one is
//! installed, and prints the usual help when none is. Built with the `app`
//! feature, the app runs in-process and its children are this `functor`.
#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

/// A copy of the CLI in a directory of its own: the build directory may hold
/// a real `functor-app` beside it, which `functor` would find first. Kept in
/// the build's own temp directory, replaced each run.
fn functor() -> Command {
    static COPY: OnceLock<PathBuf> = OnceLock::new();
    let exe = COPY.get_or_init(|| {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("default-app");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("functor");
        let built = env!("CARGO_BIN_EXE_functor");
        if std::fs::hard_link(built, &exe).is_err() {
            std::fs::copy(built, &exe).unwrap();
        }
        exe
    });
    let mut c = Command::new(exe);
    c.env("PATH", "/nonexistent");
    c
}

// In-process (`app` on macOS) bare `functor` opens the window instead.
#[cfg(not(all(feature = "app", target_os = "macos")))]
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

// In-process (`app` on macOS) bare `functor` opens the window instead.
#[cfg(not(all(feature = "app", target_os = "macos")))]
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

#[test]
fn a_command_missing_its_own_subcommand_keeps_its_help() {
    let out = functor()
        .env("FUNCTOR_APP_BIN", "/bin/echo")
        .arg("inspect")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("Usage: functor inspect"));
}

// In-process (`app` on macOS) bare `functor` opens the window instead.
#[cfg(not(all(feature = "app", target_os = "macos")))]
#[test]
fn bare_functor_runs_the_app() {
    let out = functor()
        .env("FUNCTOR_APP_BIN", "/bin/echo")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("--functor "));
}

#[test]
fn help_and_version_never_open_the_app() {
    for flag in ["--help", "--version"] {
        let out = functor()
            .env("FUNCTOR_APP_BIN", "/bin/echo")
            .arg(flag)
            .output()
            .unwrap();
        let said = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "{flag}");
        assert!(said.starts_with("Functor") || said.starts_with("functor"), "{flag}: {said}");
    }
}

#[test]
fn app_compat_answers_the_apps_check_on_stdout_alone() {
    let out = functor().args(["app-compat", "--json"]).output().unwrap();
    assert!(out.status.success());
    assert!(out.stderr.is_empty(), "{}", String::from_utf8_lossy(&out.stderr));
    let answer: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(answer["app_protocol"].as_u64() >= Some(1), "{answer}");
    assert!(answer["version"].is_string(), "{answer}");
    assert!(answer["debug_protocol"].is_u64(), "{answer}");
    // Hidden from the help.
    let help = functor().arg("--help").output().unwrap();
    assert!(!String::from_utf8_lossy(&help.stdout).contains("app-compat"));
}

/// In-process, the app's children are this executable: with nothing else
/// on PATH, a game session starts and answers, and no CLI problem is said.
#[cfg(feature = "app")]
#[test]
fn the_app_in_process_runs_its_games_with_this_functor() {
    let home = tempfile::tempdir().unwrap();
    let counter = concat!(env!("CARGO_MANIFEST_DIR"), "/../examples/counter");
    let out = functor()
        .args(["-d", counter, "app", "--headless", "--print", "--start-session"])
        .args(["--no-gpu", "--no-device-link"])
        .env("FUNCTOR_APP_HOME", home.path())
        .env("FUNCTOR_BIN", "/nonexistent/functor")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{text}{err}");
    assert!(text.contains("\nsession: running pid "), "{text}");
    assert!(!text.contains("\ncli: "), "{text}");
}
